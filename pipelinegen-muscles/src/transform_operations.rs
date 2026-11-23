use super::finish_run;
use crate::artifact::part_path;
use crate::encoder::append_video_options;
use crate::process::{FFmpegRunner, ProcessCommand};
use crate::protocol::{AudioEditMetadata, MediaMetadata};
use crate::protocol::{Operation, Request, Response};
use std::fs;
use std::path::Path;

/// Configuration failures carry the full failure envelope. Boxing keeps the
/// `Result` small on the success path (clippy's `result_large_err`).
type ConfigResult<T> = Result<T, Box<Response>>;

fn config_failure(operation: Operation, input: &str, error: impl ToString) -> Box<Response> {
    Box::new(Response::failure(operation, Some(input.to_string()), error))
}

pub(super) fn execute(request: Request, operation: Operation) -> Response {
    let input = match request.source_path.as_deref() {
        Some(path)
            if !path.is_empty()
                && (operation == Operation::RemuxHls || Path::new(path).is_file()) =>
        {
            path
        }
        Some(path) => {
            return Response::failure(
                operation,
                Some(path.to_string()),
                format!("source file is not readable: {path}"),
            )
        }
        None => return Response::failure(operation, None, "source_path is required".to_string()),
    };
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => {
            return Response::failure(
                operation,
                Some(input.to_string()),
                "output_path is required".to_string(),
            )
        }
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Response::failure(
                operation,
                Some(input.to_string()),
                format!("create output directory: {error}"),
            );
        }
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    let metadata = match configure_transform(&request, operation, ffmpeg, input, &mut command) {
        Ok(metadata) => metadata,
        Err(response) => return *response,
    };
    let part = part_path(output);
    command.args(["-movflags", "+faststart", &part]);
    let result = command.output();
    finish_run(operation, input, &part, output, result, metadata)
}

/// Configures the ffmpeg graph for every transform operation. The long
/// per-operation command construction lives in one focused helper each, so the
/// dispatcher stays auditable against the operation enum.
fn configure_transform(
    request: &Request,
    operation: Operation,
    ffmpeg: &str,
    input: &str,
    command: &mut ProcessCommand,
) -> ConfigResult<Option<MediaMetadata>> {
    match operation {
        Operation::Normalize => {
            configure_normalize(request, input, command)?;
            Ok(None)
        }
        Operation::ExtractFrame => {
            command.args([
                "-ss",
                &request.timestamp_sec.unwrap_or(0.0).to_string(),
                "-i",
                input,
                "-frames:v",
                "1",
                "-an",
            ]);
            Ok(None)
        }
        Operation::CutCopy => {
            configure_cut_copy(request, input, command);
            Ok(None)
        }
        Operation::CutAndNormalize => {
            configure_cut_and_normalize(request, input, command)?;
            Ok(None)
        }
        Operation::GenerateProxy => {
            configure_generate_proxy(request, input, command)?;
            Ok(None)
        }
        Operation::GenerateStoryboard => {
            configure_storyboard(request, input, command);
            Ok(None)
        }
        Operation::RemuxHls => {
            command.args(["-i", input, "-c", "copy"]);
            Ok(None)
        }
        Operation::RemoveSilence => configure_remove_silence(ffmpeg, input, command),
        Operation::Trim => {
            configure_trim(request, input, command)?;
            Ok(None)
        }
        _ => Err(config_failure(
            operation,
            input,
            format!("unsupported transform: {}", operation.as_str()),
        )),
    }
}

