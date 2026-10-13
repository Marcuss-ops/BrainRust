#[path = "render_audio_execution/envelopes.rs"]
mod envelopes;
#[path = "render_audio_execution/loudness.rs"]
mod loudness;
#[path = "render_audio_execution/event_filter.rs"]
mod event_filter;
use envelopes::*;
use loudness::*;
use event_filter::*;

use super::plan::{track_events, validate_plan};

use super::plan::{Automation, Plan};
use super::probe::{probe_audio, probe_source, SourceProbe};
use crate::artifact::{failed_response, part_path, publish_output, sha256_file};
use crate::error::MuscleError;
use crate::process::FFmpegRunner;
use crate::protocol::{AudioAsset, Request, Response};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// The plan carries gains in dB, while the FFmpeg volume filter treats a plain
// number as a linear multiplier (volume=0 silences the stream, and negative
// "gains" would invert/amplify instead of attenuating). Convert at the
// boundary with the standard power ratio so 0 dB maps to unity and the
// canonical duck levels (-12/-18/-24 dB) attenuate as the mix policy intends.
fn source_normalize(probe: &SourceProbe) -> &'static str {
    if probe.sample_rate == Some(48000)
        && probe.channels == Some(2)
        && probe.channel_layout.as_deref() == Some("stereo")
    {
        ""
    } else {
        "aresample=48000,aformat=channel_layouts=stereo,"
    }
}

