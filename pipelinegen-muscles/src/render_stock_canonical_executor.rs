use super::plan::CanonicalRenderPlan;
use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_args;
use crate::process::{FFmpegRunner, ProcessCommand};
use crate::protocol::{MediaMetadata, Request, Response};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Frames decoded *before* the segment's first source frame so an input seek
/// can land on an earlier keyframe without losing frame exactness: the
/// residual frames are trimmed after the seek, exactly as they would be in a
/// full decode, but only a couple of frames are ever decoded twice.
const SEEK_PREROLL_FRAMES: i64 = 2;

/// One encoded timeline part: either a source segment (possibly freeze-held)
/// or a black filler for an audio-only intro.
struct PartJob {
    index: usize,
    command: ProcessCommand,
}

pub(super) fn execute_canonical_render(
    plan: &CanonicalRenderPlan,
    request: &Request,
    output: &str,
    ffmpeg: &str,
    profile: &crate::config::VideoProfile,
) -> Response {
    let part = part_path(output);
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

    let encoder = match request.media.encoder() {
        Ok(encoder) => encoder,
        Err(error) => return failed_response(None, error.to_string()),
    };
    let runner = FFmpegRunner::from_ffmpeg_path(ffmpeg);
    let temp_dir = PathBuf::from(format!("{part}.segments"));
    let _ = fs::remove_dir_all(&temp_dir);
    if let Err(error) = fs::create_dir_all(&temp_dir) {
        return failed_response(
            None,
            format!("canonical render could not create the segment directory: {error}"),
        );
    }

    let mut jobs: Vec<PartJob> = Vec::new();
    // An audio-only intro may precede the first visual segment; validation
    // guarantees the remaining timeline is contiguous.
    let leading_gap = segments[0].timeline.start_frame;
    if leading_gap > 0 {
        let path = temp_dir.join(format!("part_{:05}.mp4", jobs.len()));
        let mut command = runner.ffmpeg();
        command.args(["-hide_banner", "-loglevel", "error", "-y"]);
        command.args([
            "-f",
            "lavfi",
            "-i",
            &format!(
                "color=c=black:s={}x{}:r={}/{}",
                profile.width, profile.height, plan.fps_numerator, plan.fps_denominator
            ),
        ]);
        command.args(["-frames:v", &leading_gap.to_string(), "-an"]);
        if let Err(error) = append_video_args(&mut command, &encoder, profile, None) {
            let _ = fs::remove_dir_all(&temp_dir);
            return failed_response(None, error.to_string());
        }
        command.args(["-video_track_timescale", "90000", &path.to_string_lossy()]);
        jobs.push(PartJob {
            index: jobs.len(),
            command,
        });
    }

    let mut cursor_frame = leading_gap;
    for segment in &segments {
        let Some((path, frame_count)) = manifest.get(&segment.asset_id) else {
            let _ = fs::remove_dir_all(&temp_dir);
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
            let _ = fs::remove_dir_all(&temp_dir);
            return failed_response(None, "render_plan source frame range overflows".to_string());
        };
        if source_end > *frame_count {
            let _ = fs::remove_dir_all(&temp_dir);
            return failed_response(
                None,
                format!(
                    "render_plan source frame range exceeds asset: {}",
                    segment.asset_id
                ),
            );
        }
        if segment.timeline.start_frame > cursor_frame {
            let gap_frames = segment.timeline.start_frame - cursor_frame;
            let gap_path = temp_dir.join(format!("part_{:05}.mp4", jobs.len()));
            let mut command = runner.ffmpeg();
            command.args(["-hide_banner", "-loglevel", "error", "-y"]);
            command.args([
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "color=c=black:s={}x{}:r={}/{}",
                    profile.width, profile.height, plan.fps_numerator, plan.fps_denominator
                ),
            ]);
            command.args(["-frames:v", &gap_frames.to_string(), "-an"]);
            if let Err(error) = append_video_args(&mut command, &encoder, profile, None) {
                let _ = fs::remove_dir_all(&temp_dir);
                return failed_response(None, error.to_string());
            }
            command.args([
                "-video_track_timescale",
                "90000",
                &gap_path.to_string_lossy(),
            ]);
            jobs.push(PartJob {
                index: jobs.len(),
                command,
            });
        }

        // Input seek: decode from just before the segment instead of from
        // frame 0 of the asset. `-ss` before `-i` with accurate seeking leaves
        // the first decoded frame at the requested boundary, so the residual
        // trim below stays frame exact.
        let seek_frames = segment
            .source
            .start_frame
            .saturating_sub(SEEK_PREROLL_FRAMES)
            .max(0);
        let trim_start = segment.source.start_frame - seek_frames;
        let trim_end = trim_start + segment.source.frame_count;

        let mut filter = format!(
            "trim=start_frame={trim_start}:end_frame={trim_end},setpts=PTS-STARTPTS,scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}/{},setsar=1",
            profile.width,
            profile.height,
            profile.width,
            profile.height,
            plan.fps_numerator,
            plan.fps_denominator
        );
        if segment.freeze {
            // Hold the clip's final frame across the tail's timeline frames.
            // tpad clones the last frame for the remaining duration so only
            // the tail is synthesized; the real clip stays a trimmed copy.
            let hold_frames = segment.timeline.frame_count.saturating_sub(1);
            let hold_seconds =
                (hold_frames as f64 * plan.fps_denominator as f64) / plan.fps_numerator as f64;
            filter.push_str(&format!(
                ",tpad=stop_mode=clone:stop_duration={hold_seconds:.9},setpts=PTS-STARTPTS"
            ));
        }

        let part_path = temp_dir.join(format!("part_{:05}.mp4", jobs.len()));
        let mut command = runner.ffmpeg();
        command.args(["-hide_banner", "-loglevel", "error", "-y"]);
        if seek_frames > 0 {
            command.args(["-ss", &seek_seconds(seek_frames, plan)]);
        }
        command.args(["-i", path]);
        command.args(["-vf", &filter]);
        command.args([
            "-frames:v",
            &segment.timeline.frame_count.to_string(),
            "-an",
        ]);
        if let Err(error) = append_video_args(&mut command, &encoder, profile, None) {
            let _ = fs::remove_dir_all(&temp_dir);
            return failed_response(None, error.to_string());
        }
        command.args([
            "-video_track_timescale",
            "90000",
            &part_path.to_string_lossy(),
        ]);
        jobs.push(PartJob {
            index: jobs.len(),
            command,
        });
        cursor_frame = segment.timeline.start_frame + segment.timeline.frame_count;
    }

    let encode_started = std::time::Instant::now();
    if let Err(error) = run_parts(jobs) {
        let _ = fs::remove_dir_all(&temp_dir);
        return failed_response(None, error);
    }
    let parts = fs::read_dir(&temp_dir)
        .map(|entries| {
            let mut paths = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "mp4"))
                .collect::<Vec<_>>();
            paths.sort();
            paths
        })
        .unwrap_or_default();
    if let Err(error) = concat_parts(&runner, &parts, &part, &temp_dir) {
        let _ = fs::remove_dir_all(&temp_dir);
        return failed_response(None, error);
    }
    let _ = fs::remove_dir_all(&temp_dir);
    let ffmpeg_ms = encode_started.elapsed().as_millis() as i64;
    match publish_output(&part, output) {
        Ok(()) => {
            let duration_sec = plan.duration_frames as f64 * plan.fps_denominator as f64
                / plan.fps_numerator as f64;
            Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: Some(MediaMetadata {
                    duration_sec,
                    bitrate: None,
                    width: profile.width,
                    height: profile.height,
                    fps: plan.fps_numerator as f64 / plan.fps_denominator as f64,
                    video_codec: None,
                    pixel_format: None,
                    format_name: None,
                    stream_count: 0,
                    video_stream_count: 0,
                    audio_stream_count: 0,
                    fps_num: 0,
                    fps_den: 0,
                    audio_codec: None,
                    audio_profile: None,
                    sample_rate: None,
                    channels: None,
                    start_pts: None,
                    has_video: true,
                    has_audio: false,
                    mix_ms: None,
                    aac_encode_ms: None,
                    probe_ms: None,
                    hash_ms: None,
                    ffmpeg_ms: Some(ffmpeg_ms.max(1)),
                    startup_ms: None,
                    publish_ms: None,
                    op_ms: None,
                    final_audio_sha256: None,
                    audio_copy_eligible: None,
                    audio_encode_passes: None,
                    subtitle_raster_cpu: None,
                    decode_ms: None,
                    filter_graph_ms: None,
                    subtitle_raster_ms: None,
                    watermark_raster_ms: None,
                    frame_conversion_ms: None,
                    encode_ms: Some(ffmpeg_ms.max(1)),
                    audio_mux_ms: None,
                    ..Default::default()
                }),
                error: None,
            }
        }
        Err(error) => failed_response(None, error),
    }
}

