//! Manifest-aware timeline transforms — the Rust side of the overlay
//! manifest handoff (OVERLAY-MANIFEST-CONTRACT.md §3, §4).
//!
//! Two operations adopt the two wire fields the final worker must learn:
//!
//! * `project_overlay_manifest` — the §3 projection: the master video gets
//!   every manifest entry's window REPLACED by the same window of the
//!   certified artifact (packed `artifact_in_us/artifact_out_us` positions
//!   when present, else the plan position). Overlapping windows are
//!   idempotent (the packed artifact composes them at the right z-order), so
//!   the projection is pure frame substitution — no compositing.
//! * `extract_timeline_window` — the `scene.clip.{timeline_item_id,in_us,
//!   out_us}` wire: phrase-only scene clips are windows of the SAME artifact,
//!   extracted by manifest entry id instead of materialized as standalone
//!   Drive clips. This is the worker-side twin of `render_clip_exec`'s
//!   `manifest` mode.
//!
//! Both re-encode through the canonical encoder policy (a replacement needs
//! real frames, never packets), keep the master audio verbatim where the
//! master has any (overlay segments are silent; the final audio enters once
//! at `mux_audio_copy`), and fail closed on any manifest inconsistency.

use crate::artifact::{failed_response, part_path};
use crate::encoder::append_video_options;
use crate::manifest_projection::{
    merge_pieces, resolve_entry, resolve_sub_window, us_to_frame_ceil, us_to_frame_floor,
    ProjectionPiece,
};
use crate::probe::{ffprobe_path, probe_file};
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use crate::transform::finish_run;
use std::fs;
use std::path::Path;

