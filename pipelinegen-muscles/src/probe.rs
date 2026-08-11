use crate::artifact::failed_response;
use crate::config;
use crate::config::EncoderPolicy;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use serde::Deserialize;
use std::path::Path;

pub(crate) fn execute(request: Request) -> Response {
    probe(request)
}

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    format: ProbeFormat,
    streams: Vec<ProbeStream>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    pix_fmt: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    profile: Option<String>,
    start_time: Option<String>,
}

pub(crate) fn ffprobe_path(ffmpeg: &str) -> String {
    let path = Path::new(ffmpeg);
    if path.file_name().and_then(|name| name.to_str()) == Some("ffmpeg") {
        return path
            .with_file_name("ffprobe")
            .to_string_lossy()
            .into_owned();
    }
    "ffprobe".to_string()
}

pub(crate) fn validate_output(
    ffprobe: &str,
    path: &str,
    no_audio: bool,
    expected_duration: f64,
    profile: &config::VideoProfile,
    encoder: &EncoderPolicy,
) -> Result<f64, String> {
    let mut command = FFmpegRunner::from_ffprobe_path(ffprobe).ffprobe();
    command.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        path,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("ffprobe failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("ffprobe exited with {}", output.status));
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("ffprobe parse failed: {error}"))?;
    let duration = probe
        .format
        .duration
        .as_deref()
        .ok_or_else(|| "ffprobe returned no duration".to_string())?
        .parse::<f64>()
        .map_err(|error| format!("invalid duration: {error}"))?;
    if duration <= 0.0 || (duration - expected_duration).abs() > 0.25 {
        return Err(format!(
            "duration violation: got {duration:.3}s, expected {expected_duration:.3}s"
        ));
    }
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| "no video stream".to_string())?;
    if video.width != Some(profile.width)
        || video.height != Some(profile.height)
        || !video_codec_matches(video.codec_name.as_deref(), &encoder.codec)
        || video.pix_fmt.as_deref() != Some("yuv420p")
        || parse_frame_rate(video.avg_frame_rate.as_deref().unwrap_or(""))
            .map_or(true, |fps| (fps - profile.fps as f64).abs() > 0.5)
    {
        return Err("canonical video profile violation".to_string());
    }
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    if no_audio {
        if audio.is_some() {
            return Err("audio stream present with no_audio=true".to_string());
        }
    } else {
        let audio = audio.ok_or_else(|| "audio stream is missing".to_string())?;
        let expected_sample_rate = profile.sample_rate.to_string();
        if audio.codec_name.as_deref() != Some(profile.audio_codec.as_str())
            || audio.sample_rate.as_deref() != Some(expected_sample_rate.as_str())
            || audio.channels != Some(profile.channels)
        {
            return Err("canonical audio profile violation".to_string());
        }
    }
    Ok(duration)
}

fn video_codec_matches(actual: Option<&str>, expected: &str) -> bool {
    let expected = expected.to_ascii_lowercase();
    match actual {
        Some(codec) if expected.contains("_nvenc") => codec == "h264",
        Some(codec) if expected == "libx264" || expected == "h264" => codec == "h264",
        Some(codec) => codec == expected,
        None => false,
    }
}

fn parse_frame_rate(value: &str) -> Option<f64> {
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator.parse::<f64>().ok()?;
        let denominator = denominator.parse::<f64>().ok()?;
        if denominator == 0.0 {
            return None;
        }
        return Some(numerator / denominator);
    }
    value.parse::<f64>().ok()
}

fn probe(request: Request) -> Response {
    let source = match request.source_path.as_deref() {
        Some(path) if !path.is_empty() && Path::new(path).is_file() => path,
        Some(path) => {
            return failed_response(
                Some(path.to_string()),
                format!("source file is not readable: {path}"),
            )
        }
        None => return failed_response(None, "source_path is required".to_string()),
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    match probe_file(&ffprobe_path(ffmpeg), source) {
        Ok(metadata) => Response {
            ok: true,
            operation: "probe".to_string(),
            source_path: Some(source.to_string()),
            items: Vec::new(),
            metadata: Some(metadata),
            error: None,
        },
        Err(error) => failed_response(Some(source.to_string()), error),
    }
}

pub(crate) fn probe_file(ffprobe: &str, path: &str) -> Result<MediaMetadata, String> {
    let mut command = FFmpegRunner::from_ffprobe_path(ffprobe).ffprobe();
    command.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        path,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("ffprobe failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("ffprobe exited with {}", output.status));
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("ffprobe parse failed: {error}"))?;
    let duration_sec = probe
        .format
        .duration
        .as_deref()
        .unwrap_or("0")
        .parse::<f64>()
        .unwrap_or(0.0);
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"));
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    Ok(MediaMetadata {
        duration_sec,
        bitrate: probe
            .format
            .bit_rate
            .as_deref()
            .and_then(|value| value.parse().ok()),
        width: video.and_then(|stream| stream.width).unwrap_or(0),
        height: video.and_then(|stream| stream.height).unwrap_or(0),
        fps: video
            .and_then(|stream| parse_frame_rate(stream.avg_frame_rate.as_deref().unwrap_or("")))
            .unwrap_or(0.0),
        video_codec: video.and_then(|stream| stream.codec_name.clone()),
        pixel_format: video.and_then(|stream| stream.pix_fmt.clone()),
        audio_codec: audio.and_then(|stream| stream.codec_name.clone()),
        audio_profile: audio.and_then(|stream| stream.profile.clone()),
        sample_rate: audio.and_then(|stream| stream.sample_rate.as_deref()?.parse().ok()),
        channels: audio.and_then(|stream| stream.channels),
        start_pts: audio
            .and_then(|stream| stream.start_time.as_deref()?.parse::<f64>().ok())
            .map(|value| value.round() as i64),
        has_video: video.is_some(),
        has_audio: audio.is_some(),
    })
}
