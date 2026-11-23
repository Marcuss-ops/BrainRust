use super::*;

impl<'a> RenderContext<'a> {
    /// Read the request, validate it and run one audio render.
    pub(super) fn render(&self) -> Result<Response, RenderError> {
        let prepare_started = std::time::Instant::now();
        let mut scratch = Scratch::default();
        let mut sources = SourceCaches::prefetch(self.plan, self.by_id, self.ffmpeg);
        let loudness_ms = sources.loudness_ms();
        let chains = self.chains(&mut sources)?;
        let part = self.prepare_output()?;
        let (jobs, stem_inputs) = self.stem_jobs(chains, &part, &mut scratch);
        // Everything before this instant is source preparation. The loudness
        // measurement is reported separately, so prepare_ms never counts it
        // twice and the two together close the pre-mix block.
        let prepare = PrepareTiming {
            prepare_ms: (prepare_started.elapsed().as_millis() as i64 - loudness_ms).max(0),
            loudness_ms,
        };
        // mix_ms measures the producer wall, which overlaps the encode by
        // construction; total_ms (measured by the caller) remains the wall.
        let mix_started = std::time::Instant::now();
        for job in &jobs {
            make_fifo(&job.sink).map_err(|error| RenderError::new(None, error))?;
        }
        // Keep every filter graph out of argv: with many clips/scenes the
        // graph can exceed the host ARG_MAX even though the plan is valid.
        let filter = final_mix_script(stem_inputs.len(), self.master_secs);
        let filter_script = format!("{part}.final.filter.txt");
        fs::write(&filter_script, &filter).map_err(|error| {
            RenderError::new(None, format!("write audio filter script: {error}"))
        })?;
        scratch.track(filter_script.clone());

        let run = EncodeRun::start(
            self,
            &jobs,
            &stem_inputs,
            &filter_script,
            &part,
            mix_started,
        )?;
        let timing = run.finish(self.master_secs);
        if let Err(error) = timing.encode {
            let _ = fs::remove_file(&part);
            return Err(RenderError::new(None, error));
        }
        if let Some(error) = timing.writer {
            let _ = fs::remove_file(&part);
            return Err(RenderError::new(None, error));
        }
        self.publish(&part, timing, prepare)
    }

    /// Resolve every compiled event and legacy layer into one chain spec.
    fn chains(&self, sources: &mut SourceCaches) -> Result<Vec<ChainSpec>, RenderError> {
        let mut chains = self.track_chains(sources)?;
        chains.extend(self.layer_chains(sources)?);
        Ok(chains)
    }

    /// Build the resolved chains from the compiled timeline (the "Go decides"
    /// boundary). Everything is validated here in plan order so the typed
    /// errors keep their meaning; the fan-out only renders what this accepts.
    fn track_chains(&self, sources: &mut SourceCaches) -> Result<Vec<ChainSpec>, RenderError> {
        let mut chains = Vec::new();
        for (track_id, event) in track_events(self.plan)
            .into_iter()
            .filter(|(_, event)| event.r#type != "SILENCE")
        {
            let path =
                self.resolve_asset(event.asset_id.as_deref(), "audio event asset is unresolved")?;
            let source_end = match event.source_in_us.checked_add(event.source_duration_us) {
                Some(value) if event.source_duration_us > 0 => value,
                _ => {
                    return Err(RenderError::new(
                        None,
                        "audio event source range is incomplete".to_string(),
                    ))
                }
            };
            let probe = sources
                .ensure_source(event.asset_id.as_deref().unwrap_or(""), path, source_end)
                .map_err(|error| RenderError::new(Some(path.clone()), error.to_string()))?;
            let targeting = self.track_targeting(&track_id);
            chains.push(ChainSpec {
                path: path.clone(),
                source_in_us: event.source_in_us,
                source_end_us: source_end,
                delay_ms: (event.timeline_start_us + 500) / 1000,
                gain: track_gain_expr(event.gain_db, &targeting),
                // eval=frame is mandatory for automation: the volume filter's
                // default eval=once decides the gain at the first frame (t=0),
                // so every event would keep ONE level for its whole duration.
                // A static gain takes eval=once (behaviourally identical, no
                // per-frame expression cost).
                eval_mode: volume_eval_mode(!targeting.is_empty()),
                // BGM enters at the canonical loudness; every other role joins
                // unmodified, exactly as before.
                normalize: format!(
                    "{}{}",
                    source_normalize(&probe),
                    if event.r#type.eq_ignore_ascii_case("bgm") {
                        sources.bgm_loudness_prefix(path)
                    } else {
                        String::new()
                    }
                ),
            });
        }
        Ok(chains)
    }

