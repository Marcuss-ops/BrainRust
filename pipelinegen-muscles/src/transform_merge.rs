use crate::artifact::{failed_response, part_path, publish_output};
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;
use std::path::Path;

pub(super) fn execute(request: Request) -> Response {
    let inputs = request.input_paths.as_deref().unwrap_or_default();
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    if inputs.is_empty() {
        return failed_response(None, "input_paths are required".to_string());
    }
    if let Some(path) = inputs.iter().find(|path| !Path::new(path).is_file()) {
        return failed_response(
            Some(path.to_string()),
            format!("source file is not readable: {path}"),
        );
    }
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(
                Some(inputs[0].to_string()),
                format!("create output directory: {error}"),
            );
        }
    }
    let list_path =
        std::env::temp_dir().join(format!("pipelinegen_concat_{}.txt", std::process::id()));
    let content = inputs
        .iter()
        .map(|path| {
            format!(
                "file '{}'",
                path.replace('\\', "\\\\").replace('\'', "'\\''")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if let Err(error) = fs::write(&list_path, content) {
        return failed_response(
            Some(inputs[0].to_string()),
            format!("write concat list: {error}"),
        );
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let part = part_path(output);
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
    ]);
    command.arg(list_path.to_string_lossy().to_string());
    command.args(["-c", "copy", &part]);
    let result = command.output();
    let _ = fs::remove_file(&list_path);
    match result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "merge_inputs".to_string(),
                source_path: Some(inputs[0].to_string()),
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(Some(inputs[0].to_string()), error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                Some(inputs[0].to_string()),
                format!(
                    "merge_inputs failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(
                Some(inputs[0].to_string()),
                format!("merge_inputs failed to start: {error}"),
            )
        }
    }
}
