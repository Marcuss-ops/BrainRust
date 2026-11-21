//! Audio source preparation: parallel probes, authoritative range checks and
//! the one-per-path BGM loudness measurement. The render orchestration owns the
//! plan; this module owns what is learned about the source files.

use std::collections::HashMap;
use std::path::Path;

use super::{
    bgm_loudness_trim_db, loudness_trim_filter, measure_mean_volume_db, probe_source, track_events,
    LoudnessCache, Plan, SourceProbe,
};
use crate::error::MuscleError;

pub(super) struct SourceCaches {
    probes: HashMap<String, SourceProbe>,
    loudness: HashMap<String, Option<f64>>,
    // Wall clock of the BGM loudness measurement in `prefetch`. The caller
    // reports it as its own stage timing: the measurement decodes the whole
    // music asset, so folding it into an unmeasured pre-mix block hid the
    // dominant cost of a short timeline.
    loudness_ms: i64,
    ffmpeg: String,
}

impl SourceCaches {
    /// Probes every unique event/layer path once and measures every unique BGM
    /// path once, both with bounded parallelism. Probe failures are swallowed
    /// here: the authoritative per-event `ensure_source` call re-probes and
    /// reports the typed error in plan order, so prefetching can only save
    /// waits, never change the outcome.
    pub(super) fn prefetch(plan: &Plan, by_id: &HashMap<String, String>, ffmpeg: &str) -> Self {
        Self::prefetch_with_cache(plan, by_id, ffmpeg, LoudnessCache::from_env())
    }

    /// `prefetch` with an explicit cache, so tests can serve a hit without
    /// touching the worker's environment or a real decoder.
    pub(super) fn prefetch_with_cache(
        plan: &Plan,
        by_id: &HashMap<String, String>,
        ffmpeg: &str,
        cache: LoudnessCache,
    ) -> Self {
        let mut probes = HashMap::new();
        let mut unique_paths: Vec<String> = Vec::new();
        let mut consider = |path: &str| {
            if Path::new(path).is_file() && !unique_paths.iter().any(|p| p == path) {
                unique_paths.push(path.to_string());
            }
        };
        for (_, event) in track_events(plan)
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
            let probed = crate::parallel::bounded_map(
                &unique_paths,
                crate::parallel::MAX_PARALLEL_IO,
                |path| probe_source(ffmpeg, path).map_err(|error| error.to_string()),
            );
            for (path, result) in unique_paths.iter().zip(probed) {
                if let Ok(probe) = result {
                    probes.insert(path.clone(), probe);
                }
            }
        }

        let mut loudness = HashMap::new();
        let mut loudness_ms = 0i64;
        let mut bgm_paths: Vec<String> = Vec::new();
        let mut consider_bgm = |path: &str| {
            if Path::new(path).is_file() && !bgm_paths.iter().any(|p| p == path) {
                bgm_paths.push(path.to_string());
            }
        };
        for (_, event) in track_events(plan)
            .into_iter()
            .filter(|(_, event)| event.r#type.eq_ignore_ascii_case("bgm"))
        {
            if let Some(path) = event.asset_id.as_ref().and_then(|id| by_id.get(id)) {
                consider_bgm(path);
            }
        }
        for layer in &plan.background_music {
            if let Some(path) = by_id.get(&layer.asset_id) {
                consider_bgm(path);
            }
        }
        // A cached measurement is a hit on the *value*, not on the stage
        // timing: reading a 150-byte entry costs microseconds and belongs to
        // prepare_ms, while loudness_ms must keep reporting the decode that a
        // cold job pays for.
        let mut pending: Vec<String> = Vec::new();
        for path in &bgm_paths {
            match cache.load(path) {
                Some(mean) => {
                    loudness.insert(path.clone(), Some(mean));
                }
                None => pending.push(path.clone()),
            }
        }
        if !pending.is_empty() {
            let measured_started = std::time::Instant::now();
            let measured =
                crate::parallel::bounded_map(&pending, crate::parallel::MAX_PARALLEL_IO, |path| {
                    Ok(measure_mean_volume_db(ffmpeg, path).ok())
                });
            loudness_ms = measured_started.elapsed().as_millis() as i64;
            for (path, mean) in pending.iter().zip(measured) {
                let mean = mean.ok().flatten();
                loudness.insert(path.clone(), mean);
                // Only a real measurement is cached: a failed attempt stays a
                // failed attempt for this render and is retried next time.
                if let Some(value) = mean {
                    cache.store(path, value);
                }
            }
        }

        Self {
            probes,
            loudness,
            loudness_ms,
            ffmpeg: ffmpeg.to_string(),
        }
    }

    /// Wall clock spent measuring BGM mean volume during `prefetch`.
    pub(super) fn loudness_ms(&self) -> i64 {
        self.loudness_ms
    }

