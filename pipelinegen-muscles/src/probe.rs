use crate::artifact::failed_response;

use crate::config;
use crate::config::EncoderPolicy;
use crate::error::MuscleError;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

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
    format_name: Option<String>,
    nb_streams: Option<u32>,
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
    level: Option<i32>,
    refs: Option<u32>,
    time_base: Option<String>,
    sample_aspect_ratio: Option<String>,
    color_range: Option<String>,
    color_space: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    field_order: Option<String>,
    gop_size: Option<u32>,
    has_b_frames: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_boolish")]
    closed_captions: Option<bool>,
    bit_rate: Option<String>,
    extradata: Option<String>,
    channel_layout: Option<String>,
    start_time: Option<String>,
}

// ffprobe emits some boolean-ish stream flags as numeric 0/1 values on
// common builds, while other builds emit JSON booleans. Accept both forms so
// the media contract is stable across ffprobe versions.
fn deserialize_boolish<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Bool(value) => Some(value),
        serde_json::Value::Number(value) => value.as_i64().map(|value| value != 0),
        serde_json::Value::String(value) => match value.as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        serde_json::Value::Null => None,
        _ => None,
    })
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
) -> Result<f64, MuscleError> {
    let mut command = FFmpegRunner::from_ffprobe_path(ffprobe).ffprobe();
    command.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        "-show_data",
        path,
    ]);
    let output = command
        .output()
        .map_err(|error| MuscleError::generic(format!("ffprobe failed to start: {error}")))?;
    if !output.status.success() {
        return Err(MuscleError::generic(format!(
            "ffprobe exited with {}",
            output.status
        )));
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| MuscleError::generic(format!("ffprobe parse failed: {error}")))?;
    let duration = probe
        .format
        .duration
        .as_deref()
        .ok_or_else(|| MuscleError::generic("ffprobe returned no duration"))?
        .parse::<f64>()
        .map_err(|error| MuscleError::generic(format!("invalid duration: {error}")))?;
    if duration <= 0.0 || (duration - expected_duration).abs() > 0.25 {
        return Err(MuscleError::generic(format!(
            "duration violation: got {duration:.3}s, expected {expected_duration:.3}s"
        )));
    }
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| MuscleError::generic("no video stream"))?;
    if video.width != Some(profile.width)
        || video.height != Some(profile.height)
        || !video_codec_matches(video.codec_name.as_deref(), &encoder.codec)
        || video.pix_fmt.as_deref() != Some("yuv420p")
        || parse_frame_rate(video.avg_frame_rate.as_deref().unwrap_or(""))
            .is_none_or(|fps| (fps - profile.fps_float()).abs() > 0.5)
    {
        return Err(MuscleError::generic("canonical video profile violation"));
    }
    // The assembly contract is a bitstream identity, not a codec name. Every
    // fact below changes either the SPS/PPS or the timestamps that
    // `concat -c copy` reuses from the first segment, while the nominal
    // geometry can stay identical. A cached cut written before this gate is
    // rejected here and re-encoded by the caller, which is the cache
    // invalidation the encoder-profile change requires.
    if video.profile.as_deref() != Some("High")
        || video.level != Some(40)
        || video.refs != Some(1)
        || video.has_b_frames.unwrap_or(1) != 0
        || video.time_base.as_deref() != Some("1/90000")
        || video.sample_aspect_ratio.as_deref() != Some("1:1")
        || video.color_range.as_deref() != Some("tv")
        || video.color_space.as_deref() != Some("bt709")
        || video.color_transfer.as_deref() != Some("bt709")
        || video.color_primaries.as_deref() != Some("bt709")
    {
        return Err(MuscleError::generic(
            "canonical H.264 bitstream violation (High 4.0, refs 1, no B-frames, 1/90000, 1:1 SAR, bt709/tv required)",
        ));
    }
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    if no_audio {
        if audio.is_some() {
            return Err(MuscleError::generic(
                "audio stream present with no_audio=true",
            ));
        }
    } else {
        let audio = audio.ok_or_else(|| MuscleError::generic("audio stream is missing"))?;
        let expected_sample_rate = profile.sample_rate.to_string();
        if audio.codec_name.as_deref() != Some(profile.audio_codec.as_str())
            || audio.sample_rate.as_deref() != Some(expected_sample_rate.as_str())
            || audio.channels != Some(profile.channels)
        {
            return Err(MuscleError::generic("canonical audio profile violation"));
        }
    }
    Ok(duration)
}