    /// The legacy `background_music`/`sfx` layers, kept on the same chain
    /// contract as the compiled events.
    fn layer_chains(&self, sources: &mut SourceCaches) -> Result<Vec<ChainSpec>, RenderError> {
        let mut chains = Vec::new();
        for (layer_name, layers) in [
            ("BGM", &self.plan.background_music),
            ("SFX", &self.plan.sfx),
        ] {
            for layer in layers {
                let path = match self.by_id.get(&layer.asset_id) {
                    Some(path) if Path::new(path).is_file() => path,
                    Some(path) => {
                        return Err(RenderError::new(
                            Some(path.clone()),
                            format!("audio source is not readable: {path}"),
                        ))
                    }
                    None => {
                        return Err(RenderError::new(
                            None,
                            format!("{layer_name} asset is unresolved"),
                        ))
                    }
                };
                if layer.duration_us <= 0 || layer.timeline_start_us < 0 {
                    return Err(RenderError::new(
                        None,
                        format!("{layer_name} layer range is invalid"),
                    ));
                }
                let probe = sources
                    .ensure_source(&layer.asset_id, path, layer.duration_us)
                    .map_err(|error| RenderError::new(Some(path.clone()), error.to_string()))?;
                let targeting = self.layer_targeting(layer_name)?;
                chains.push(ChainSpec {
                    path: path.clone(),
                    source_in_us: 0,
                    source_end_us: layer.duration_us,
                    delay_ms: (layer.timeline_start_us + 500) / 1000,
                    gain: track_gain_expr(layer.gain_db, &targeting),
                    eval_mode: volume_eval_mode(!targeting.is_empty()),
                    normalize: format!(
                        "{}{}",
                        source_normalize(&probe),
                        if layer_name == "BGM" {
                            sources.bgm_loudness_prefix(path)
                        } else {
                            String::new()
                        }
                    ),
                });
            }
        }
        Ok(chains)
    }

