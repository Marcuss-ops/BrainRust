use super::{copy, gpu, policy};
use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;

pub(super) fn render_stock_simple(request: &Request, inputs: &[String], output: &str) -> Response {
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    match policy::copy_only_eligibility(request, inputs, ffmpeg) {
        Ok(true) => return copy::render_stock_copy_only(request, inputs, output, ffmpeg),
        Ok(false) => {}
        Err(error) => return failed_response(None, error),
    }
    if policy::gpu_composite_eligibility(request, inputs, ffmpeg) {
        return gpu::render_stock_gpu_composite(request, inputs, output, ffmpeg);
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
            "-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0",
            "-i", &list_path, "-c", "copy", &concat_path,
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
        "-hide_banner", "-loglevel", "error", "-y", "-i", &source, "-vf",
    ]);
    command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps));
    command.args(["-map", "0:v:0?"]);
    if request.keep_audio.unwrap_or(false) {
        command.args([
            "-map", "0:a:0?", "-c:a", profile.audio_codec.as_str(), "-ar",
            &profile.sample_rate.to_string(), "-ac", &profile.channels.to_string(),
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