pub(crate) fn video_codec_matches(actual: Option<&str>, expected: &str) -> bool {
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

// parse_frame_rate_rational parses the exact num/den rational ffprobe reports
// in avg_frame_rate (e.g. "30/1", "30000/1001"). It is kept lossless so the
// overlay media contract can compare frame rates exactly instead of through a
// float round-trip. (0, 0) means "not reported / indeterminate".
fn parse_u32_pair(value: &str) -> Option<(u32, u32)> {
    let (num, den) = value.split_once('/')?;
    Some((num.parse().ok()?, den.parse().ok()?))
}

fn format_level(level: i32) -> String {
    if level >= 10 {
        format!("{}.{}", level / 10, level % 10)
    } else {
        level.to_string()
    }
}

// hash_ffprobe_extradata hashes the DECODED avcC bytes, not ffprobe's ASCII
// hex dump. ffprobe prints the dump as offset-prefixed lines with a printable
// column; hashing the text would produce a different identity from the Go
// producer and the assembler would reject every correct input.
fn hash_ffprobe_extradata(dump: &str) -> Option<String> {
    let mut raw = Vec::new();
    for line in dump.lines() {
        let Some((_, payload)) = line.split_once(':') else {
            continue;
        };
        let hex_part = payload.trim_start().split("  ").next().unwrap_or("");
        for word in hex_part.split_whitespace() {
            if word.len() % 2 != 0 {
                return None;
            }
            let mut index = 0;
            while index < word.len() {
                let byte = u8::from_str_radix(&word[index..index + 2], 16).ok()?;
                raw.push(byte);
                index += 2;
            }
        }
    }
    if raw.is_empty() {
        return None;
    }
    hash_hex(&raw).ok()
}

fn hash_hex(bytes: &[u8]) -> Result<String, MuscleError> {
    let mut command = Command::new("sha256sum");
    let output = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .ok_or_else(|| std::io::Error::other("sha256 stdin unavailable"))?
                .write_all(bytes)?;
            child.wait_with_output()
        })
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(MuscleError::generic("sha256sum failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string())
}

fn parse_frame_rate_rational(value: &str) -> (u32, u32) {
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator.parse::<u32>().unwrap_or(0);
        let denominator = denominator.parse::<u32>().unwrap_or(0);
        return (numerator, denominator);
    }
    (0, 0)
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

pub(crate) fn probe_file(ffprobe: &str, path: &str) -> Result<MediaMetadata, MuscleError> {
    let mut command = FFmpegRunner::from_ffprobe_path(ffprobe).ffprobe();
    command.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        "-show_data",
        path,
    ]);
    let output = command
        .output()
        .map_err(|error| MuscleError::generic(format!("ffprobe failed to start: {error}")))?;
    if !output.status.success() {
        return Err(MuscleError::generic(format!(
            "ffprobe exited with {}",
            output.status
        )));
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| MuscleError::generic(format!("ffprobe parse failed: {error}")))?;
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
    let stream_count = probe
        .format
        .nb_streams
        .unwrap_or(probe.streams.len() as u32);
    let video_stream_count = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("video"))
        .count() as u32;
    let audio_stream_count = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
        .count() as u32;
    let (fps_num, fps_den) = video
        .and_then(|stream| stream.avg_frame_rate.as_deref())
        .map(parse_frame_rate_rational)
        .unwrap_or((0, 0));
    Ok(MediaMetadata {
        audio_edits: Vec::new(),
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
        video_profile: video.and_then(|stream| stream.profile.clone()),
        video_level: video.and_then(|stream| stream.level.map(format_level)),
        pixel_format: video.and_then(|stream| stream.pix_fmt.clone()),
        video_time_base_num: video.and_then(|stream| {
            stream
                .time_base
                .as_deref()
                .and_then(parse_u32_pair)
                .map(|pair| pair.0)
        }),
        video_time_base_den: video.and_then(|stream| {
            stream
                .time_base
                .as_deref()
                .and_then(parse_u32_pair)
                .map(|pair| pair.1)
        }),
        sar_num: video.and_then(|stream| {
            stream
                .sample_aspect_ratio
                .as_deref()
                .and_then(parse_u32_pair)
                .map(|pair| pair.0)
        }),
        sar_den: video.and_then(|stream| {
            stream
                .sample_aspect_ratio
                .as_deref()
                .and_then(parse_u32_pair)
                .map(|pair| pair.1)
        }),
        color_range: video.and_then(|stream| stream.color_range.clone()),
        color_space: video.and_then(|stream| stream.color_space.clone()),
        color_transfer: video.and_then(|stream| stream.color_transfer.clone()),
        color_primaries: video.and_then(|stream| stream.color_primaries.clone()),
        field_order: video.and_then(|stream| stream.field_order.clone()),
        keyframe_interval: video.and_then(|stream| stream.gop_size),
        b_frames: video.and_then(|stream| stream.has_b_frames),
        closed_gop: video.and_then(|stream| stream.closed_captions),
        video_extradata_sha256: video
            .and_then(|stream| stream.extradata.as_deref())
            .and_then(hash_ffprobe_extradata),
        format_name: probe.format.format_name.clone(),
        stream_count,
        video_stream_count,
        audio_stream_count,
        fps_num,
        fps_den,
        audio_codec: audio.and_then(|stream| stream.codec_name.clone()),
        audio_profile: audio.and_then(|stream| stream.profile.clone()),
        audio_time_base_num: audio.and_then(|stream| {
            stream
                .time_base
                .as_deref()
                .and_then(parse_u32_pair)
                .map(|pair| pair.0)
        }),
        audio_time_base_den: audio.and_then(|stream| {
            stream
                .time_base
                .as_deref()
                .and_then(parse_u32_pair)
                .map(|pair| pair.1)
        }),
        sample_rate: audio.and_then(|stream| stream.sample_rate.as_deref()?.parse().ok()),
        channels: audio.and_then(|stream| stream.channels),
        channel_layout: audio.and_then(|stream| stream.channel_layout.clone()),
        audio_bitrate: audio.and_then(|stream| stream.bit_rate.as_deref()?.parse().ok()),
        audio_extradata_sha256: audio
            .and_then(|stream| stream.extradata.as_deref())
            .and_then(|value| hash_hex(value.as_bytes()).ok()),
        start_pts: audio
            .and_then(|stream| stream.start_time.as_deref()?.parse::<f64>().ok())
            .map(|value| value.round() as i64),
        has_video: video.is_some(),
        has_audio: audio.is_some(),
        mix_ms: None,
        aac_encode_ms: None,
        probe_ms: None,
        hash_ms: None,
        ffmpeg_ms: None,
        startup_ms: None,
        publish_ms: None,
        op_ms: None,
        final_audio_sha256: None,
        audio_copy_eligible: None,
        audio_encode_passes: None,
        subtitle_raster_cpu: None,
        decode_ms: None,
        filter_graph_ms: None,
        subtitle_raster_ms: None,
        watermark_raster_ms: None,
        frame_conversion_ms: None,
        encode_ms: None,
        audio_mux_ms: None,
    })
}

#[cfg(test)]
mod extradata_tests {
    use super::hash_ffprobe_extradata;

    // ffprobe prints avcC as an offset-prefixed hex dump with a printable
    // column. The identity must be SHA-256 of the decoded bytes, exactly as
    // the Go producer and the RenderingGen profiler compute it.
    #[test]
    fn hashes_decoded_avcc_bytes() {
        let dump = "\n00000000: 0164 0028  .d.(\n";
        assert_eq!(
            hash_ffprobe_extradata(dump).as_deref(),
            Some("fb65b97698978545528e2d21234bc971d759ed4602b0af9b14aa89492dd2c096")
        );
    }

    #[test]
    fn rejects_missing_dump() {
        assert!(hash_ffprobe_extradata("").is_none());
    }
}
