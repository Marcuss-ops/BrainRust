use crate::probe::{ffprobe_path, probe_file};
use crate::protocol::{MediaMetadata, Request};

/// GPU compositing is deliberately narrower than GPU encoding. It is selected
/// only when NVENC is resolved, no pixel effect/transition needs a CPU-only
/// filter, and all inputs already share the requested geometry and frame rate.
/// This prevents a hidden GPU->CPU->GPU transfer and leaves resize/pad/fps work
/// on the safe CPU composite path until a CUDA-compatible graph exists.
pub(super) fn gpu_composite_eligibility(
    request: &Request,
    inputs: &[String],
    ffmpeg: &str,
) -> bool {
    if !request.no_transitions.unwrap_or(false)
        || !request.no_effects.unwrap_or(false)
        || request
            .transitions
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || request
            .effect_paths
            .as_ref()
            .is_some_and(|items| !items.is_empty())
    {
        return false;
    }
    let Ok(profile) = request.media.profile() else {
        return false;
    };
    let Ok(encoder) = request.media.encoder() else {
        return false;
    };
    if !encoder.codec.to_ascii_lowercase().ends_with("_nvenc") {
        return false;
    }
    let ffprobe = ffprobe_path(ffmpeg);
    let mut codec: Option<String> = None;
    for input in inputs {
        let Ok(metadata) = probe_file(&ffprobe, input) else {
            return false;
        };
        if !metadata.has_video
            || metadata.width != profile.width
            || metadata.height != profile.height
            || (metadata.fps - profile.fps as f64).abs() > 0.5
            || metadata.video_codec.is_none()
        {
            return false;
        }
        if let Some(reference) = &codec {
            if metadata.video_codec.as_deref() != Some(reference.as_str()) {
                return false;
            }
        } else {
            codec = metadata.video_codec;
        }
    }
    true
}

/// Resolves the stock fast path conservatively. A stream-copy render is only
/// safe when every input already satisfies the requested canonical contract;
/// otherwise the normal composite/encode path remains responsible for scale,
/// padding, frame-rate conversion, and codec normalization.
pub(super) fn copy_only_eligibility(
    request: &Request,
    inputs: &[String],
    ffmpeg: &str,
) -> Result<bool, String> {
    if !request.no_transitions.unwrap_or(false)
        || !request.no_effects.unwrap_or(false)
        || request
            .transitions
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || request
            .effect_paths
            .as_ref()
            .is_some_and(|items| !items.is_empty())
    {
        return Ok(false);
    }
    let profile = request.media.profile()?;
    let encoder = request.media.encoder()?;
    let ffprobe = ffprobe_path(ffmpeg);
    let mut first: Option<MediaMetadata> = None;
    for input in inputs {
        let metadata = probe_file(&ffprobe, input)?;
        if !metadata.has_video
            || metadata.width != profile.width
            || metadata.height != profile.height
            || (metadata.fps - profile.fps as f64).abs() > 0.5
            || !video_codec_matches(metadata.video_codec.as_deref(), &encoder.codec)
            || metadata.pixel_format.as_deref() != Some("yuv420p")
        {
            return Ok(false);
        }
        if request.keep_audio.unwrap_or(false)
            && (!metadata.has_audio
                || metadata.audio_codec.as_deref() != Some(profile.audio_codec.as_str())
                || metadata.sample_rate != Some(profile.sample_rate)
                || metadata.channels != Some(profile.channels))
        {
            return Ok(false);
        }
        if let Some(reference) = &first {
            if metadata.video_codec != reference.video_codec
                || metadata.pixel_format != reference.pixel_format
                || metadata.audio_codec != reference.audio_codec
                || metadata.sample_rate != reference.sample_rate
                || metadata.channels != reference.channels
            {
                return Ok(false);
            }
        } else {
            first = Some(metadata);
        }
    }
    Ok(true)
}

fn video_codec_matches(actual: Option<&str>, expected: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    match actual {
        Some(codec) if expected.ends_with("_nvenc") => codec == "h264",
        Some(codec) if expected == "libx264" || expected == "h264" => codec == "h264",
        Some(codec) => codec == expected,
        None => false,
    }
}