/// `project_overlay_manifest`: replace every manifest entry's window on the
/// master with that entry's artifact window. `source_path` is the master,
/// `input_paths[0]` is the certified overlay artifact.
pub(super) fn execute_projection(request: Request) -> Response {
    let (master, artifact, output) = match projection_inputs(&request) {
        Ok(values) => values,
        Err(response) => return *response,
    };
    let manifest = match request.overlay_manifest.as_ref() {
        Some(manifest) => manifest,
        None => {
            return failed_response(Some(master), "overlay_manifest is required".to_string());
        }
    };
    let profile = match request.media.profile() {
        Ok(profile) => profile,
        Err(error) => return failed_response(Some(master), error),
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let ffprobe = ffprobe_path(ffmpeg);
    let master_meta = match probe_file(&ffprobe, &master) {
        Ok(meta) => meta,
        Err(error) => return failed_response(Some(master), error),
    };
    let artifact_meta = match probe_file(&ffprobe, &artifact) {
        Ok(meta) => meta,
        Err(error) => return failed_response(Some(artifact), error),
    };
    if let Err(error) = same_frame_grid(&master_meta, &artifact_meta) {
        return failed_response(Some(master), error);
    }
    if !master_meta.has_video || !artifact_meta.has_video {
        return failed_response(
            Some(master),
            "manifest projection requires a video master and a video artifact".to_string(),
        );
    }
    let (fps_num, fps_den) = (master_meta.fps_num, master_meta.fps_den);
    let master_frames = frame_count(&master_meta, fps_num, fps_den);
    let artifact_frames = frame_count(&artifact_meta, fps_num, fps_den);

    // §3: resolve every entry to frames, then merge into disjoint runs.
    let mut pieces = Vec::with_capacity(manifest.entries.len());
    for entry in &manifest.entries {
        match resolve_entry(entry, fps_num, fps_den, artifact_frames, master_frames) {
            Ok(piece) => pieces.push(piece),
            Err(error) => return failed_response(Some(master), error),
        }
    }
    let runs = match merge_pieces(pieces) {
        Ok(runs) => runs,
        Err(error) => return failed_response(Some(master), error),
    };

    // Whole-master coverage: master passthrough outside the runs, artifact
    // frames inside. Every segment keeps the master frame count, so a copied
    // master audio stream stays in sync.
    let segments = coverage_segments(&runs, master_frames);
    let part = part_path(&output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    command.args(["-i", &master, "-i", &artifact]);
    let filter = match projection_filter(&segments, &profile) {
        Ok(filter) => filter,
        Err(error) => return failed_response(Some(master), error),
    };
    command.args(["-filter_complex", &filter, "-map", "[vout]"]);
    if master_meta.has_audio {
        command.args(["-map", "0:a?", "-c:a", "copy"]);
    } else {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, &request) {
        return failed_response(Some(master), error);
    }
    command.arg(&part);
    finish_run(
        "project_overlay_manifest",
        &master,
        &part,
        &output,
        command.output(),
        None,
    )
}

/// `extract_timeline_window`: cut one manifest entry's artifact window (or a
/// plan sub-window of it, `scene.clip.{timeline_item_id,in_us,out_us}`) as a
/// silent video segment. `source_path` is the artifact.
pub(super) fn execute_window(request: Request) -> Response {
    let artifact = match request.source_path.as_deref() {
        Some(path) if !path.is_empty() && Path::new(path).is_file() => path.to_string(),
        Some(path) => {
            return failed_response(
                Some(path.to_string()),
                format!("source file is not readable: {path}"),
            );
        }
        None => return failed_response(None, "source_path is required".to_string()),
    };
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path.to_string(),
        _ => {
            return failed_response(
                Some(artifact),
                "output_path is required".to_string(),
            );
        }
    };
    if let Some(parent) = Path::new(&output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(Some(artifact), format!("create output directory: {error}"));
        }
    }
    let manifest = match request.overlay_manifest.as_ref() {
        Some(manifest) => manifest,
        None => {
            return failed_response(Some(artifact), "overlay_manifest is required".to_string());
        }
    };
    let window = match request.timeline_window.as_ref() {
        Some(window) => window,
        None => {
            return failed_response(Some(artifact), "timeline_window is required".to_string());
        }
    };
    let entry = match manifest
        .entries
        .iter()
        .find(|entry| entry.item_id == window.timeline_item_id)
    {
        Some(entry) => entry,
        None => {
            return failed_response(
                Some(artifact),
                format!(
                    "timeline_window references unknown manifest entry {}",
                    window.timeline_item_id
                ),
            );
        }
    };
    let (in_us, out_us) = match (window.in_us, window.out_us) {
        (Some(in_us), Some(out_us)) => {
            match resolve_sub_window(entry, in_us, out_us) {
                Ok(span) => span,
                Err(error) => return failed_response(Some(artifact), error),
            }
        }
        _ => (
            entry.artifact_in_us.unwrap_or(entry.in_us),
            entry.artifact_out_us.unwrap_or(entry.out_us),
        ),
    };
    let profile = match request.media.profile() {
        Ok(profile) => profile,
        Err(error) => return failed_response(Some(artifact), error),
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let ffprobe = ffprobe_path(ffmpeg);
    let artifact_meta = match probe_file(&ffprobe, &artifact) {
        Ok(meta) => meta,
        Err(error) => return failed_response(Some(artifact), error),
    };
    if !artifact_meta.has_video {
        return failed_response(
            Some(artifact),
            "timeline window extraction requires a video artifact".to_string(),
        );
    }
    let (fps_num, fps_den) = (artifact_meta.fps_num, artifact_meta.fps_den);
    let artifact_frames = frame_count(&artifact_meta, fps_num, fps_den);
    let start = us_to_frame_floor(in_us, fps_num, fps_den);
    let end = us_to_frame_ceil(out_us, fps_num, fps_den);
    if start >= artifact_frames {
        return failed_response(
            Some(artifact),
            format!(
                "timeline window starts beyond the artifact (frame {start} of {artifact_frames})"
            ),
        );
    }
    let end = end.min(artifact_frames);
    if end <= start {
        return failed_response(
            Some(artifact),
            "timeline window has an empty artifact range".to_string(),
        );
    }

    let part = part_path(&output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y", "-i", &artifact]);
    let filter = window_filter(start, end, &profile);
    command.args(["-filter_complex", &filter, "-map", "[vout]", "-an"]);
    if let Err(error) = append_video_options(&mut command, &request) {
        return failed_response(Some(artifact), error);
    }
    command.arg(&part);
    finish_run(
        "extract_timeline_window",
        &artifact,
        &part,
        &output,
        command.output(),
        None,
    )
}

/// Shared prologue for the projection: master + exactly one artifact input,
/// output required, output directory created.
fn projection_inputs(request: &Request) -> Result<(String, String, String), Box<Response>> {
    let master = match request.source_path.as_deref() {
        Some(path) if !path.is_empty() && Path::new(path).is_file() => path.to_string(),
        Some(path) => {
            return Err(Box::new(failed_response(
                Some(path.to_string()),
                format!("source file is not readable: {path}"),
            )));
        }
        None => return Err(Box::new(failed_response(None, "source_path is required"))),
    };
    let inputs = request.input_paths.clone().unwrap_or_default();
    if inputs.len() != 1 {
        return Err(Box::new(failed_response(
            Some(master),
            "exactly one artifact input is required",
        )));
    }
    let artifact = inputs[0].clone();
    if !Path::new(&artifact).is_file() {
        return Err(Box::new(failed_response(
            Some(master),
            format!("source file is not readable: {artifact}"),
        )));
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path.to_string(),
        _ => {
            return Err(Box::new(failed_response(
                Some(master),
                "output_path is required",
            )));
        }
    };
    if let Some(parent) = Path::new(&output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(Box::new(failed_response(
                Some(master),
                format!("create output directory: {error}"),
            )));
        }
    }
    Ok((master, artifact, output))
}

fn same_frame_grid(master: &MediaMetadata, artifact: &MediaMetadata) -> Result<(), String> {
    if master.fps_num == 0
        || master.fps_den == 0
        || master.fps_num * artifact.fps_den != artifact.fps_num * master.fps_den
    {
        return Err(format!(
            "manifest projection requires the artifact and master on the same frame grid \
             (master {}/{}, artifact {}/{})",
            master.fps_num, master.fps_den, artifact.fps_num, artifact.fps_den
        ));
    }
    Ok(())
}

fn frame_count(meta: &MediaMetadata, fps_num: u32, fps_den: u32) -> i64 {
    (meta.duration_sec * fps_num as f64 / fps_den as f64).round() as i64
}

/// One output segment: frames from the master (0) or the artifact (1).
struct Segment {
    input: usize,
    start_frame: i64,
    end_frame: i64,
}

/// Walk the merged runs over `[0, master_frames)`, alternating master
/// passthrough and artifact replacement. Runs are disjoint and sorted
/// (merge_pieces), so the coverage is complete and non-overlapping.
fn coverage_segments(runs: &[ProjectionPiece], master_frames: i64) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    for run in runs {
        if run.master_start > cursor {
            segments.push(Segment {
                input: 0,
                start_frame: cursor,
                end_frame: run.master_start,
            });
        }
        segments.push(Segment {
            input: 1,
            start_frame: run.artifact_start,
            end_frame: run.artifact_end(),
        });
        cursor = run.master_end;
    }
    if cursor < master_frames {
        segments.push(Segment {
            input: 0,
            start_frame: cursor,
            end_frame: master_frames,
        });
    }
    segments
}

