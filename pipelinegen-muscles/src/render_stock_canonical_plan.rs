use serde::Deserialize;

use crate::error::MuscleError;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CanonicalRenderPlan {
    pub(crate) version: String,
    pub(crate) fps_numerator: i64,
    pub(crate) fps_denominator: i64,
    pub(crate) duration_frames: i64,
    pub(crate) timeline_hash: String,
    pub(crate) manifest_sha256: String,
    pub(crate) plan_sha256: String,
    pub(crate) manifest: Vec<CanonicalManifestEntry>,
    pub(crate) video_tracks: Vec<CanonicalVideoTrack>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CanonicalManifestEntry {
    pub(crate) asset_id: String,
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) frame_count: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CanonicalVideoTrack {
    pub(crate) index: i32,
    pub(crate) segments: Vec<CanonicalVideoSegment>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CanonicalVideoSegment {
    pub(crate) asset_id: String,
    pub(crate) source: CanonicalFrameRange,
    pub(crate) timeline: CanonicalFrameRange,
    pub(crate) z_index: i32,
    #[serde(default)]
    pub(crate) freeze: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CanonicalFrameRange {
    pub(crate) start_frame: i64,
    pub(crate) frame_count: i64,
}

pub(crate) fn validate_canonical_plan(plan: &CanonicalRenderPlan) -> Result<(), MuscleError> {
    if plan.version != "render-plan.v2"
        || plan.fps_numerator <= 0
        || plan.fps_denominator <= 0
        || plan.duration_frames <= 0
    {
        return Err(MuscleError::generic(
            "invalid render_plan version, frame rate, or duration_frames",
        ));
    }
    for hash in [
        &plan.timeline_hash,
        &plan.manifest_sha256,
        &plan.plan_sha256,
    ] {
        if hash.len() != 64
            || !hash
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(MuscleError::generic(
                "render_plan hashes must be lowercase SHA256 values",
            ));
        }
    }
    let mut manifest = HashMap::new();
    for entry in &plan.manifest {
        if entry.asset_id.trim().is_empty()
            || entry.path.trim().is_empty()
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            || entry.frame_count <= 0
        {
            return Err(MuscleError::generic(
                "render_plan manifest entry is incomplete",
            ));
        }
        if manifest.contains_key(&entry.asset_id) {
            return Err(MuscleError::generic(format!(
                "render_plan manifest asset is duplicated: {}",
                entry.asset_id
            )));
        }
        if !Path::new(&entry.path).is_file() {
            return Err(MuscleError::generic(format!(
                "render_plan manifest path is not readable: {}",
                entry.path
            )));
        }
        let actual = sha256_file(&entry.path)?;
        if actual != entry.sha256 {
            return Err(MuscleError::generic(format!(
                "render_plan manifest hash mismatch for {}",
                entry.asset_id
            )));
        }
        manifest.insert(entry.asset_id.clone(), entry.frame_count);
    }

    // An audio-only intro may precede the first visual segment. Once the
    // primary video track starts, visual segments remain contiguous and must
    // cover the remainder of the output, matching the Go RenderPlan contract.
    let mut expected_timeline: Option<i64> = None;
    for track in &plan.video_tracks {
        if track.index < 0 {
            return Err(MuscleError::generic("render_plan track index is invalid"));
        }
        if track.index != 0 && !track.segments.is_empty() {
            return Err(MuscleError::generic(
                "render_plan additional populated tracks are unsupported",
            ));
        }
        for segment in &track.segments {
            let Some(timeline_end) = segment
                .timeline
                .start_frame
                .checked_add(segment.timeline.frame_count)
            else {
                return Err(MuscleError::generic(format!(
                    "render_plan integer frame segment overflows: {}",
                    segment.asset_id
                )));
            };
            let Some(asset_frame_count) = manifest.get(&segment.asset_id) else {
                return Err(MuscleError::generic(format!(
                    "render_plan asset is missing: {}",
                    segment.asset_id
                )));
            };
            let Some(source_end) = segment
                .source
                .start_frame
                .checked_add(segment.source.frame_count)
            else {
                return Err(MuscleError::generic(format!(
                    "render_plan source frame range overflows: {}",
                    segment.asset_id
                )));
            };
            let expected_start = expected_timeline.unwrap_or(segment.timeline.start_frame);
            // A freeze tail stretches one source frame across many timeline
            // frames; every other segment keeps the 1:1 source↔timeline map.
            let source_matches_timeline = if segment.freeze {
                segment.source.frame_count == 1
            } else {
                segment.source.frame_count == segment.timeline.frame_count
            };
            if segment.source.start_frame < 0
                || segment.source.frame_count <= 0
                || segment.timeline.start_frame != expected_start
                || segment.timeline.frame_count <= 0
                || !source_matches_timeline
                || source_end > *asset_frame_count
                || timeline_end > plan.duration_frames
                || segment.z_index < 0
            {
                return Err(MuscleError::generic(format!(
                    "render_plan integer frame segment is invalid: {}",
                    segment.asset_id
                )));
            }
            expected_timeline = Some(
                expected_start
                    .checked_add(segment.timeline.frame_count)
                    .ok_or_else(|| {
                        MuscleError::generic("render_plan timeline frame count overflows")
                    })?,
            );
        }
    }
    if expected_timeline.unwrap_or(0) != plan.duration_frames {
        return Err(MuscleError::generic(
            "render_plan timeline does not cover duration_frames",
        ));
    }
    Ok(())
}

fn sha256_file(path: &str) -> Result<String, MuscleError> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| MuscleError::generic(format!("compute SHA256 for {path}: {error}")))?;
    if !output.status.success() {
        return Err(MuscleError::generic(format!(
            "compute SHA256 for {path} failed"
        )));
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if digest.len() != 64
        || !digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(MuscleError::generic(format!(
            "sha256sum returned an invalid digest for {path}"
        )));
    }
    Ok(digest)
}