fn configure_normalize(
    request: &Request,
    input: &str,
    command: &mut ProcessCommand,
) -> ConfigResult<()> {
    let profile = request
        .media
        .profile()
        .map_err(|error| config_failure(Operation::Normalize, input, error))?;
    command.args(["-i", input, "-vf"]);
    command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}/{},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps_num, profile.fps_den));
    append_video_options(command, request)
        .map_err(|error| config_failure(Operation::Normalize, input, error))?;
    if request.keep_audio.unwrap_or(false) {
        command.args([
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
    Ok(())
}

fn configure_cut_copy(request: &Request, input: &str, command: &mut ProcessCommand) {
    command.args([
        "-ss",
        &request.start_sec.unwrap_or(0.0).to_string(),
        "-i",
        input,
    ]);
    if let Some(end) = request.end_sec.filter(|value| *value > 0.0) {
        command.args(["-t", &(end - request.start_sec.unwrap_or(0.0)).to_string()]);
    }
    command.args(["-map", "0:v:0?", "-map", "0:a:0?", "-c", "copy"]);
    if request.no_audio.unwrap_or(false) {
        command.arg("-an");
    }
}

fn configure_cut_and_normalize(
    request: &Request,
    input: &str,
    command: &mut ProcessCommand,
) -> ConfigResult<()> {
    let profile = request
        .media
        .profile()
        .map_err(|error| config_failure(Operation::CutAndNormalize, input, error))?;
    let start = request.start_sec.unwrap_or(0.0);
    let end = request.end_sec.unwrap_or(start);
    command.args([
        "-ss",
        &start.to_string(),
        "-i",
        input,
        "-t",
        &(end - start).to_string(),
        "-vf",
    ]);
    command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}/{},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps_num, profile.fps_den));
    append_video_options(command, request)
        .map_err(|error| config_failure(Operation::CutAndNormalize, input, error))?;
    if request.no_audio.unwrap_or(false) {
        command.arg("-an");
    } else {
        command.args([
            "-c:a",
            profile.audio_codec.as_str(),
            "-ar",
            &profile.sample_rate.to_string(),
            "-ac",
            &profile.channels.to_string(),
        ]);
    }
    Ok(())
}

fn configure_generate_proxy(
    request: &Request,
    input: &str,
    command: &mut ProcessCommand,
) -> ConfigResult<()> {
    command.args(["-i", input, "-vf", "scale=-2:720"]);
    append_video_options(command, request)
        .map_err(|error| config_failure(Operation::GenerateProxy, input, error))?;
    let profile = request
        .media
        .profile()
        .map_err(|error| config_failure(Operation::GenerateProxy, input, error))?;
    command.args([
        "-c:a",
        profile.audio_codec.as_str(),
        "-ar",
        &profile.sample_rate.to_string(),
        "-ac",
        &profile.channels.to_string(),
    ]);
    Ok(())
}

fn configure_storyboard(request: &Request, input: &str, command: &mut ProcessCommand) {
    let interval = request.interval_frames.unwrap_or(10).max(1);
    let columns = request.columns.unwrap_or(5).max(1);
    let rows = request.rows.unwrap_or(5).max(1);
    command.args(["-i", input, "-vf"]);
    command.arg(format!(
        "select=not(mod(n\\,{interval})),scale=320:-1,tile={columns}x{rows}"
    ));
    command.args(["-frames:v", "1", "-an"]);
}

fn configure_remove_silence(
    ffmpeg: &str,
    input: &str,
    command: &mut ProcessCommand,
) -> ConfigResult<Option<MediaMetadata>> {
    // Preserve a short 120 ms breathing gap and remove only the excess in
    // longer pauses. The detector and silenceremove thresholds stay identical
    // so timing edits describe the actual cuts.
    //
    // P0 mix canonicalization (2026-08-26): the cleaned output is normalized
    // to the canonical 48 kHz stereo mix profile in the SAME pass that strips
    // silence, so the renderer's source_normalize can skip
    // aresample/aformat for every voiceover source. Edge TTS emits 24 kHz
    // mono; without this the mix graph upmixed and resampled each event on
    // every render. The fingerprint bump (voiceover-content-v2) ensures
    // pre-canonicalization cache rows are never reused.
    let edits = detect_silence_edits(ffmpeg, input).map_err(|error| {
        config_failure(
            Operation::RemoveSilence,
            input,
            format!("remove_silence detection failed: {error}"),
        )
    })?;
    let metadata = MediaMetadata {
        audio_edits: edits,
        ..Default::default()
    };
    let canonical_rate = crate::config::CANONICAL_SAMPLE_RATE.to_string();
    command.args(["-i", input, "-af", "silenceremove=start_periods=1:start_threshold=-40dB:start_silence=0.12:stop_periods=-1:stop_threshold=-40dB:stop_silence=0.12", "-c:a", "libmp3lame", "-q:a", "2", "-ar", canonical_rate.as_str(), "-ac", "2"]);
    Ok(Some(metadata))
}

