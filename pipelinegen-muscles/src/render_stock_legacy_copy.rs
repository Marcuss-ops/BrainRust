use crate::artifact::{failed_response, part_path, publish_output};
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;

pub(super) fn concat_source(
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

pub(super) fn render_stock_copy_only(
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