/// Frame-accurate splice filtergraph: every segment is trimmed on its own
/// input's frame grid and normalized to the canonical profile, then
/// concatenated. Deliberately NO `fps` re-time here: master and artifact are
/// already on the same frame grid (same_frame_grid fail-closed above), and
/// the `fps` filter duplicates a frame at each segment end to fill its output
/// window — which would shift the §3 window mapping and inflate the frame
/// count through concat. `trim`+`setpts` preserves exact frames and timing.
fn projection_filter(
    segments: &[Segment],
    profile: &crate::config::VideoProfile,
) -> Result<String, String> {
    if segments.is_empty() {
        return Err("manifest projection produced no output segments".to_string());
    }
    let mut chains = Vec::with_capacity(segments.len());
    let mut labels = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        chains.push(format!(
            "[{}:v]trim=start_frame={}:end_frame={},setpts=PTS-STARTPTS{}[v{}]",
            segment.input,
            segment.start_frame,
            segment.end_frame,
            normalize_chain(profile),
            index
        ));
        labels.push(format!("[v{index}]"));
    }
    chains.push(format!(
        "{}concat=n={}:v=1:a=0[vout]",
        labels.join(""),
        segments.len()
    ));
    Ok(chains.join(";"))
}

fn window_filter(
    start_frame: i64,
    end_frame: i64,
    profile: &crate::config::VideoProfile,
) -> String {
    format!(
        "[0:v]trim=start_frame={start_frame}:end_frame={end_frame},setpts=PTS-STARTPTS{}[vout]",
        normalize_chain(profile)
    )
}

/// Spatial/pixel normalization for concat compatibility. No fps re-time —
/// the frame grid is the master's and must stay exact (see projection_filter).
fn normalize_chain(profile: &crate::config::VideoProfile) -> String {
    format!(
        ",scale={}:{},setsar=1,format=yuv420p",
        profile.width, profile.height
    )
}
