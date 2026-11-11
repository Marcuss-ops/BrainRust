//! Audio source preparation: parallel probes, authoritative range checks and
//! the one-per-path BGM loudness measurement. The render orchestration owns the
//! plan; this module owns what is learned about the source files.

use std::collections::HashMap;
use std::path::Path;

use super::{
    bgm_loudness_trim_db, loudness_trim_filter, measure_mean_volume_db, probe_source, track_events,
    Plan, SourceProbe,
};
use crate::error::MuscleError;

pub(super) struct SourceCaches {
    probes: HashMap<String, SourceProbe>,
    loudness: HashMap<String, Option<f64>>,
    ffmpeg: String,
}

impl SourceCaches {
    /// Probes every unique event/layer path once and measures every unique BGM
    /// path once, both with bounded parallelism. Probe failures are swallowed
    /// here: the authoritative per-event `ensure_source` call re-probes and
    /// reports the typed error in plan order, so prefetching can only save
    /// waits, never change the outcome.
    pub(super) fn prefetch(plan: &Plan, by_id: &HashMap<String, String>, ffmpeg: &str) -> Self {
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
        if !bgm_paths.is_empty() {
            let measured = crate::parallel::bounded_map(
                &bgm_paths,
                crate::parallel::MAX_PARALLEL_IO,
                |path| Ok(measure_mean_volume_db(ffmpeg, path).ok()),
            );
            for (path, mean) in bgm_paths.iter().zip(measured) {
                loudness.insert(path.clone(), mean.ok().flatten());
            }
        }

        Self {
            probes,
            loudness,
            ffmpeg: ffmpeg.to_string(),
        }
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
