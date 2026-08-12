use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Deserialize)]
struct CanonicalRenderPlan {
    version: String,
    fps: u32,
    fps_numerator: i64,
    fps_denominator: i64,
    duration_frames: i64,
    timeline_hash: String,
    manifest_sha256: String,
    plan_sha256: String,
    manifest: Vec<CanonicalManifestEntry>,
    video_tracks: Vec<CanonicalVideoTrack>,
}

#[derive(Debug, Deserialize)]
struct CanonicalManifestEntry {
    asset_id: String,
    path: String,
    sha256: String,
    frame_count: i64,
}

#[derive(Debug, Deserialize)]
struct CanonicalVideoTrack {
    index: i32,
    segments: Vec<CanonicalVideoSegment>,
}

#[derive(Debug, Deserialize)]
struct CanonicalVideoSegment {
    asset_id: String,
    source: CanonicalFrameRange,
    timeline: CanonicalFrameRange,
    z_index: i32,
}

#[derive(Debug, Deserialize)]
struct CanonicalFrameRange {
    start_frame: i64,
    frame_count: i64,
}

pub(super) fn render_stock_canonical(request: Request) -> Response {
    let raw_plan = match request.render_plan.clone() {
        Some(value) => value,
        None => return failed_response(None, "render_plan is required".to_string()),
    };
    let plan: CanonicalRenderPlan = match serde_json::from_value(raw_plan) {
        Ok(plan) => plan,
        Err(error) => return failed_response(None, format!("invalid render_plan: {error}")),
    };
    if let Err(error) = validate_canonical_plan(&plan) {
        return failed_response(None, error);
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    let profile = match request.media.profile() {
        Ok(profile) => profile,
        Err(error) => return failed_response(None, error),
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let part = part_path(output);
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }

    let manifest = plan
        .manifest
        .iter()
        .map(|entry| {
            (
                entry.asset_id.clone(),
                (entry.path.clone(), entry.frame_count),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut segments = plan
        .video_tracks
        .iter()
        .flat_map(|track| track.segments.iter())
        .collect::<Vec<_>>();
    // The current canonical compiler emits one primary track. Do not silently
    // flatten additional tracks (which would lose their z-order semantics);
    // validation rejects them below. Sorting by destination frame and z-index
    // makes the executor's ordering explicit for future same-frame layers.
    segments.sort_by_key(|segment| (segment.timeline.start_frame, segment.z_index));
    if segments.is_empty() {
        return failed_response(None, "render_plan has no video segments".to_string());
    }
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for segment in &segments {
        let Some((path, frame_count)) = manifest.get(&segment.asset_id) else {
            return failed_response(
                None,
                format!("render_plan asset missing: {}", segment.asset_id),
            );
        };
        let Some(source_end) = segment
            .source
            .start_frame
            .checked_add(segment.source.frame_count)
        else {
            return failed_response(None, "render_plan source frame range overflows".to_string());
        };
        if source_end > *frame_count {
            return failed_response(
                None,
                format!(
                    "render_plan source frame range exceeds asset: {}",
                    segment.asset_id
                ),
            );
        }
        command.args(["-i", path]);
    }
    let mut filter = String::new();
    let mut concat_labels = Vec::new();
    let mut cursor_frame = 0i64;
    let mut filler_index = 0usize;
    for (index, segment) in segments.iter().enumerate() {
        if segment.timeline.start_frame > cursor_frame {
            let gap_frames = segment.timeline.start_frame - cursor_frame;
            let gap_seconds =
                (gap_frames as f64 * plan.fps_denominator as f64) / plan.fps_numerator as f64;
            filter.push_str(&format!(
                "color=c=black:s={}x{}:r={}/{}:d={:.9},trim=end_frame={},setpts=PTS-STARTPTS[vpad{}];",
                profile.width, profile.height, plan.fps_numerator, plan.fps_denominator,
                gap_seconds, gap_frames, filler_index
            ));
            concat_labels.push(format!("[vpad{}]", filler_index));
            filler_index += 1;
        }
        let end_frame = match segment
            .source
            .start_frame
            .checked_add(segment.source.frame_count)
        {
            Some(value) => value,
            None => {
                return failed_response(
                    None,
                    "render_plan source frame range overflows".to_string(),
                )
            }
        };
        filter.push_str(&format!(
            "[{index}:v]trim=start_frame={}:end_frame={},setpts=PTS-STARTPTS,scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1[v{index}];",
            segment.source.start_frame,
            end_frame,
            profile.width,
            profile.height,
            profile.width,
            profile.height,
            format!("{}/{}", plan.fps_numerator, plan.fps_denominator)
        ));
        concat_labels.push(format!("[v{}]", index));
        cursor_frame = segment.timeline.start_frame + segment.timeline.frame_count;
    }
    if cursor_frame < plan.duration_frames {
        let gap_frames = plan.duration_frames - cursor_frame;
        let gap_seconds =
            (gap_frames as f64 * plan.fps_denominator as f64) / plan.fps_numerator as f64;
        filter.push_str(&format!(
            "color=c=black:s={}x{}:r={}/{}:d={:.9},trim=end_frame={},setpts=PTS-STARTPTS[vpad{}];",
            profile.width,
            profile.height,
            plan.fps_numerator,
            plan.fps_denominator,
            gap_seconds,
            gap_frames,
            filler_index
        ));
        concat_labels.push(format!("[vpad{}]", filler_index));
    }
    filter.push_str(&concat_labels.join(""));
    filter.push_str(&format!("concat=n={}:v=1:a=0[vfinal]", concat_labels.len()));
    command.args(["-filter_complex", &filter, "-map", "[vfinal]"]);
    if let Err(error) = append_video_options(&mut command, &request) {
        return failed_response(None, error);
    }
    command.args(["-an", "-movflags", "+faststart", &part]);
    match command.output() {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "canonical render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("canonical render failed to start: {error}"))
        }
    }
}

fn validate_canonical_plan(plan: &CanonicalRenderPlan) -> Result<(), String> {
    if plan.version != "render-plan.v2"
        || plan.fps == 0
        || plan.fps_numerator <= 0
        || plan.fps_denominator <= 0
        || plan.duration_frames <= 0
    {
        return Err("invalid render_plan version, frame rate, or duration_frames".to_string());
    }
    let nominal_numerator = match plan.fps_numerator.checked_add(plan.fps_denominator / 2) {
        Some(value) => value,
        None => return Err("render_plan nominal fps calculation overflows".to_string()),
    };
    let nominal_fps = nominal_numerator / plan.fps_denominator;
    if nominal_fps != i64::from(plan.fps) {
        return Err("render_plan nominal fps disagrees with rational frame rate".to_string());
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
            return Err("render_plan hashes must be lowercase SHA256 values".to_string());
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
            return Err("render_plan manifest entry is incomplete".to_string());
        }
        if manifest.contains_key(&entry.asset_id) {
            return Err(format!(
                "render_plan manifest asset is duplicated: {}",
                entry.asset_id
            ));
        }
        if !Path::new(&entry.path).is_file() {
            return Err(format!(
                "render_plan manifest path is not readable: {}",
                entry.path
            ));
        }
        let actual = sha256_file(&entry.path)?;
        if actual != entry.sha256 {
            return Err(format!(
                "render_plan manifest hash mismatch for {}",
                entry.asset_id
            ));
        }
        manifest.insert(entry.asset_id.clone(), entry.frame_count);
    }
    // An audio-only intro may precede the first visual segment. Once the
    // primary video track starts, visual segments remain contiguous and must
    // cover the remainder of the output, matching the Go RenderPlan contract.
    let mut expected_timeline: Option<i64> = None;
    for track in &plan.video_tracks {
        if track.index < 0 {
            return Err("render_plan track index is invalid".to_string());
        }
        if track.index != 0 && !track.segments.is_empty() {
            return Err("render_plan additional populated tracks are unsupported".to_string());
        }
        for segment in &track.segments {
            let Some(timeline_end) = segment
                .timeline
                .start_frame
                .checked_add(segment.timeline.frame_count)
            else {
                return Err(format!(
                    "render_plan integer frame segment overflows: {}",
                    segment.asset_id
                ));
            };
            let Some(asset_frame_count) = manifest.get(&segment.asset_id) else {
                return Err(format!(
                    "render_plan asset is missing: {}",
                    segment.asset_id
                ));
            };
            let Some(source_end) = segment
                .source
                .start_frame
                .checked_add(segment.source.frame_count)
            else {
                return Err(format!(
                    "render_plan source frame range overflows: {}",
                    segment.asset_id
                ));
            };
            let expected_start = expected_timeline.unwrap_or(segment.timeline.start_frame);
            if segment.source.start_frame < 0
                || segment.source.frame_count <= 0
                || segment.timeline.start_frame != expected_start
                || segment.timeline.frame_count <= 0
                || segment.source.frame_count != segment.timeline.frame_count
                || source_end > *asset_frame_count
                || timeline_end > plan.duration_frames
                || segment.z_index < 0
            {
                return Err(format!(
                    "render_plan integer frame segment is invalid: {}",
                    segment.asset_id
                ));
            }
            expected_timeline = Some(
                expected_start
                    .checked_add(segment.timeline.frame_count)
                    .ok_or_else(|| "render_plan timeline frame count overflows".to_string())?,
            );
        }
    }
    if expected_timeline.unwrap_or(0) != plan.duration_frames {
        return Err("render_plan timeline does not cover duration_frames".to_string());
    }
    Ok(())
}

fn sha256_file(path: &str) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| format!("compute SHA256 for {path}: {error}"))?;
    if !output.status.success() {
        return Err(format!("compute SHA256 for {path} failed"));
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
        return Err(format!("sha256sum returned an invalid digest for {path}"));
    }
    Ok(digest)
}

