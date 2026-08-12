use super::effects::{transition_filter, validate_resolved_render_plan};
use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::probe::{ffprobe_path, probe_file};
use crate::process::FFmpegRunner;
use crate::protocol::{self, MediaMetadata, Request, Response};
use std::fs;
use std::path::Path;

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
        return render_stock_simple(&request, &inputs, output);
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

fn render_stock_simple(request: &Request, inputs: &[String], output: &str) -> Response {
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    match copy_only_eligibility(request, inputs, ffmpeg) {
        Ok(true) => return render_stock_copy_only(request, inputs, output, ffmpeg),
        Ok(false) => {}
        Err(error) => return failed_response(None, error),
    }
    if gpu_composite_eligibility(request, inputs, ffmpeg) {
        return render_stock_gpu_composite(request, inputs, output, ffmpeg);
    }
    let source = if inputs.len() == 1 {
        inputs[0].clone()
    } else {
        let list_path = format!("{output}.rust.concat.txt");
        let concat_path = format!("{output}.rust.concat.mp4");
        let lines = inputs
            .iter()
            .map(|path| format!("file '{}'", path.replace('\'', "'\\''")))
            .collect::<Vec<_>>();
        if let Err(error) = fs::write(&list_path, lines.join("\n")) {
            return failed_response(None, format!("write concat list: {error}"));
        }
        let mut concat = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
        concat.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            &list_path,
            "-c",
            "copy",
            &concat_path,
        ]);
        let result = concat.output();
        let _ = fs::remove_file(&list_path);
        match result {
            Ok(result) if result.status.success() => concat_path,
            Ok(result) => {
                let _ = fs::remove_file(&concat_path);
                return failed_response(
                    None,
                    format!(
                        "stock concat failed: {}",
                        String::from_utf8_lossy(&result.stderr).trim()
                    ),
                );
            }
            Err(error) => {
                let _ = fs::remove_file(&concat_path);
                return failed_response(None, format!("stock concat failed to start: {error}"));
            }
        }
    };
    let profile = match request.media.profile() {
        Ok(value) => value,
        Err(error) => {
            if inputs.len() > 1 {
                let _ = fs::remove_file(&source);
            }
            return failed_response(None, error);
        }
    };
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        &source,
        "-vf",
    ]);
    command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps));
    command.args(["-map", "0:v:0?"]);
    if request.keep_audio.unwrap_or(false) {
        command.args([
            "-map",
            "0:a:0?",
            "-c:a",
            profile.audio_codec.as_str(),
            "-ar",
            &profile.sample_rate.to_string(),
            "-ac",
            &profile.channels.to_string(),
        ]);
    } else {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, request) {
        if inputs.len() > 1 {
            let _ = fs::remove_file(&source);
        }
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);
    let result = command.output();
    if inputs.len() > 1 {
        let _ = fs::remove_file(&source);
    }
    match result {
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
                    "stock normalize failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("stock normalize failed to start: {error}"))
        }
    }
}

/// GPU compositing is deliberately narrower than GPU encoding. It is selected
/// only when NVENC is resolved, no pixel effect/transition needs a CPU-only
/// filter, and all inputs already share the requested geometry and frame rate.
/// This prevents a hidden GPU->CPU->GPU transfer and leaves resize/pad/fps work
/// on the safe CPU composite path until a CUDA-compatible graph exists.
fn gpu_composite_eligibility(request: &Request, inputs: &[String], ffmpeg: &str) -> bool {
    if !request.no_transitions.unwrap_or(false)
        || !request.no_effects.unwrap_or(false)
        || request
            .transitions
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || request
            .effect_paths
            .as_ref()
            .is_some_and(|items| !items.is_empty())
    {
        return false;
    }
    let Ok(profile) = request.media.profile() else {
        return false;
    };
    let Ok(encoder) = request.media.encoder() else {
        return false;
    };
    if !encoder.codec.to_ascii_lowercase().ends_with("_nvenc") {
        return false;
    }
    let ffprobe = ffprobe_path(ffmpeg);
    let mut codec: Option<String> = None;
    for input in inputs {
        let Ok(metadata) = probe_file(&ffprobe, input) else {
            return false;
        };
        if !metadata.has_video
            || metadata.width != profile.width
            || metadata.height != profile.height
            || (metadata.fps - profile.fps as f64).abs() > 0.5
            || metadata.video_codec.is_none()
        {
            return false;
        }
        if let Some(reference) = &codec {
            if metadata.video_codec.as_deref() != Some(reference.as_str()) {
                return false;
            }
        } else {
            codec = metadata.video_codec;
        }
    }
    true
}

