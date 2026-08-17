use super::effects::{transition_filter, validate_resolved_render_plan};
use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use std::fs;
use std::path::Path;

#[path = "render_stock_legacy_copy.rs"]
mod copy;
#[path = "render_stock_legacy_fast.rs"]
mod fast;
#[path = "render_stock_legacy_gpu.rs"]
mod gpu;
#[path = "render_stock_legacy_policy.rs"]
mod policy;

pub(super) fn render_stock(request: Request) -> Response {
    let inputs = request.input_paths.clone().unwrap_or_default();
    if inputs.is_empty() {
        return failed_response(None, "input_paths is required".to_string());
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    if inputs.iter().any(|path| !Path::new(path).is_file()) {
        return failed_response(
            Some(inputs.join(",")),
            "a render input is not readable".to_string(),
        );
    }
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    if request.no_transitions.unwrap_or(false) && request.no_effects.unwrap_or(false) {
        return fast::render_stock_simple(&request, &inputs, output);
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let profile = match request.media.profile() {
        Ok(value) => value,
        Err(error) => return failed_response(None, error),
    };
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in &inputs {
        command.args(["-i", input]);
    }

    let duration = request.clip_duration_sec.unwrap_or(0);
    let transitions = request.transitions.clone().unwrap_or_default();
    let effect_assignments = request.effect_paths.clone().unwrap_or_default();
    if let Err(error) = validate_resolved_render_plan(
        inputs.len(),
        request.no_transitions.unwrap_or(false),
        &transitions,
        request.no_effects.unwrap_or(false),
        &effect_assignments,
    ) {
        return failed_response(None, error);
    }

    let mut effect_for_clip: Vec<Option<(usize, String)>> = vec![None; inputs.len()];
    for (assignment_index, effect) in effect_assignments.iter().enumerate() {
        if effect.clip_index >= inputs.len() || !Path::new(&effect.path).is_file() {
            return failed_response(
                None,
                format!("invalid resolved effect path: {}", effect.path),
            );
        }
        command.args(["-i", &effect.path]);
        effect_for_clip[effect.clip_index] =
            Some((inputs.len() + assignment_index, effect.path.clone()));
    }

    let mut filter = String::new();
    for index in 0..inputs.len() {
        let mut filters = vec![format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps)];
        if !request.no_transitions.unwrap_or(false) {
            for transition in transitions
                .iter()
                .filter(|transition| transition.clip_index == index)
            {
                filters.push(transition_filter(
                    &transition.id,
                    duration,
                    transition.segment == "start",
                ));
            }
        }
        let effect = effect_for_clip[index].as_ref();
        let with_effect = effect.is_some() && !request.no_effects.unwrap_or(false);
        if with_effect {
            let (effect_input, _) = effect.expect("checked above");
            let opacity = request.overlay_opacity.unwrap_or(0.25);
            filter.push_str(&format!("[{}:v]{}[vtemp{}];[{}:v]scale={}:{}:force_original_aspect_ratio=decrease,fps={},setsar=1,format=yuva420p,colorchannelmixer=aa={}[effect{}];[vtemp{}][effect{}]overlay=shortest=1[v{}];", index, filters.join(","), index, effect_input, profile.width, profile.height, profile.fps, opacity, index, index, index, index));
        } else {
            filter.push_str(&format!("[{}:v]{}[v{}];", index, filters.join(","), index));
        }
    }
    for index in 0..inputs.len() {
        filter.push_str(&format!("[v{}]", index));
    }
    filter.push_str(&format!("concat=n={}:v=1:a=0[vfinal]", inputs.len()));
    command.args(["-filter_complex", &filter, "-map", "[vfinal]"]);
    if !request.keep_audio.unwrap_or(false) {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, &request) {
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);
    let encode_started = std::time::Instant::now();
    let encode_result = command.output();
    let ffmpeg_ms = encode_started.elapsed().as_millis() as i64;
    match encode_result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                // The legacy composite path does not probe its output, so the
                // media fields report the resolved profile plus the native
                // ffmpeg encode wall time only; duration_sec stays 0.
                metadata: Some(MediaMetadata {
                    duration_sec: 0.0,
                    bitrate: None,
                    width: profile.width,
                    height: profile.height,
                    fps: profile.fps as f64,
                    video_codec: None,
                    pixel_format: None,
                    audio_codec: None,
                    audio_profile: None,
                    sample_rate: None,
                    channels: None,
                    start_pts: None,
                    has_video: true,
                    has_audio: request.keep_audio.unwrap_or(false),
                    mix_ms: None,
                    aac_encode_ms: None,
                    probe_ms: None,
                    hash_ms: None,
                    ffmpeg_ms: Some(ffmpeg_ms.max(1)),
                    final_audio_sha256: None,
                }),
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "stock render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("stock render failed: {error}"))
        }
    }
}
