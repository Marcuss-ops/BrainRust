use crate::artifact::{part_path, publish_output};
use crate::config::{EncoderPolicy, VideoProfile};
use crate::encoder::append_video_args;
use crate::process::{FFmpegRunner, ProcessCommand};
use crate::protocol::{MediaMetadata, Operation, Request, Response};
use crate::render_stock_canonical_plan::{CanonicalRenderPlan, CanonicalVideoSegment};
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

/// Segment scratch directory that removes itself on every exit path. The
/// executor used to repeat `remove_dir_all` before each early return; a new
/// `return` could silently leave gigabytes behind.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn create(path: PathBuf) -> std::io::Result<Self> {
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn execute_canonical_render(
    plan: &CanonicalRenderPlan,
    request: &Request,
    output: &str,
    ffmpeg: &str,
    profile: &VideoProfile,
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
        return Response::failure(
            Operation::RenderStock,
            None,
            "render_plan has no video segments".to_string(),
        );
    }

    let encoder = match request.media.encoder() {
        Ok(encoder) => encoder,
        Err(error) => return Response::failure(Operation::RenderStock, None, error.to_string()),
    };
    let runner = FFmpegRunner::from_ffmpeg_path(ffmpeg);
    let scratch = match ScratchDir::create(PathBuf::from(format!("{part}.segments"))) {
        Ok(scratch) => scratch,
        Err(error) => {
            return Response::failure(
                Operation::RenderStock,
                None,
                format!("canonical render could not create the segment directory: {error}"),
            )
        }
    };
    let temp_dir = scratch.path();
    let jobs = match build_part_jobs(
        &runner, &encoder, profile, plan, temp_dir, &manifest, &segments,
    ) {
        Ok(jobs) => jobs,
        Err(error) => return Response::failure(Operation::RenderStock, None, error),
    };

    let encode_started = std::time::Instant::now();
    if let Err(error) = run_parts(jobs) {
        return Response::failure(Operation::RenderStock, None, error);
    }
    let parts = collect_parts(temp_dir);
    if let Err(error) = concat_parts(
        &runner,
        &parts,
        &part,
        temp_dir,
        plan.fps_numerator,
        plan.fps_denominator,
    ) {
        return Response::failure(Operation::RenderStock, None, error);
    }
    let ffmpeg_ms = encode_started.elapsed().as_millis() as i64;
    certify_and_publish(&part, output, plan, profile, &encoder, ffmpeg, ffmpeg_ms)
}

/// Builds one encoded part per black gap and per timeline segment.
fn build_part_jobs(
    runner: &FFmpegRunner,
    encoder: &EncoderPolicy,
    profile: &VideoProfile,
    plan: &CanonicalRenderPlan,
    temp_dir: &Path,
    manifest: &HashMap<String, (String, i64)>,
    segments: &[&CanonicalVideoSegment],
) -> Result<Vec<PartJob>, String> {
    let mut jobs: Vec<PartJob> = Vec::new();
    // An audio-only intro may precede the first visual segment; validation
    // guarantees the remaining timeline is contiguous.
    let leading_gap = segments[0].timeline.start_frame;
    if leading_gap > 0 {
        let index = jobs.len();
        jobs.push(black_part(
            runner,
            encoder,
            profile,
            plan,
            leading_gap,
            temp_dir,
            index,
        )?);
    }

    let mut cursor_frame = leading_gap;
    for segment in segments {
        let Some((path, frame_count)) = manifest.get(&segment.asset_id) else {
            return Err(format!("render_plan asset missing: {}", segment.asset_id));
        };
        let Some(source_end) = segment
            .source
            .start_frame
            .checked_add(segment.source.frame_count)
        else {
            return Err("render_plan source frame range overflows".to_string());
        };
        if source_end > *frame_count {
            return Err(format!(
                "render_plan source frame range exceeds asset: {}",
                segment.asset_id
            ));
        }
        if segment.timeline.start_frame > cursor_frame {
            let gap_frames = segment.timeline.start_frame - cursor_frame;
            let index = jobs.len();
            jobs.push(black_part(
                runner, encoder, profile, plan, gap_frames, temp_dir, index,
            )?);
        }
        let index = jobs.len();
        let part_path = temp_dir.join(format!("part_{index:05}.mp4"));
        jobs.push(segment_part(
            runner, encoder, profile, plan, segment, path, &part_path,
        )?);
        cursor_frame = segment.timeline.start_frame + segment.timeline.frame_count;
    }
    Ok(jobs)
}