fn configure_trim(
    request: &Request,
    input: &str,
    command: &mut ProcessCommand,
) -> ConfigResult<()> {
    let max_duration = request.max_duration_sec.unwrap_or(0.0);
    if !max_duration.is_finite() || max_duration <= 0.0 {
        return Err(config_failure(
            Operation::Trim,
            input,
            "max_duration_sec must be positive".to_string(),
        ));
    }
    command.args(["-i", input, "-t", &max_duration.to_string()]);
    let extension = Path::new(input)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "mp4" | "mov" | "mkv") {
        let profile = request
            .media
            .profile()
            .map_err(|error| config_failure(Operation::Trim, input, error))?;
        command.args(["-map", "0:v:0?", "-map", "0:a:0?"]);
        append_video_options(command, request)
            .map_err(|error| config_failure(Operation::Trim, input, error))?;
        command.args([
            "-c:a",
            profile.audio_codec.as_str(),
            "-b:a",
            profile.audio_bitrate.as_str(),
            "-ar",
            &profile.sample_rate.to_string(),
            "-ac",
            &profile.channels.to_string(),
        ]);
    } else if extension == "wav" {
        command.args(["-vn", "-c:a", "pcm_s16le"]);
    } else {
        command.args(["-vn", "-c:a", "libmp3lame", "-q:a", "2"]);
    }
    Ok(())
}

fn detect_silence_edits(ffmpeg: &str, input: &str) -> Result<Vec<AudioEditMetadata>, String> {
    let runner = crate::process::FFmpegRunner::from_ffmpeg_path(ffmpeg);
    let mut command = runner.ffmpeg();
    command.args([
        "-hide_banner",
        "-i",
        input,
        "-af",
        "silencedetect=noise=-40dB:d=0.12",
        "-f",
        "null",
        "-",
    ]);
    let result = command.output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).trim().to_string());
    }
    let mut start: Option<f64> = None;
    let mut edits = Vec::new();
    let mut removed_before = 0.0f64;
    for line in String::from_utf8_lossy(&result.stderr).lines() {
        if let Some(value) = line.split("silence_start:").nth(1) {
            start = value.split_whitespace().next().and_then(|v| v.parse().ok());
        } else if let Some(value) = line.split("silence_end:").nth(1) {
            let end: f64 = value
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or("invalid silence_end from ffmpeg")?;
            if let Some(begin) = start.take() {
                let cut_start = (begin + 0.12).min(end);
                if end - cut_start > 0.001 {
                    let source_start_us = (cut_start * 1_000_000.0).round() as i64;
                    let source_end_us = (end * 1_000_000.0).round() as i64;
                    let output_us = ((cut_start - removed_before) * 1_000_000.0).round() as i64;
                    edits.push(AudioEditMetadata {
                        source_start_us,
                        source_end_us,
                        output_start_us: output_us,
                        output_end_us: output_us,
                    });
                    removed_before += end - cut_start;
                }
            }
        }
    }
    if let Some(begin) = start {
        let probe = crate::probe::probe_file(&crate::probe::ffprobe_path(ffmpeg), input)
            .map_err(|e| e.to_string())?;
        let end = probe.duration_sec;
        let cut_start = (begin + 0.12).min(end);
        if end - cut_start > 0.001 {
            let source_start_us = (cut_start * 1_000_000.0).round() as i64;
            let source_end_us = (end * 1_000_000.0).round() as i64;
            let output_us = ((cut_start - removed_before) * 1_000_000.0).round() as i64;
            edits.push(AudioEditMetadata {
                source_start_us,
                source_end_us,
                output_start_us: output_us,
                output_end_us: output_us,
            });
        }
    }
    Ok(edits)
}
