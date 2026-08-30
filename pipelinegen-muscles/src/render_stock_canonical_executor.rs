use super::plan::CanonicalRenderPlan;
use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use std::collections::HashMap;
use std::fs;

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
                profile.width,
                profile.height,
                plan.fps_numerator,
                plan.fps_denominator,
                gap_seconds,
                gap_frames,
                filler_index
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
        if segment.freeze {
            // Hold the clip's final frame across the tail's timeline frames.
            // tpad clones the last frame for the remaining duration so only
            // the tail is synthesized; the real clip stays a trimmed copy.
            let hold_frames = segment.timeline.frame_count.saturating_sub(1);
            let hold_seconds =
                (hold_frames as f64 * plan.fps_denominator as f64) / plan.fps_numerator as f64;
            filter.push_str(&format!(
                "[{index}:v]trim=start_frame={}:end_frame={},setpts=PTS-STARTPTS,scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1,tpad=stop_mode=clone:stop_duration={:.9},setpts=PTS-STARTPTS[v{index}];",
                segment.source.start_frame,
                end_frame,
                profile.width,
                profile.height,
                profile.width,
                profile.height,
                format!("{}/{}", plan.fps_numerator, plan.fps_denominator),
                hold_seconds
            ));
        } else {
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
        }
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
    if let Err(error) = append_video_options(&mut command, request) {
        return failed_response(None, error);
    }
    command.args(["-an", "-movflags", "+faststart", &part]);
    let encode_started = std::time::Instant::now();
    let encode_result = command.output();
    let ffmpeg_ms = encode_started.elapsed().as_millis() as i64;
    match encode_result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
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
                        gpu_copy_bytes: None,
                        video_zero_copy: None,
                        decode_ms: None,
                        filter_graph_ms: None,
                        subtitle_raster_ms: None,
                        watermark_raster_ms: None,
                        frame_conversion_ms: None,
                        encode_ms: Some(ffmpeg_ms.max(1)),
                        audio_mux_ms: None,
                    }),
                    error: None,
                }
            }
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