/// Timestamp of `frame` for an input seek, with enough digits that a normal
/// frame boundary (including 30000/1001) is not rounded onto the next frame.
fn seek_seconds(frame: i64, plan: &CanonicalRenderPlan) -> String {
    format!(
        "{:.9}",
        frame as f64 * plan.fps_denominator as f64 / plan.fps_numerator as f64
    )
}

/// Runs every part in a bounded thread pool. Parts are independent (each has
/// its own source, filter graph and output), so wall time scales with cores
/// while the byte content of every part stays identical to a serial run.
fn run_parts(jobs: Vec<PartJob>) -> Result<(), String> {
    let workers = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .min(jobs.len())
        .max(1);
    let results: Mutex<Vec<(usize, std::io::Result<crate::process::ProcessOutput>)>> =
        Mutex::new(Vec::with_capacity(jobs.len()));
    let jobs = Mutex::new(std::collections::VecDeque::from(jobs));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let job = jobs.lock().expect("segment queue lock").pop_front();
                let Some(job) = job else { return };
                let result = job.command.output();
                results
                    .lock()
                    .expect("segment result lock")
                    .push((job.index, result));
            });
        }
    });

    let mut results = results.into_inner().expect("segment result lock");
    results.sort_by_key(|(index, _)| *index);
    let mut failures = Vec::new();
    for (index, result) in results {
        match result {
            Ok(output) if output.status.success() => {}
            Ok(output) => failures.push(format!(
                "segment {index}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => failures.push(format!("segment {index} failed to start: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("canonical render failed: {}", failures.join("; ")))
    }
}

/// Concatenates the encoded parts in timeline order without re-encoding. Every
/// part was produced by the same encoder contract (codec, profile, level,
/// pixel format, frame rate, timescale), which is what makes the stream copy
/// legal and byte-deterministic.
fn concat_parts(
    runner: &FFmpegRunner,
    parts: &[PathBuf],
    output: &str,
    temp_dir: &Path,
) -> Result<(), String> {
    if parts.is_empty() {
        return Err("canonical render produced no segments".to_string());
    }
    let list_path = temp_dir.join("concat.txt");
    let mut list = String::new();
    for part in parts {
        list.push_str(&format!("file '{}'\n", part.to_string_lossy()));
    }
    fs::write(&list_path, list)
        .map_err(|error| format!("canonical render could not write the concat list: {error}"))?;
    let mut command = runner.ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    command.args([
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &list_path.to_string_lossy(),
    ]);
    command.args([
        "-c",
        "copy",
        "-an",
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
        "-movflags",
        "+faststart",
        output,
    ]);
    let result = command.output();
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "canonical render concat failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Err(format!("canonical render concat failed to start: {error}")),
    }
}