    /// One asset id resolved to a readable path, with the two distinct errors
    /// the caller wants: unreadable (carries the path) vs unresolved (none).
    fn resolve_asset(
        &self,
        asset_id: Option<&str>,
        unresolved: &str,
    ) -> Result<&'a String, RenderError> {
        match asset_id.and_then(|id| self.by_id.get(id)) {
            Some(path) if Path::new(path).is_file() => Ok(path),
            Some(path) => Err(RenderError::new(
                Some(path.clone()),
                format!("audio source is not readable: {path}"),
            )),
            None => Err(RenderError::new(None, unresolved.to_string())),
        }
    }

    /// Automations that target one compiled track. A triggered entry ducks
    /// under its trigger track; an untriggered one is a fade over the window.
    fn track_targeting(&'a self, track_id: &str) -> Vec<(&'a Automation, bool)> {
        self.plan
            .automation
            .iter()
            .filter_map(|automation| {
                let target = if !automation.target_track_id.trim().is_empty() {
                    &automation.target_track_id
                } else {
                    &automation.target_layer
                };
                target
                    .eq_ignore_ascii_case(track_id)
                    .then_some((automation, !automation.trigger_track_id.trim().is_empty()))
            })
            .collect()
    }

    /// Automations that target one legacy layer. Unlike the track path this
    /// validates the window, because a layer group carries no compiled range.
    fn layer_targeting(
        &'a self,
        layer_name: &str,
    ) -> Result<Vec<(&'a Automation, bool)>, RenderError> {
        let mut targeting = Vec::new();
        for automation in &self.plan.automation {
            if automation.target_track_id.eq_ignore_ascii_case(layer_name)
                || automation.target_layer.eq_ignore_ascii_case(layer_name)
            {
                if automation.start_us < 0 || automation.end_us <= automation.start_us {
                    return Err(RenderError::new(
                        None,
                        "audio automation range is invalid".to_string(),
                    ));
                }
                targeting.push((automation, !automation.trigger_track_id.trim().is_empty()));
            }
        }
        Ok(targeting)
    }

    /// Create the output directory and claim the unique part path.
    fn prepare_output(&self) -> Result<String, RenderError> {
        if let Some(parent) = Path::new(self.output).parent() {
            fs::create_dir_all(parent).map_err(|error| {
                RenderError::new(None, format!("create output directory: {error}"))
            })?;
        }
        Ok(part_path(self.output))
    }

    /// Split the chains into round-robin stems and describe one render job per
    /// stem. Every temporary path is tracked for cleanup.
    fn stem_jobs(
        &self,
        chains: Vec<ChainSpec>,
        part: &str,
        scratch: &mut Scratch,
    ) -> (Vec<StemJob>, Vec<String>) {
        let count = stem_count(chains.len());
        let stems = partition_stems(chains, count);
        let mut stem_inputs: Vec<String> = Vec::with_capacity(stems.len());
        let jobs: Vec<StemJob> = stems
            .iter()
            .enumerate()
            .map(|(index, chains)| StemJob {
                script: format!("{part}.stem{index}.filter.txt"),
                sink: format!("{part}.stem{index}.f32.fifo"),
                filter: stem_filter_script(chains, self.master_secs),
                paths: chains.iter().map(|chain| chain.path.clone()).collect(),
            })
            .collect();
        for job in &jobs {
            scratch.track(job.script.clone());
            scratch.track(job.sink.clone());
            stem_inputs.push(job.sink.clone());
        }
        (jobs, stem_inputs)
    }

    /// Probe the encoded master, attach the stage timings, publish atomically
    /// and hash what was actually published. Success is reported only after
    /// all four steps; nothing here can claim `ok` for a missing file.
    fn publish(
        &self,
        part: &str,
        timing: EncodeTiming,
        prepare: PrepareTiming,
    ) -> Result<Response, RenderError> {
        let probe_started = std::time::Instant::now();
        let mut metadata = probe_audio(self.ffmpeg, part, self.master_secs).map_err(|error| {
            let _ = fs::remove_file(part);
            RenderError::new(None, error)
        })?;
        let probe_ms = probe_started.elapsed().as_millis() as i64;
        metadata.prepare_ms = Some(prepare.prepare_ms);
        metadata.loudness_ms = Some(prepare.loudness_ms);
        metadata.mix_ms = Some(timing.mix_ms);
        metadata.aac_encode_ms = Some(timing.aac_encode_ms);
        metadata.probe_ms = Some(probe_ms);
        // Single-encode contract, reported from the execution plane where the
        // encode command lives: the master is AAC-LC-encoded exactly ONCE (the
        // stems are lossless PCM mix intermediates, never an encode). If a
        // future change encodes the master more than once, this constant is
        // the place that must change.
        metadata.audio_encode_passes = Some(1);
        publish_output(part, self.output).map_err(|error| RenderError::new(None, error))?;
        // Hash the published final audio in its owner (Rust), so the Go
        // adapter maps hash_ms + the digest instead of re-measuring the hash
        // round-trip as a single block.
        let hash_started = std::time::Instant::now();
        let digest = sha256_file(self.output).map_err(|error| RenderError::new(None, error))?;
        metadata.hash_ms = Some((hash_started.elapsed().as_millis() as i64).max(1));
        metadata.final_audio_sha256 = Some(digest);
        Ok(Response::success(
            Operation::RenderAudioPlan,
            None,
            Vec::new(),
            Some(metadata),
        ))
    }
}
