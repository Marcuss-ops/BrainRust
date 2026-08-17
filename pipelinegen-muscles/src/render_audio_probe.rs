use crate::process::FFmpegRunner;
use crate::protocol::MediaMetadata;

#[derive(serde::Deserialize)]
struct Probe {
    format: Format,
    streams: Vec<Stream>,
}

#[derive(serde::Deserialize)]
struct Format {
    duration: Option<String>,
    bit_rate: Option<String>,
}

#[derive(serde::Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    start_time: Option<String>,
}

pub(super) fn probe_source_duration(ffmpeg: &str, path: &str) -> Result<f64, String> {
    let ffprobe = crate::probe::ffprobe_path(ffmpeg);
    let mut command = FFmpegRunner::from_ffprobe_path(&ffprobe).ffprobe();
    command.args([
        "-v", "error", "-show_entries", "format=duration", "-of",
        "default=noprint_wrappers=1:nokey=1", path,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("ffprobe source failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("source asset is corrupt or unreadable: {path}"));
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("invalid source duration for {path}: {error}"))?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err(format!("source asset has no usable duration: {path}"));
    }
    Ok(duration)
}

pub(super) fn probe_audio(
    ffmpeg: &str, path: &str, expected_duration: f64,
) -> Result<MediaMetadata, String> {
    let ffprobe = crate::probe::ffprobe_path(ffmpeg);
    let mut command = FFmpegRunner::from_ffprobe_path(&ffprobe).ffprobe();
    command.args([
        "-v", "quiet", "-print_format", "json", "-show_format", "-show_streams", path,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("ffprobe failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("ffprobe exited with {}", output.status));
    }
    let probe: Probe = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("ffprobe parse failed: {error}"))?;
    let duration = probe
        .format
        .duration
        .as_deref()
        .ok_or("ffprobe returned no duration")?
        .parse::<f64>()
        .map_err(|error| format!("invalid duration: {error}"))?;
    let audio = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
        .collect::<Vec<_>>();
    if audio.len() != 1 {
        return Err(format!("expected exactly one audio stream, found {}", audio.len()));
    }
    let stream = audio[0];
    if stream.codec_name.as_deref() != Some("aac")
        || stream.profile.as_deref().map_or(true, |profile| !profile.eq_ignore_ascii_case("LC"))
        || stream.sample_rate.as_deref() != Some("48000")
        || stream.channels != Some(2)
    {
        return Err("rendered audio violates canonical AAC-LC 48kHz stereo contract".into());
    }
    // The master is apad+atrim'd to the canonical duration in the mix pass, so
    // only the AAC encoder padding may remain. Fail closed beyond ±40ms — the
    // same window the Go adapter certifies (ValidateFinalAudioReference).
    if (duration - expected_duration).abs() > 0.040 {
        return Err(format!("rendered audio duration {duration:.3}s differs from expected {expected_duration:.3}s"));
    }
    Ok(MediaMetadata {
        duration_sec: duration,
        bitrate: probe.format.bit_rate.as_deref().and_then(|value| value.parse().ok()),
        width: 0,
        height: 0,
        fps: 0.0,
        video_codec: None,
        pixel_format: None,
        format_name: None,
        stream_count: probe.streams.len() as u32,
        video_stream_count: 0,
        audio_stream_count: audio.len() as u32,
        fps_num: 0,
        fps_den: 0,
        audio_codec: stream.codec_name.clone(),
        audio_profile: stream.profile.clone(),
        sample_rate: stream.sample_rate.as_deref().and_then(|value| value.parse().ok()),
        channels: stream.channels,
        start_pts: stream.start_time.as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .map(|value| value.round() as i64),
        has_video: false,
        has_audio: true,
        mix_ms: None,
        aac_encode_ms: None,
        probe_ms: None,
        hash_ms: None,
        ffmpeg_ms: None,
        final_audio_sha256: None,
    })
}