/// A black filler part for an audio-only intro or an uncovered timeline gap.
fn black_part(
    runner: &FFmpegRunner,
    encoder: &EncoderPolicy,
    profile: &VideoProfile,
    plan: &CanonicalRenderPlan,
    frames: i64,
    temp_dir: &Path,
    index: usize,
) -> Result<PartJob, String> {
    let path = temp_dir.join(format!("part_{index:05}.mp4"));
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
    command.args(["-frames:v", &frames.to_string(), "-an"]);
    append_video_args(&mut command, encoder, profile, None).map_err(|error| error.to_string())?;
    command.args(["-video_track_timescale", "90000", &path.to_string_lossy()]);
    Ok(PartJob { index, command })
}

/// One trimmed (and optionally freeze-extended) source segment.
fn segment_part(
    runner: &FFmpegRunner,
    encoder: &EncoderPolicy,
    profile: &VideoProfile,
    plan: &CanonicalRenderPlan,
    segment: &CanonicalVideoSegment,
    path: &str,
    part_path: &Path,
) -> Result<PartJob, String> {
    // Input seek: decode from just before the segment instead of from frame 0
    // of the asset. `-ss` before `-i` with accurate seeking leaves the first
    // decoded frame at the requested boundary, so the residual trim below
    // stays frame exact.
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
        // tpad clones the last frame for the remaining duration so only the
        // tail is synthesized; the real clip stays a trimmed copy.
        let hold_frames = segment.timeline.frame_count.saturating_sub(1);
        let hold_seconds =
            (hold_frames as f64 * plan.fps_denominator as f64) / plan.fps_numerator as f64;
        filter.push_str(&format!(
            ",tpad=stop_mode=clone:stop_duration={hold_seconds:.9},setpts=PTS-STARTPTS"
        ));
    }

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
    append_video_args(&mut command, encoder, profile, None).map_err(|error| error.to_string())?;
    command.args([
        "-video_track_timescale",
        "90000",
        &part_path.to_string_lossy(),
    ]);
    Ok(PartJob { index: 0, command })
}

/// Reads the finished part files in deterministic order.
fn collect_parts(temp_dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(temp_dir)
        .map(|entries| {
            let mut paths = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "mp4"))
                .collect::<Vec<_>>();
            paths.sort();
            paths
        })
        .unwrap_or_default()
}