fn render_stock_gpu_composite(
    request: &Request,
    inputs: &[String],
    output: &str,
    ffmpeg: &str,
) -> Response {
    let (source, temporary_source) = match concat_source(inputs, output, ffmpeg, "gpu") {
        Ok(value) => value,
        Err(error) => return failed_response(None, error),
    };
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-hwaccel",
        "cuda",
        "-hwaccel_output_format",
        "cuda",
        "-i",
        &source,
        "-map",
        "0:v:0?",
    ]);
    if request.keep_audio.unwrap_or(false) {
        command.args(["-map", "0:a:0?"]);
    } else {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, request) {
        if temporary_source {
            let _ = fs::remove_file(&source);
        }
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);
    let result = command.output();
    if temporary_source {
        let _ = fs::remove_file(&source);
    }
    match result {
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
                    "GPU composite render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("GPU composite render failed: {error}"))
        }
    }
}

fn concat_source(
    inputs: &[String],
    output: &str,
    ffmpeg: &str,
    suffix: &str,
) -> Result<(String, bool), String> {
    if inputs.len() == 1 {
        return Ok((inputs[0].clone(), false));
    }
    let list_path = format!("{output}.rust.{suffix}.concat.txt");
    let concat_path = format!("{output}.rust.{suffix}.concat.mp4");
    let lines = inputs
        .iter()
        .map(|path| format!("file '{}'", path.replace('\'', "'\\''")))
        .collect::<Vec<_>>();
    fs::write(&list_path, lines.join("\n"))
        .map_err(|error| format!("write {suffix} concat list: {error}"))?;
    let mut concat = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    concat.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &list_path,
        "-c",
        "copy",
        &concat_path,
    ]);
    let result = concat.output();
    let _ = fs::remove_file(&list_path);
    match result {
        Ok(result) if result.status.success() => Ok((concat_path, true)),
        Ok(result) => {
            let _ = fs::remove_file(&concat_path);
            Err(format!(
                "{suffix} concat failed: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            ))
        }
        Err(error) => {
            let _ = fs::remove_file(&concat_path);
            Err(format!("{suffix} concat failed to start: {error}"))
        }
    }
}

/// Resolves the stock fast path conservatively. A stream-copy render is only
/// safe when every input already satisfies the requested canonical contract;
/// otherwise the normal composite/encode path remains responsible for scale,
/// padding, frame-rate conversion, and codec normalization.
fn copy_only_eligibility(
    request: &Request,
    inputs: &[String],
    ffmpeg: &str,
) -> Result<bool, String> {
    if !request.no_transitions.unwrap_or(false)
        || !request.no_effects.unwrap_or(false)
        || request
            .transitions
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || request
            .effect_paths
            .as_ref()
            .is_some_and(|items| !items.is_empty())
    {
        return Ok(false);
    }

    let profile = request.media.profile()?;
    let encoder = request.media.encoder()?;
    let ffprobe = ffprobe_path(ffmpeg);
    let mut first: Option<MediaMetadata> = None;
    for input in inputs {
        let metadata = probe_file(&ffprobe, input)?;
        if !metadata.has_video
            || metadata.width != profile.width
            || metadata.height != profile.height
            || (metadata.fps - profile.fps as f64).abs() > 0.5
            || !video_codec_matches(metadata.video_codec.as_deref(), &encoder.codec)
            || metadata.pixel_format.as_deref() != Some("yuv420p")
        {
            return Ok(false);
        }
        if request.keep_audio.unwrap_or(false)
            && (!metadata.has_audio
                || metadata.audio_codec.as_deref() != Some(profile.audio_codec.as_str())
                || metadata.sample_rate != Some(profile.sample_rate)
                || metadata.channels != Some(profile.channels))
        {
            return Ok(false);
        }
        if let Some(reference) = &first {
            if metadata.video_codec != reference.video_codec
                || metadata.pixel_format != reference.pixel_format
                || metadata.audio_codec != reference.audio_codec
                || metadata.sample_rate != reference.sample_rate
                || metadata.channels != reference.channels
            {
                return Ok(false);
            }
        } else {
            first = Some(metadata);
        }
    }
    Ok(true)
}

fn video_codec_matches(actual: Option<&str>, expected: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    match actual {
        Some(codec) if expected.ends_with("_nvenc") => codec == "h264",
        Some(codec) if expected == "libx264" || expected == "h264" => codec == "h264",
        Some(codec) => codec == expected,
        None => false,
    }
}

fn render_stock_copy_only(
    request: &Request,
    inputs: &[String],
    output: &str,
    ffmpeg: &str,
) -> Response {
    let part = part_path(output);
    let list_path = format!("{output}.rust.copy.concat.txt");
    let lines = inputs
        .iter()
        .map(|path| format!("file '{}'", path.replace('\'', "'\\''")))
        .collect::<Vec<_>>();
    if let Err(error) = fs::write(&list_path, lines.join("\n")) {
        return failed_response(None, format!("write copy concat list: {error}"));
    }
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &list_path,
    ]);
    if !request.keep_audio.unwrap_or(false) {
        command.arg("-an");
    }
    command.args([
        "-map",
        "0:v:0",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        &part,
    ]);
    let result = command.output();
    let _ = fs::remove_file(&list_path);
    match result {
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
                    "copy-only stock render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!("copy-only stock render failed to start: {error}"),
            )
        }
    }
}