    /// Returns the cached probe or runs the authoritative one. The 40 ms
    /// tolerance absorbs container-duration rounding while still rejecting a
    /// source that cannot cover the declared range.
    pub(super) fn ensure_source(
        &mut self,
        asset_id: &str,
        path: &str,
        required_end_us: i64,
    ) -> Result<SourceProbe, MuscleError> {
        let probe = if let Some(probe) = self.probes.get(path) {
            probe.clone()
        } else {
            let probe = probe_source(&self.ffmpeg, path)?;
            self.probes.insert(path.to_owned(), probe.clone());
            probe
        };
        if probe.duration_sec * 1_000_000.0 + 40_000.0 < required_end_us as f64 {
            return Err(MuscleError::generic(format!(
                "audio asset {asset_id} is shorter than required source range"
            )));
        }
        Ok(probe)
    }

    /// Static trim that lands a BGM source at the canonical loudness. Measured
    /// once per unique path; a failed measurement caches `None` and fails open
    /// exactly once, matching the historical behaviour.
    pub(super) fn bgm_loudness_prefix(&mut self, path: &str) -> String {
        let measured = match self.loudness.get(path) {
            Some(cached) => *cached,
            None => {
                let measured = measure_mean_volume_db(&self.ffmpeg, path).ok();
                self.loudness.insert(path.to_owned(), measured);
                measured
            }
        };
        measured
            .and_then(bgm_loudness_trim_db)
            .map(loudness_trim_filter)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "pipelinegen-sources-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn bgm_plan(asset_path: &str) -> (Plan, HashMap<String, String>) {
        let plan: Plan = serde_json::from_str(
            r#"{
                "audio_plan_version": "compiled-audio-plan.v1",
                "duration_us": 1000000,
                "background_music": [
                    { "asset_id": "bgm-1", "timeline_start_us": 0, "duration_us": 1000000 }
                ],
                "canonical_audio_profile": {
                    "codec": "aac", "profile": "LC", "sample_rate": 48000,
                    "channels": 2, "channel_layout": "stereo", "bitrate": "192k"
                }
            }"#,
        )
        .expect("plan");
        let by_id = HashMap::from([("bgm-1".to_string(), asset_path.to_string())]);
        (plan, by_id)
    }

    /// A stand-in decoder: prints the line `measure_mean_volume_db` parses.
    fn fake_ffmpeg(dir: &std::path::Path, mean_db: &str) -> String {
        let path = dir.join("fake-ffmpeg.sh");
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho 'mean_volume: {mean_db} dB' >&2\nexit 0\n"),
        )
        .expect("fake ffmpeg");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake ffmpeg");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_measured_bgm_is_stored_and_the_next_render_reads_it_back() {
        let temp = TestDir::new();
        let asset = temp.0.join("bgm.mp3");
        std::fs::write(&asset, b"asset bytes").expect("asset");
        let asset = asset.to_string_lossy().into_owned();
        let (plan, by_id) = bgm_plan(&asset);
        let cache_dir = temp.0.join("cache");
        let ffmpeg = fake_ffmpeg(&temp.0, "-18.0");

        let mut cold = SourceCaches::prefetch_with_cache(
            &plan,
            &by_id,
            &ffmpeg,
            LoudnessCache::at(cache_dir.clone()),
        );
        let measured = cold.bgm_loudness_prefix(&asset);
        assert_eq!(
            measured, "volume=-5.000000dB,",
            "-23 dB target minus -18 dB source"
        );

        // The decoder is gone: only a cache hit can still produce the trim, and
        // a hit must not be reported as loudness time.
        let warm = SourceCaches::prefetch_with_cache(
            &plan,
            &by_id,
            "/nonexistent/ffmpeg-must-not-run",
            LoudnessCache::at(cache_dir),
        );
        assert_eq!(
            warm.loudness_ms(),
            0,
            "a cache hit must not be timed as a decode"
        );
        assert_eq!(
            LoudnessCache::at(temp.0.join("cache")).load(&asset),
            Some(-18.0),
            "the cold render must have stored the measurement"
        );
    }

    #[test]
    fn an_unmeasured_path_fails_open_without_a_trim() {
        let temp = TestDir::new();
        let asset = temp.0.join("bgm.mp3");
        std::fs::write(&asset, b"asset bytes").expect("asset");
        let asset = asset.to_string_lossy().into_owned();
        let (plan, by_id) = bgm_plan(&asset);

        let mut sources = SourceCaches::prefetch_with_cache(
            &plan,
            &by_id,
            "/nonexistent/ffmpeg",
            LoudnessCache::at(temp.0.join("cache")),
        );
        assert_eq!(sources.bgm_loudness_prefix(&asset), "");
        let entries = std::fs::read_dir(temp.0.join("cache"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(
            entries, 0,
            "a failed measurement must not be cached as if it were an answer"
        );
    }
}
