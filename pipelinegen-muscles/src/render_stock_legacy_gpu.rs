use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;
use super::copy;

pub(super) fn render_stock_gpu_composite(
    request: &Request,
    inputs: &[String],
    output: &str,
    ffmpeg: &str,
) -> Response {
    let (source, temporary_source) = match copy::concat_source(inputs, output, ffmpeg, "gpu") {
        Ok(value) => value,
        Err(error) => return failed_response(None, error),
    };
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner", "-loglevel", "error", "-y", "-hwaccel", "cuda",
        "-hwaccel_output_format", "cuda", "-i", &source, "-map", "0:v:0?",
    ]);
    if request.keep_audio.unwrap_or(false) {
        command.args(["-map", "0:a:0?"]);
    } else {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, request) {
        if temporary_source { let _ = fs::remove_file(&source); }
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);
    let result = command.output();
    if temporary_source { let _ = fs::remove_file(&source); }
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
            failed_response(None, format!("GPU composite render failed: {}", String::from_utf8_lossy(&result.stderr).trim()))
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("GPU composite render failed: {error}"))
        }
    }
}
