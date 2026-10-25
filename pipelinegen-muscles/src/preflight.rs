//! Fail-fast final-job preflight (P1.5, Rust mirror).
//!
//! Mirrors `enforceFinalJobPreflight` in
//! `internal/capabilities/scripts/final_job.go`: predictable remote
//! rejection classes fail locally in <1ms before the Master PREPARE
//! exists. Same rules, same fail-closed order:
//! 1. certified audio duration must be positive,
//! 2. every scene duration must be finite and > 0,
//! 3. every scene must carry a resolvable media locator.
//!
//! The Go side stays authoritative for enqueue decisions. This module keeps
//! the Rust mirror of the same rules unit-tested; the retired
//! `scene_intelligence` worker was its only wire endpoint.

use crate::error::MuscleError;
use serde::{Deserialize, Serialize};

/// One scene's preflight inputs (minimal projection of the remote scene).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PreflightScene {
    #[serde(default)]
    pub scene_id: String,
    /// Duration in seconds; must be finite and > 0.
    pub duration_seconds: Option<f64>,
    /// Resolvable media locator (Drive URL or file ID). Empty = reject.
    #[serde(default)]
    pub media_locator: String,
}

impl PreflightScene {
    pub fn new(scene_id: &str, duration_seconds: Option<f64>, media_locator: &str) -> Self {
        Self {
            scene_id: scene_id.to_string(),
            duration_seconds,
            media_locator: media_locator.to_string(),
        }
    }
}

/// Validate the preflight gate. `Ok(())` admits, `Err(msg)` fails closed
/// with the first violated rule (same order as the Go gate).
pub fn validate(certified_audio_ms: i64, scenes: &[PreflightScene]) -> Result<(), MuscleError> {
    if certified_audio_ms <= 0 {
        return Err(MuscleError::generic(
            "final_job preflight: certified audio duration must be positive",
        ));
    }
    for (i, scene) in scenes.iter().enumerate() {
        let seconds = scene.duration_seconds.unwrap_or(0.0);
        if seconds.is_nan() || seconds.is_infinite() || seconds <= 0.0 {
            return Err(MuscleError::generic(format!(
                "final_job preflight: scene {i} has an invalid duration {seconds:?}"
            )));
        }
        if scene.media_locator.trim().is_empty() {
            let id = if scene.scene_id.trim().is_empty() {
                format!("index {i}")
            } else {
                format!("{:?}", scene.scene_id)
            };
            return Err(MuscleError::generic(format!(
                "final_job preflight: scene {id} carries no resolvable media locator (predicted remote prefetch rejection)"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_scenes() -> Vec<PreflightScene> {
        vec![
            PreflightScene::new("s-01", Some(4.5), "drive:abc123"),
            PreflightScene::new("s-02", Some(6.0), "https://example.com/clip.mp4"),
        ]
    }

    #[test]
    fn valid_payload_admitted() {
        assert!(validate(5000, &ok_scenes()).is_ok());
    }

    #[test]
    fn certified_audio_must_be_positive() {
        for bad in [0, -1, -5000] {
            let err = validate(bad, &ok_scenes()).unwrap_err();
            assert!(err.contains("certified audio"), "unexpected: {err}");
        }
    }

    #[test]
    fn invalid_durations_fail_closed() {
        for bad in [
            None,
            Some(0.0),
            Some(-2.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ] {
            let mut scenes = ok_scenes();
            scenes[1].duration_seconds = bad;
            let err = validate(5000, &scenes).unwrap_err();
            assert!(err.contains("invalid duration"), "unexpected: {err}");
        }
    }

    #[test]
    fn missing_media_locator_fails_closed() {
        for bad in ["", "   "] {
            let mut scenes = ok_scenes();
            scenes[0].media_locator = bad.to_string();
            let err = validate(5000, &scenes).unwrap_err();
            assert!(
                err.contains("no resolvable media locator"),
                "unexpected: {err}"
            );
        }
    }

    #[test]
    fn audio_checked_before_scenes() {
        // Same order as the Go gate: audio first.
        let mut scenes = ok_scenes();
        scenes[0].duration_seconds = Some(f64::NAN);
        let err = validate(0, &scenes).unwrap_err();
        assert!(err.contains("certified audio"), "unexpected: {err}");
    }

    #[test]
    fn empty_scene_list_with_valid_audio_admitted() {
        assert!(validate(1000, &[]).is_ok());
    }
}
