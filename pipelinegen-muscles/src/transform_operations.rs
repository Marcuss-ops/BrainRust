use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::process::FFmpegRunner;
use crate::protocol::{Request, Response};
use std::fs;
use std::path::Path;

pub(super) fn execute(request: Request, operation: &str) -> Response {
    let input = match request.source_path.as_deref() {
        Some(path) if !path.is_empty() && (operation == "remux_hls" || Path::new(path).is_file()) => path,
        Some(path) => return failed_response(Some(path.to_string()), format!("source file is not readable: {path}")),
        None => return failed_response(None, "source_path is required".to_string()),
    };
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(Some(input.to_string()), "output_path is required".to_string()),
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(Some(input.to_string()), format!("create output directory: {error}"));
        }
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    match operation {
        "normalize" => {
            let profile = match request.media.profile() {
                Ok(value) => value,
                Err(error) => return failed_response(Some(input.to_string()), error),
            };
            command.args(["-i", input, "-vf"]);
            command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}", profile.width, profile.height, profile.width, profile.height, profile.fps));
            if let Err(error) = append_video_options(&mut command, &request) {
                return failed_response(Some(input.to_string()), error);
            }
            if request.keep_audio.unwrap_or(false) {
                command.args(["-c:a", profile.audio_codec.as_str(), "-ar", &profile.sample_rate.to_string(), "-ac", &profile.channels.to_string()]);
            } else { command.arg("-an"); }
        }
        "extract_frame" => {
            command.args(["-ss", &request.timestamp_sec.unwrap_or(0.0).to_string(), "-i", input, "-frames:v", "1", "-an"]);
        },
        "cut_copy" => {
            command.args(["-ss", &request.start_sec.unwrap_or(0.0).to_string(), "-i", input]);
            if let Some(end) = request.end_sec.filter(|value| *value > 0.0) {
                command.args(["-t", &(end - request.start_sec.unwrap_or(0.0)).to_string()]);
            }
            command.args(["-map", "0:v:0?", "-map", "0:a:0?", "-c", "copy"]);
            if request.no_audio.unwrap_or(false) { command.arg("-an"); }
        }
        "cut_and_normalize" => {
            let profile = match request.media.profile() { Ok(value) => value, Err(error) => return failed_response(Some(input.to_string()), error) };
            let start = request.start_sec.unwrap_or(0.0);
            let end = request.end_sec.unwrap_or(start);
            command.args(["-ss", &start.to_string(), "-i", input, "-t", &(end - start).to_string(), "-vf"]);
            command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}", profile.width, profile.height, profile.width, profile.height, profile.fps));
            if let Err(error) = append_video_options(&mut command, &request) { return failed_response(Some(input.to_string()), error); }
            if request.no_audio.unwrap_or(false) { command.arg("-an"); } else {
                command.args(["-c:a", profile.audio_codec.as_str(), "-ar", &profile.sample_rate.to_string(), "-ac", &profile.channels.to_string()]);
            }
        }
        "watermark" => {
            let overlay = match request.overlay_path.as_deref() { Some(value) => value, None => return failed_response(Some(input.to_string()), "overlay_path is required".to_string()) };
            command.args(["-i", input, "-i", overlay, "-filter_complex"]);
            command.arg(format!("[1:v]format=rgba,colorchannelmixer=aa={}[wm];[0:v][wm]overlay=(W-w)/2:(H-h)/2", request.opacity.unwrap_or(0.25)));
            command.args(["-map", "0:v:0", "-map", "0:a:0?"]);
            if let Err(error) = append_video_options(&mut command, &request) { return failed_response(Some(input.to_string()), error); }
            let profile = match request.media.profile() { Ok(value) => value, Err(error) => return failed_response(Some(input.to_string()), error) };
            command.args(["-c:a", profile.audio_codec.as_str(), "-ar", &profile.sample_rate.to_string(), "-ac", &profile.channels.to_string()]);
        }
        "generate_proxy" => {
            command.args(["-i", input, "-vf", "scale=-2:720"]);
            if let Err(error) = append_video_options(&mut command, &request) { return failed_response(Some(input.to_string()), error); }
            let profile = match request.media.profile() { Ok(value) => value, Err(error) => return failed_response(Some(input.to_string()), error) };
            command.args(["-c:a", profile.audio_codec.as_str(), "-ar", &profile.sample_rate.to_string(), "-ac", &profile.channels.to_string()]);
        }
        "generate_storyboard" => {
            let interval = request.interval_frames.unwrap_or(10).max(1);
            let columns = request.columns.unwrap_or(5).max(1);
            let rows = request.rows.unwrap_or(5).max(1);
            command.args(["-i", input, "-vf"]);
            command.arg(format!("select=not(mod(n\\,{interval})),scale=320:-1,tile={columns}x{rows}"));
            command.args(["-frames:v", "1", "-an"]);
        }
        "remux_hls" => {
            command.args(["-i", input, "-c", "copy"]);
        },
        "remove_silence" => {
            command.args(["-i", input, "-af", "silenceremove=start_periods=1:start_threshold=-45dB:start_silence=0.25:stop_periods=-1:stop_threshold=-45dB:stop_silence=0.35", "-c:a", "libmp3lame", "-q:a", "2"]);
        },
        "trim" => {
            let max_duration = request.max_duration_sec.unwrap_or(0.0);
            if !max_duration.is_finite() || max_duration <= 0.0 { return failed_response(Some(input.to_string()), "max_duration_sec must be positive".to_string()); }
            command.args(["-i", input, "-t", &max_duration.to_string()]);
            let extension = Path::new(input).extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(extension.as_str(), "mp4" | "mov" | "mkv") {
                let profile = match request.media.profile() { Ok(value) => value, Err(error) => return failed_response(Some(input.to_string()), error) };
                command.args(["-map", "0:v:0?", "-map", "0:a:0?"]);
                if let Err(error) = append_video_options(&mut command, &request) { return failed_response(Some(input.to_string()), error); }
                command.args(["-c:a", profile.audio_codec.as_str(), "-b:a", profile.audio_bitrate.as_str(), "-ar", &profile.sample_rate.to_string(), "-ac", &profile.channels.to_string()]);
            } else if extension == "wav" { command.args(["-vn", "-c:a", "pcm_s16le"]); } else { command.args(["-vn", "-c:a", "libmp3lame", "-q:a", "2"]); }
        }
        _ => return failed_response(Some(input.to_string()), format!("unsupported transform: {operation}")),
    }
    let part = part_path(output);
    command.args(["-movflags", "+faststart", &part]);
    match command.output() {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response { ok: true, operation: operation.to_string(), source_path: Some(input.to_string()), items: Vec::new(), metadata: None, error: None },
            Err(error) => failed_response(Some(input.to_string()), error),
        },
        Ok(result) => { let _ = fs::remove_file(&part); failed_response(Some(input.to_string()), format!("{operation} failed: {}", String::from_utf8_lossy(&result.stderr).trim())) }
        Err(error) => { let _ = fs::remove_file(&part); failed_response(Some(input.to_string()), format!("{operation} failed to start: {error}")) }
    }
}
