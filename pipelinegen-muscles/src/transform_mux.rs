use crate::artifact::{failed_response, part_path, publish_output};
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;
use std::path::Path;

pub(super) fn execute(request: Request) -> Response {
    let inputs = request.input_paths.as_deref().unwrap_or_default();
    if inputs.len() != 2 {
        return failed_response(None, "mux_audio_copy requires exactly video and final audio inputs".to_string());
    }
    if let Some(path) = inputs.iter().find(|path| !Path::new(path).is_file()) {
        return failed_response(Some(path.to_string()), format!("source file is not readable: {path}"));
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(Some(inputs[0].to_string()), "output_path is required".to_string()),
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) { return failed_response(Some(inputs[0].to_string()), format!("create output directory: {error}")); }
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y", "-i", inputs[0].as_str(), "-i", inputs[1].as_str(), "-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "copy", "-shortest", "-movflags", "+faststart", &part]);
    match command.output() {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response { ok: true, operation: "mux_audio_copy".to_string(), source_path: Some(inputs[0].to_string()), items: Vec::new(), metadata: None, error: None },
            Err(error) => failed_response(Some(inputs[0].to_string()), error),
        },
        Ok(result) => { let _ = fs::remove_file(&part); failed_response(Some(inputs[0].to_string()), format!("mux_audio_copy failed: {}", String::from_utf8_lossy(&result.stderr).trim())) }
        Err(error) => { let _ = fs::remove_file(&part); failed_response(Some(inputs[0].to_string()), format!("mux_audio_copy failed to start: {error}")) }
    }
}