// event_filter builds the ffmpeg filter graph for ONE already-compiled
// audio event. This is the boundary where "Go decides, Rust executes" is
// enforced: the graph plays EXACTLY the declared source range
// [source_in_us, source_end) once, delayed to the declared timeline start.
// There is no loop filter, no stream_loop, and no duration discovery — a
// plan that wants the music repeated must contain N explicit events (the
// Go loop expander's output). If the source is shorter than the declared
// range, ensure_source fails the render instead of inventing a loop.
//
// eval_mode is the volume filter's expression-evaluation mode
// (volume_eval_mode): "eval=frame" when the gain expression depends on
// absolute output time (automation), "eval=once" for static gains.
//
// normalize is the source_normalize output for this event's asset: the
// aresample/aformat chain when the source is not canonical, empty when it
// already is 48 kHz stereo.
pub(super) fn execute(request: Request) -> Response {
    let output = match request.output_path.as_deref() {
        Some(value) if !value.is_empty() => value,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    // The plan arrives typed from the protocol boundary; a malformed plan
    // already failed request parsing fail-closed, so absence is the only
    // rejection left here.
    let plan: Plan = match request.audio_plan.clone() {
        Some(value) => value,
        None => return failed_response(None, "audio_plan is invalid".to_string()),
    };
    if let Err(error) = validate_plan(&plan) {
        return failed_response(None, error.to_string());
    }
    // The plan carries the canonical microsecond duration, but the video
    // renderer emits FrameAt(duration_us) whole frames, which can round the
    // timeline up by up to half a frame. The master is padded to the
    // frame-aligned duration (master_duration_us) so audio_duration >=
    // video_duration instead of leaving a silent tail at the end of the mux.
    let master_duration_us = plan.master_duration_us.unwrap_or(plan.duration_us);
    let master_secs = master_duration_us as f64 / 1_000_000.0;
    let by_id: HashMap<String, String> = request
        .audio_assets
        .unwrap_or_default()
        .into_iter()
        .map(|asset: AudioAsset| (asset.asset_id, asset.path))
        .collect();
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    // Parallel source probes (P1.6): every event/layer probe is an
    // independent ffprobe subprocess, so pre-probe all unique asset paths
    // at once instead of paying them sequentially inside the build loops.
    // Best-effort only: failures here are swallowed and the authoritative
    // per-event loop below re-probes (sequentially) and reports the typed
    // error in plan order, so pre-probing can never change the outcome —
    // only collapse N sequential waits into one parallel wait.
    let mut source_probes: HashMap<String, SourceProbe> = HashMap::new();
    {
        let mut unique_paths: Vec<String> = Vec::new();
        let mut consider = |path: &str| {
            if Path::new(path).is_file() && !unique_paths.iter().any(|p| p == path) {
                unique_paths.push(path.to_string());
            }
        };
        for (_, event) in track_events(&plan)
            .into_iter()
            .filter(|(_, event)| event.r#type != "SILENCE")
        {
            if let Some(path) = event.asset_id.as_ref().and_then(|id| by_id.get(id)) {
                consider(path);
            }
        }
        for layers in [&plan.background_music, &plan.sfx] {
            for layer in layers {
                if let Some(path) = by_id.get(&layer.asset_id) {
                    consider(path);
                }
            }
        }
        if !unique_paths.is_empty() {
            let probed: Vec<(String, Result<SourceProbe, MuscleError>)> =
                std::thread::scope(|scope| {
                    unique_paths
                        .iter()
                        .map(|path| {
                            let ffmpeg = ffmpeg.to_string();
                            let path_owned = path.clone();
                            scope.spawn(move || {
                                let result = probe_source(&ffmpeg, &path_owned);
                                (path_owned, result)
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                        .map(|handle| {
                            handle.join().unwrap_or_else(|_| {
                                (
                                    String::new(),
                                    Err(MuscleError::generic("audio probe thread panicked")),
                                )
                            })
                        })
                        .collect()
                });
            for (path, result) in probed {
                if let Ok(probe) = result {
                    source_probes.insert(path, probe);
                }
            }
        }
    }
    let mut ensure_source =
        |asset_id: &str, path: &str, required_end_us: i64| -> Result<SourceProbe, MuscleError> {
            let probe = if let Some(probe) = source_probes.get(path) {
                probe.clone()
            } else {
                let probe = probe_source(ffmpeg, path)?;
                source_probes.insert(path.to_owned(), probe.clone());
                probe
            };
            if probe.duration_sec * 1_000_000.0 + 40_000.0 < required_end_us as f64 {
                return Err(MuscleError::generic(format!(
                    "audio asset {asset_id} is shorter than required source range"
                )));
            }
            Ok(probe)
        };
    // bgm_loudness_prefix returns the static trim filter that lands a BGM
    // source at the canonical loudness, measured once per unique path (the
    // loop expander reuses one source across N events — one decode pass,
    // N references). Non-BGM callers never reach here, and a failed
    // measurement caches None so the render fails open exactly once.
    let mut loudness_cache: HashMap<String, Option<f64>> = HashMap::new();
    let mut bgm_loudness_prefix = |path: &str| -> String {
        let measured = match loudness_cache.get(path) {
            Some(cached) => *cached,
            None => {
                let measured = measure_mean_volume_db(ffmpeg, path).ok();
                loudness_cache.insert(path.to_owned(), measured);
                measured
            }
        };
        measured
            .and_then(bgm_loudness_trim_db)
            .map(loudness_trim_filter)
            .unwrap_or_default()
    };
    let mut inputs = Vec::new();
    let mut filters = Vec::new();
    for (track_id, event) in track_events(&plan)
        .into_iter()
        .filter(|(_, event)| event.r#type != "SILENCE")
    {
        let path = match event.asset_id.as_ref().and_then(|id| by_id.get(id)) {
            Some(path) if Path::new(path).is_file() => path,
            Some(path) => {
                return failed_response(
                    Some(path.clone()),
                    format!("audio source is not readable: {path}"),
                )
            }
            None => return failed_response(None, "audio event asset is unresolved".to_string()),
        };
        let index = inputs.len();
        inputs.push(path.clone());
        let source_end = match event.source_in_us.checked_add(event.source_duration_us) {
            Some(value) if event.source_duration_us > 0 => value,
            _ => {
                return failed_response(None, "audio event source range is incomplete".to_string())
            }
        };
        let probe = match ensure_source(event.asset_id.as_deref().unwrap_or(""), path, source_end) {
            Ok(probe) => probe,
            Err(error) => return failed_response(Some(path.clone()), error.to_string()),
        };
        let delay_ms = (event.timeline_start_us + 500) / 1000;
        let mut targeting: Vec<(&Automation, bool)> = Vec::new();
        for automation in &plan.automation {
            let target = if !automation.target_track_id.trim().is_empty() {
                &automation.target_track_id
            } else {
                &automation.target_layer
            };
            if target.eq_ignore_ascii_case(&track_id) {
                // A triggered entry ducks under its trigger track; an
                // untriggered one is a fade shape over the layer window.
                targeting.push((automation, !automation.trigger_track_id.trim().is_empty()));
            }
        }
        let gain = track_gain_expr(event.gain_db, &targeting);
        // eval=frame is mandatory for automation: the volume filter's
        // default eval=once decides the gain at the first frame (t=0),
        // so every event would keep ONE level for its whole duration — a
        // duck window [0,20s) would duck a looped BGM's second event
        // (delay 20s) instead of the first, and multi-scene clip ducking
        // windows would never match. With eval=frame the expression is
        // evaluated per frame and, because adelay shifts the stream PTS
        // to absolute output time, the between(t, ...) windows land at
        // the exact absolute positions the plan declares. Events with no
        // targeting entries have a static gain and take eval=once instead
        // (behaviourally identical, no per-frame expression cost).
        filters.push(event_filter(
            index,
            event.source_in_us,
            source_end,
            delay_ms,
            &gain,
            volume_eval_mode(!targeting.is_empty()),
            // BGM enters at the canonical loudness (see
            // BGM_SOURCE_LOUDNESS_TARGET_DB); every other role joins
            // unmodified, exactly as before.
            &format!(
                "{}{}",
                source_normalize(&probe),
                if event.r#type.eq_ignore_ascii_case("bgm") {
                    bgm_loudness_prefix(path)
                } else {
                    String::new()
                }
            ),
        ));
    }
    for (layer_name, layers) in [("BGM", &plan.background_music), ("SFX", &plan.sfx)] {
        for layer in layers {
            let path = match by_id.get(&layer.asset_id) {
                Some(path) if Path::new(path).is_file() => path,
                Some(path) => {
                    return failed_response(
                        Some(path.clone()),
                        format!("audio source is not readable: {path}"),
                    )
                }
                None => return failed_response(None, format!("{layer_name} asset is unresolved")),
            };
            if layer.duration_us <= 0 || layer.timeline_start_us < 0 {
                return failed_response(None, format!("{layer_name} layer range is invalid"));
            }
            let index = inputs.len();
            inputs.push(path.clone());
            let probe = match ensure_source(&layer.asset_id, path, layer.duration_us) {
                Ok(probe) => probe,
                Err(error) => return failed_response(Some(path.clone()), error.to_string()),
            };
            let mut targeting: Vec<(&Automation, bool)> = Vec::new();
            for automation in &plan.automation {
                if automation.target_track_id.eq_ignore_ascii_case(layer_name)
                    || automation.target_layer.eq_ignore_ascii_case(layer_name)
                {
                    if automation.start_us < 0 || automation.end_us <= automation.start_us {
                        return failed_response(
                            None,
                            "audio automation range is invalid".to_string(),
                        );
                    }
                    targeting.push((automation, !automation.trigger_track_id.trim().is_empty()));
                }
            }
            let gain = track_gain_expr(layer.gain_db, &targeting);
            let delay_ms = (layer.timeline_start_us + 500) / 1000;
            let duration = layer.duration_us as f64 / 1_000_000.0;
            // Same eval contract as the track path above: automation
            // windows are absolute output time and must be re-evaluated per
            // frame (eval=frame); a static layer gain takes eval=once.
            let eval_mode = volume_eval_mode(!targeting.is_empty());
            // Legacy BGM layers get the same canonical-loudness trim as
            // track-role BGM events; SFX keep their canonical absolute level.
            let normalize = format!(
                "{}{}",
                source_normalize(&probe),
                if layer_name == "BGM" {
                    bgm_loudness_prefix(path)
                } else {
                    String::new()
                }
            );
            filters.push(format!("[{index}:a]atrim=duration={duration},asetpts=PTS-STARTPTS,{normalize}adelay={delay_ms}|{delay_ms},volume='{gain}':{eval_mode}[a{index}]"));
        }
    }
    // Silence anchor + apad + atrim. The full-length silent bed is always
    // part of the mix, so the graph emits samples for the entire canonical
    // duration even when every real source ends early (SILENCE events carry
    // no asset and are intentionally dropped above). apad then fills any
    // remaining shortfall and atrim removes any overflow, so the master is
    // pinned to exactly plan.duration_us instead of relying on the output
    // `-t` limit alone.
    let pad_secs = master_secs;
    filters.push(format!(
        "anullsrc=r=48000:cl=stereo,atrim=duration={pad_secs},asetpts=PTS-STARTPTS[asilence]"
    ));
    let mix_labels: String = (0..inputs.len())
        .map(|index| format!("[a{index}]"))
        .chain(std::iter::once("[asilence]".to_string()))
        .collect();
    let filter = if inputs.is_empty() {
        format!("{};[asilence]apad=whole_dur={pad_secs},atrim=duration={pad_secs},alimiter=limit=0.95[aout]", filters.join(";"))
    } else {
        format!("{};{}amix=inputs={}:duration=longest:normalize=0[mixed];[mixed]apad=whole_dur={pad_secs},atrim=duration={pad_secs},alimiter=limit=0.95[aout]", filters.join(";"), mix_labels, inputs.len() + 1)
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    let part = part_path(output);
    // Keep the filter graph out of argv. With many clips/scenes the graph can
    // exceed the host ARG_MAX limit even though the actual audio plan is
    // valid. ffmpeg's script input has the same semantics without that limit.
    let filter_script = format!("{part}.filter_complex.txt");
    if let Err(error) = fs::write(&filter_script, &filter) {
        return failed_response(None, format!("write audio filter script: {error}"));
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");

    // Single pass: render the complete filter graph directly to the
    // canonical AAC-LC master. The previous PCM intermediate caused a full
    // second read/write/encode pass over the entire timeline. The filter
    // graph still performs the exact same mix, limiter, padding and trim;
    // only the lossless intermediate is removed.
    let mut encode_command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    encode_command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in &inputs {
        encode_command.args(["-i", input]);
    }
    encode_command.args([
        "-filter_complex_script",
        &filter_script,
        "-map",
        "[aout]",
        "-c:a",
        "aac",
        "-profile:a",
        "aac_low",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-b:a",
        &plan.canonical_audio_profile.bitrate,
        "-t",
        &master_secs.to_string(),
        &part,
    ]);
    let encode_started = std::time::Instant::now();
    let encode_result = encode_command.output();
    let aac_encode_ms = encode_started.elapsed().as_millis() as i64;
    let _ = fs::remove_file(&filter_script);
    match encode_result {
        Ok(result) if result.status.success() => {}
        Ok(result) => {
            let _ = fs::remove_file(&part);
            return failed_response(
                None,
                format!(
                    "render_audio_plan encode failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            );
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            return failed_response(
                None,
                format!("render_audio_plan encode failed to start: {error}"),
            );
        }
    }

    // Probe the encoded master, then attach the stage timings to the
    // certified metadata before publishing.
    let probe_started = std::time::Instant::now();
    let probe_result = probe_audio(ffmpeg, &part, master_secs);
    let probe_ms = probe_started.elapsed().as_millis() as i64;
    match probe_result {
        Ok(mut metadata) => {
            // The mix is now fused into the single encode pass. Keep the
            // legacy metric explicit rather than reporting a fabricated
            // second-pass duration.
            metadata.mix_ms = Some(0);
            metadata.aac_encode_ms = Some(aac_encode_ms);
            metadata.probe_ms = Some(probe_ms);
            // Single-pass contract, reported from the execution plane where
            // the encode command lives: the filter graph renders the master
            // directly to AAC-LC with ONE ffmpeg encode (no PCM intermediate,
            // no decode/re-encode cycle). The Go adapter reads this instead
            // of hard-coding the count. If a future change adds a second
            // pass, this constant is the place that must be updated.
            metadata.audio_encode_passes = Some(1);
            match publish_output(&part, output) {
                Ok(()) => {
                    // Hash the published final audio in its owner (Rust), so
                    // the Go adapter maps hash_ms + the digest instead of
                    // re-measuring the hash round-trip as a single block.
                    let hash_started = std::time::Instant::now();
                    match sha256_file(output) {
                        Ok(digest) => {
                            let hash_ms = hash_started.elapsed().as_millis() as i64;
                            metadata.hash_ms = Some(hash_ms.max(1));
                            metadata.final_audio_sha256 = Some(digest);
                            Response {
                                ok: true,
                                operation: "render_audio_plan".into(),
                                source_path: None,
                                items: Vec::new(),
                                metadata: Some(metadata),
                                error: None,
                            }
                        }
                        Err(error) => failed_response(None, error),
                    }
                }
                Err(error) => failed_response(None, error),
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bgm_loudness_trim_db, duck_envelope, event_filter, fade_envelope, linear_gain,
        loudness_trim_filter, parse_volumedetect_mean, source_normalize, track_gain_expr,
        volume_eval_mode, Automation, SourceProbe, BGM_SOURCE_LOUDNESS_TARGET_DB,
    };

    #[test]
    fn fade_envelope_ramps_from_and_to_silence() {
        // Gate A's contract: 800ms fade-in, 1000ms fade-out over [0,25s).
        let expr = fade_envelope(0, 25_000_000, 800_000, 1_000_000);
        assert!(
            expr.starts_with("if(between(t,0.000000,25.000000),"),
            "envelope must be unity outside its window: {expr}"
        );
        assert!(
            expr.contains("min(min((t-0.000000)/0.800000,(25.000000-t)/1.000000),1)"),
            "envelope must ramp from silence and back: {expr}"
        );
    }

    #[test]
    fn fade_envelope_without_ramps_is_unity() {
        assert_eq!(fade_envelope(0, 10_000_000, 0, 0), "1");
    }

    #[test]
    fn duck_envelope_holds_ratio_with_edge_ramps() {
        // Duck to -28dB against a -18dB base over [0,15s): 120ms attack,
        // 350ms release.
        let ratio = 10f64.powf((-28.0 - -18.0) / 20.0);
        let expr = duck_envelope(0, 15_000_000, 120_000, 350_000, ratio);
        assert!(
            expr.contains("1+(0.316228-1)*min((t-0.000000)/0.120000,1)"),
            "attack must ramp base→duck: {expr}"
        );
        assert!(
            expr.contains("if(lt(t,14.650000),0.316228,"),
            "the ducked level must hold between the ramps: {expr}"
        );
        assert!(
            expr.contains("0.316228+(1-0.316228)*min((15.000000-t)/0.350000,1)"),
            "release must ramp duck→base: {expr}"
        );
    }

    #[test]
    fn duck_envelope_without_ramps_is_the_plain_step() {
        let expr = duck_envelope(5_000_000, 9_000_000, 0, 0, 0.25);
        assert_eq!(
            expr, "if(between(t,5.000000,9.000000),0.250000,1)",
            "zero attack/release must keep the historical step semantics"
        );
    }

    #[test]
    fn track_gain_expr_multiplies_the_static_gain_by_every_targeting_envelope() {
        let fade = Automation {
            target_track_id: "bgm".into(),
            target_layer: String::new(),
            trigger_track_id: String::new(),
            start_us: 0,
            end_us: 25_000_000,
            gain_db: -18.0,
            attack_us: 800_000,
            release_us: 1_000_000,
        };
        let duck = Automation {
            target_track_id: "bgm".into(),
            target_layer: String::new(),
            trigger_track_id: "voiceover".into(),
            start_us: 0,
            end_us: 15_000_000,
            gain_db: -28.0,
            attack_us: 120_000,
            release_us: 350_000,
        };
        let expr = track_gain_expr(-18.0, &[(&fade, false), (&duck, true)]);
        assert!(expr.starts_with("0.125893*"), "static gain first: {expr}");
        assert!(
            expr.contains("if(between(t,0.000000,25.000000)"),
            "fade shape: {expr}"
        );
        assert!(
            expr.contains("0.316228"),
            "duck ratio is relative to the base gain: {expr}"
        );
    }

    #[test]
    fn event_filter_plays_only_the_declared_source_range_once() {
        // The fundamental expansion's 2nd BGM event ([20s, 40s), source
        // restarting from 0) must be a single atrim of exactly [0,20s)
        // delayed to 20s — the whole 20s source played once, never looped.
        let filter = event_filter(1, 0, 20_000_000, 20_000, "0.063096", "eval=once", "");
        assert!(filter.contains("atrim=start=0:end=20,"), "{filter}");
        assert!(filter.contains("adelay=20000|20000"), "{filter}");
        assert!(filter.contains("volume='0.063096':eval=once"), "{filter}");
        assert!(filter.ends_with("[a1]"), "{filter}");
    }

    #[test]
    fn event_filter_keeps_eval_frame_for_automated_gains() {
        // An event whose gain expression depends on absolute output time
        // (duck/fade automation targeting its track) must keep eval=frame:
        // only the per-frame evaluation makes between(t, ...) windows land
        // at the declared absolute positions after the adelay.
        let filter = event_filter(
            1,
            0,
            20_000_000,
            20_000,
            "0.063096*if(between(t,0.000000,15.000000),0.316228,1)",
            "eval=frame",
            "",
        );
        assert!(
            filter.contains(
                "volume='0.063096*if(between(t,0.000000,15.000000),0.316228,1)':eval=frame"
            ),
            "{filter}"
        );
        assert!(filter.contains("adelay=20000|20000"), "{filter}");
    }

    #[test]
    fn event_filter_truncates_the_last_loop_event_not_the_source() {
        // The fundamental expansion's 4th event covers [60s, 75s): a 15s
        // trim from the same 20s source, delayed to 60s. Rust renders the
        // declared 15s — it never plays the whole 20s source again nor
        // loops it to fill the timeline.
        let filter = event_filter(3, 0, 15_000_000, 60_000, "0.063096", "eval=once", "");
        assert!(filter.contains("atrim=start=0:end=15,"), "{filter}");
        assert!(filter.contains("adelay=60000|60000"), "{filter}");
    }

    #[test]
    fn event_filter_applies_declared_source_trim_for_sfx() {
        // An SFX with source_in_ms=250, duration_ms=900 is trimmed to the
        // declared [0.25s, 1.15s) slice of its source — the source is never
        // played beyond the compiled event.
        let filter = event_filter(0, 250_000, 1_150_000, 12_000, "0.398107", "eval=once", "");
        assert!(filter.contains("atrim=start=0.25:end=1.15,"), "{filter}");
        assert!(filter.contains("adelay=12000|12000"), "{filter}");
    }

    #[test]
    fn event_filter_never_invents_a_loop() {
        // The renderer must never add a loop/aloop/stream_loop filter: a
        // compiled plan carries N explicit events for N repeats, and Rust
        // executes them verbatim.
        for filter in [
            event_filter(0, 0, 20_000_000, 0, "0.063096", "eval=once", ""),
            event_filter(3, 0, 15_000_000, 60_000, "0.063096", "eval=frame", ""),
        ] {
            assert!(!filter.contains("loop"), "filter must never loop: {filter}");
        }
    }

    #[test]
    fn source_normalize_skips_conversion_for_canonical_assets() {
        let canonical = SourceProbe {
            duration_sec: 10.0,
            sample_rate: Some(48000),
            channels: Some(2),
            channel_layout: Some("stereo".into()),
        };
        assert_eq!(source_normalize(&canonical), "");
    }

    #[test]
    fn source_normalize_keeps_conversion_for_non_canonical_assets() {
        // Mono 24 kHz voiceover: must be resampled AND upmixed to stereo.
        let voiceover = SourceProbe {
            duration_sec: 10.0,
            sample_rate: Some(24000),
            channels: Some(1),
            channel_layout: Some("mono".into()),
        };
        assert_eq!(
            source_normalize(&voiceover),
            "aresample=48000,aformat=channel_layouts=stereo,"
        );
        // Stereo 44.1 kHz BGM: resample needed.
        let bgm = SourceProbe {
            duration_sec: 10.0,
            sample_rate: Some(44100),
            channels: Some(2),
            channel_layout: Some("stereo".into()),
        };
        assert_eq!(
            source_normalize(&bgm),
            "aresample=48000,aformat=channel_layouts=stereo,"
        );
        // 48 kHz 2ch with unknown layout: pin the stereo layout explicitly.
        let ambiguous = SourceProbe {
            duration_sec: 10.0,
            sample_rate: Some(48000),
            channels: Some(2),
            channel_layout: None,
        };
        assert_eq!(
            source_normalize(&ambiguous),
            "aresample=48000,aformat=channel_layouts=stereo,"
        );
    }

    #[test]
    fn event_filter_skips_normalize_for_canonical_sources() {
        let filter = event_filter(1, 0, 20_000_000, 20_000, "0.063096", "eval=once", "");
        assert!(!filter.contains("aresample"), "{filter}");
        assert!(!filter.contains("aformat"), "{filter}");
        assert!(
            filter.contains("asetpts=PTS-STARTPTS,adelay=20000|20000"),
            "{filter}"
        );
    }

    #[test]
    fn event_filter_normalizes_non_canonical_sources() {
        let filter = event_filter(
            1,
            0,
            20_000_000,
            20_000,
            "0.063096",
            "eval=once",
            "aresample=48000,aformat=channel_layouts=stereo,",
        );
        assert!(
            filter.contains("asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay=20000|20000"),
            "{filter}"
        );
    }

    #[test]
    fn volume_eval_mode_is_frame_only_with_automation() {
        // Static gains (no targeting entries) evaluate once; any automation
        // envelope references t and must re-evaluate per frame.
        assert_eq!(volume_eval_mode(false), "eval=once");
        assert_eq!(volume_eval_mode(true), "eval=frame");
    }

    #[test]
    fn linear_gain_maps_zero_db_to_unity() {
        assert_eq!(linear_gain(0.0), "1.000000");
    }

    #[test]
    fn linear_gain_maps_policy_duck_levels() {
        assert_eq!(linear_gain(-12.0), "0.251189");
        assert_eq!(linear_gain(-18.0), "0.125893");
        assert_eq!(linear_gain(-24.0), "0.063096");
    }

    #[test]
    fn linear_gain_is_never_negative() {
        for db in [-30.0, -18.0, -6.0, 0.0, 3.0, 12.0] {
            let value: f64 = linear_gain(db).parse().unwrap();
            assert!(value.is_finite() && value > 0.0);
        }
    }

    #[test]
    fn bgm_loudness_trim_lands_any_source_at_the_canonical_target() {
        // The bed's whole point: a full-scale tone (-3 dB mean), a curated
        // quiet bed (-37 dB mean) and a hot source all end up entering the
        // mix at the SAME loudness, so BackgroundMusicGainDB is
        // source-independent.
        for mean_db in [-3.0, -37.0, -18.0, 0.0] {
            let trim = bgm_loudness_trim_db(mean_db).expect("finite mean must trim");
            assert!(
                ((mean_db + trim) - BGM_SOURCE_LOUDNESS_TARGET_DB).abs() < 1e-9,
                "mean {mean_db} dB + trim {trim} must reach {BGM_SOURCE_LOUDNESS_TARGET_DB} dB"
            );
        }
    }

    #[test]
    fn bgm_loudness_trim_is_clamped_to_the_headroom_budget() {
        // A near-silent source must not be boosted more than ±24 dB: the
        // clamp keeps its noise floor from rising into audibility, and
        // keeps a loud source from eating the limiter's headroom.
        let silence = bgm_loudness_trim_db(-91.0).expect("finite mean must trim");
        assert_eq!(silence, 24.0);
        let loud = bgm_loudness_trim_db(6.0).expect("finite mean must trim");
        assert_eq!(loud, -24.0);
    }

    #[test]
    fn bgm_loudness_trim_fails_open_on_non_finite_measurement() {
        // volumedetect reports digital silence as -inf: no trim, the
        // render proceeds exactly as it did before normalization existed.
        assert_eq!(bgm_loudness_trim_db(f64::NEG_INFINITY), None);
        assert_eq!(bgm_loudness_trim_db(f64::NAN), None);
    }

    #[test]
    fn parse_volumedetect_mean_reads_the_mean_line() {
        let stderr = b"[Parsed_volumedetect_0 @ 0x0] n_samples: 480000\n\n[Parsed_volumedetect_0 @ 0x0] mean_volume: -23.4 dB\n[Parsed_volumedetect_0 @ 0x0] max_volume: -3.0 dB\n";
        assert_eq!(parse_volumedetect_mean(stderr), Some(-23.4));
    }

    #[test]
    fn parse_volumedetect_mean_fails_open_on_silence_or_missing_stat() {
        assert_eq!(
            parse_volumedetect_mean(b"[Parsed_volumedetect_0] mean_volume: -inf dB\n"),
            None
        );
        assert_eq!(parse_volumedetect_mean(b"no volumedetect here\n"), None);
    }

    #[test]
    fn loudness_trim_filter_is_a_volume_fragment_for_the_normalize_prefix() {
        assert_eq!(loudness_trim_filter(-19.99), "volume=-19.990000dB,");
        // The fragment must compose into the event filter chain before
        // adelay without disturbing the canonical volume expression.
        let filter = event_filter(
            0,
            0,
            10_000_000,
            0,
            "0.5",
            "eval=once",
            &loudness_trim_filter(-20.0),
        );
        assert!(
            filter.contains("volume=-20.000000dB,adelay=0|0"),
            "{filter}"
        );
        assert!(filter.contains("volume='0.5':eval=once"), "{filter}");
    }
}
