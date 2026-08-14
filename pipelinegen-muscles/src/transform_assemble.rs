use crate::artifact::{failed_response, part_path, publish_output};
use crate::probe::{ffprobe_path, probe_file};
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;
use std::path::{Path, PathBuf};

/// Implements `video.assemble.copy.v1`: assemble certified overlay segments by
/// packet-copy (stream copy) with zero decode, zero encode and zero compositing.
///
/// Every input must satisfy the certified copy-only profile (codec, geometry,
/// frame rate, closed GOP, first frame keyframe) or the operation fails closed;
/// the encoded composite path remains responsible otherwise.
pub(super) fn execute(request: Request) -> Response {
    let cert = match request.copy_certification.as_ref() {
        Some(cert) => cert,
        None => {
            return failed_response(
                None,
                "copy_certification is required for assemble_copy".to_string(),
            )
        }
    };
    if let Err(error) = cert.validate() {
        return failed_response(None, error);
    }

    let inputs = request.input_paths.as_deref().unwrap_or_default();
    if inputs.is_empty() {
        return failed_response(None, "input_paths are required".to_string());
    }
    if let Some(path) = inputs.iter().find(|path| !Path::new(path).is_file()) {
        return failed_response(
            Some(path.to_string()),
            format!("source file is not readable: {path}"),
        );
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => {
            return failed_response(
                Some(inputs[0].to_string()),
                "output_path is required".to_string(),
            )
        }
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(
                Some(inputs[0].to_string()),
                format!("create output directory: {error}"),
            );
        }
    }

    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let ffprobe = ffprobe_path(ffmpeg);
    for input in inputs {
        let metadata = match probe_file(&ffprobe, input) {
            Ok(metadata) => metadata,
            Err(error) => return failed_response(Some(input.to_string()), error),
        };
        if let Err(error) = cert.verify_metadata(&metadata) {
            return failed_response(Some(input.to_string()), error);
        }
    }

    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);

    let mut concat_list: Option<PathBuf> = None;
    if inputs.len() == 1 {
        command.args(["-i", inputs[0].as_str(), "-map", "0:v:0", "-c", "copy"]);
    } else {
        let list_path = std::env::temp_dir().join(format!(
            "pipelinegen_assemble_copy_{}.txt",
            std::process::id()
        ));
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
        let list = list_path.to_string_lossy().into_owned();
        command.args([
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            list.as_str(),
            "-map",
            "0:v:0",
            "-c",
            "copy",
        ]);
        concat_list = Some(list_path);
    }

    command.args(["-movflags", "+faststart", &part]);
    let result = command.output();
    if let Some(path) = &concat_list {
        let _ = fs::remove_file(path);
    }

    match result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "assemble_copy".to_string(),
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
                    "assemble_copy failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(
                Some(inputs[0].to_string()),
                format!("assemble_copy failed to start: {error}"),
            )
        }
    }
}