/// Certifies the actual delivery bytes against the render plan before they
/// replace the published output. A failed certification leaves the previous
/// published file untouched.
fn certify_and_publish(
    part: &str,
    output: &str,
    plan: &CanonicalRenderPlan,
    profile: &VideoProfile,
    encoder: &EncoderPolicy,
    ffmpeg: &str,
    ffmpeg_ms: i64,
) -> Response {
    let ffprobe = crate::probe::ffprobe_path(ffmpeg);
    let expected_duration =
        plan.duration_frames as f64 * plan.fps_denominator as f64 / plan.fps_numerator as f64;
    let mut metadata = match crate::probe::probe_file(&ffprobe, part) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = fs::remove_file(part);
            return Response::failure(
                Operation::RenderStock,
                None,
                format!("canonical render output could not be probed: {error}"),
            );
        }
    };
    if let Err(error) = validate_final_output(
        &metadata,
        expected_duration,
        plan.fps_numerator,
        plan.fps_denominator,
        profile.width,
        profile.height,
        &encoder.codec,
    ) {
        let _ = fs::remove_file(part);
        return Response::failure(
            Operation::RenderStock,
            None,
            format!("canonical render output failed certification: {error}"),
        );
    }
    // One ffmpeg process, one measurement: this executor has no separate
    // encoder phase to time, so `encode_ms` deliberately mirrors `ffmpeg_ms`
    // rather than inventing a second number. Readers must not treat the two as
    // independent phases; a real split needs per-stage instrumentation, not a
    // copy of the wall clock with a different label.
    metadata.ffmpeg_ms = Some(ffmpeg_ms.max(1));
    metadata.encode_ms = Some(ffmpeg_ms.max(1));
    match publish_output(part, output) {
        Ok(()) => Response::success(Operation::RenderStock, None, Vec::new(), Some(metadata)),
        Err(error) => Response::failure(Operation::RenderStock, None, error),
    }
}
/// Verifies the actual delivery bytes against the render plan. This is the
/// certification that prevents `ok: true` from being returned for a truncated,
/// audio-only, wrongly-sized or wrongly-encoded file.
fn validate_final_output(
    metadata: &MediaMetadata,
    expected_duration: f64,
    expected_fps_num: i64,
    expected_fps_den: i64,
    expected_width: u32,
    expected_height: u32,
    expected_codec: &str,
) -> Result<(), String> {
    if !metadata.has_video || metadata.video_stream_count != 1 {
        return Err(format!(
            "expected exactly one video stream, got {}",
            metadata.video_stream_count
        ));
    }
    if metadata.has_audio || metadata.audio_stream_count != 0 {
        return Err(format!(
            "expected no audio stream, got {}",
            metadata.audio_stream_count
        ));
    }
    if metadata.width != expected_width || metadata.height != expected_height {
        return Err(format!(
            "geometry {}x{} does not match {}x{}",
            metadata.width, metadata.height, expected_width, expected_height
        ));
    }
    if metadata.fps_den == 0
        || i64::from(metadata.fps_num) * expected_fps_den
            != expected_fps_num * i64::from(metadata.fps_den)
    {
        return Err(format!(
            "frame rate {}/{} does not match {}/{}",
            metadata.fps_num, metadata.fps_den, expected_fps_num, expected_fps_den
        ));
    }
    if !crate::probe::video_codec_matches(metadata.video_codec.as_deref(), expected_codec) {
        return Err(format!(
            "codec {:?} does not match {expected_codec}",
            metadata.video_codec
        ));
    }
    if metadata.pixel_format.as_deref() != Some("yuv420p") {
        return Err(format!(
            "pixel format {:?} is not yuv420p",
            metadata.pixel_format
        ));
    }
    if !metadata.duration_sec.is_finite()
        || (metadata.duration_sec - expected_duration).abs() > 0.25
    {
        return Err(format!(
            "duration {:.3}s does not match expected {:.3}s",
            metadata.duration_sec, expected_duration
        ));
    }
    Ok(())
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
                let job = match jobs.lock() {
                    Ok(mut guard) => guard.pop_front(),
                    Err(poisoned) => poisoned.into_inner().pop_front(),
                };
                let Some(job) = job else { return };
                let result = job.command.output();
                let mut results = match results.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                results.push((job.index, result));
            });
        }
    });

    let mut results = results
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    fps_num: i64,
    fps_den: i64,
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
    // The concat demuxer derives each next segment's timestamp offset from
    // the prior MP4 duration. Some FFmpeg muxers round each short fragment's
    // terminal packet duration independently, so stream-copy concatenation
    // can accumulate a sub-frame drift (for example 30 frames reported as
    // 22500/751 instead of 30/1). Re-stamp the already encoded, no-B-frame
    // packets onto the sealed CFR clock; this changes timestamps only, not
    // the H.264 bitstream or its certification hash.
    let ticks_per_frame = format!("90000*{fps_den}/{fps_num}");
    let pts = format!("round(N*{ticks_per_frame})");
    let duration = format!("round((N+1)*{ticks_per_frame})-round(N*{ticks_per_frame})");
    let timestamps = format!("setts=time_base=1/90000:pts={pts}:dts={pts}:duration={duration}");
    command.args(["-bsf:v", &timestamps]);
    command.args([
        "-c",
        "copy",
        "-an",
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
        "-video_track_timescale",
        "90000",
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

#[cfg(test)]
mod tests {
    use super::validate_final_output;
    use crate::protocol::MediaMetadata;

    fn metadata() -> MediaMetadata {
        MediaMetadata {
            duration_sec: 2.0,
            width: 320,
            height: 240,
            fps: 30.0,
            fps_num: 30,
            fps_den: 1,
            video_codec: Some("h264".to_string()),
            pixel_format: Some("yuv420p".to_string()),
            stream_count: 1,
            video_stream_count: 1,
            audio_stream_count: 0,
            has_video: true,
            has_audio: false,
            ..Default::default()
        }
    }

    #[test]
    fn final_output_certification_accepts_a_matching_probe() {
        assert!(validate_final_output(&metadata(), 2.0, 30, 1, 320, 240, "libx264").is_ok());
    }

    #[test]
    fn final_output_certification_rejects_contract_violations() {
        let mut wrong_geometry = metadata();
        wrong_geometry.width = 640;
        assert!(validate_final_output(&wrong_geometry, 2.0, 30, 1, 320, 240, "libx264").is_err());

        let mut missing_video = metadata();
        missing_video.video_stream_count = 0;
        missing_video.has_video = false;
        assert!(validate_final_output(&missing_video, 2.0, 30, 1, 320, 240, "libx264").is_err());

        let mut audio_present = metadata();
        audio_present.audio_stream_count = 1;
        audio_present.has_audio = true;
        assert!(validate_final_output(&audio_present, 2.0, 30, 1, 320, 240, "libx264").is_err());

        let mut wrong_fps = metadata();
        wrong_fps.fps_num = 25;
        assert!(validate_final_output(&wrong_fps, 2.0, 30, 1, 320, 240, "libx264").is_err());

        let mut wrong_duration = metadata();
        wrong_duration.duration_sec = 5.0;
        assert!(validate_final_output(&wrong_duration, 2.0, 30, 1, 320, 240, "libx264").is_err());
    }
}
