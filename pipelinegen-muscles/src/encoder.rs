use crate::config::{EncoderPolicy, VideoProfile};
use crate::process::ProcessCommand;
use crate::protocol::Request;

/// The only builder allowed to emit video encoder arguments.
///
/// The Go application owns the resolved policy values. Rust owns only the
/// FFmpeg mechanics: codec-specific quality flags, NVENC preset translation,
/// pixel format, frame synchronization, and GOP construction.
pub fn append_video_args(
    command: &mut ProcessCommand,
    policy: &EncoderPolicy,
    profile: &VideoProfile,
    keyframe_interval: Option<u32>,
) -> Result<(), String> {
    append_video_args_with_pixel_format(command, policy, profile, keyframe_interval, "yuv420p")
}

/// Appends the encoder contract for a hardware frame. `cuda` is an FFmpeg
/// hardware pixel format, not a codec pixel format: it tells FFmpeg to keep
/// the decoded CUDA frame on the device until NVENC consumes it.
pub fn append_video_args_cuda(
    command: &mut ProcessCommand,
    policy: &EncoderPolicy,
    profile: &VideoProfile,
    keyframe_interval: Option<u32>,
) -> Result<(), String> {
    append_video_args_with_pixel_format(command, policy, profile, keyframe_interval, "cuda")
}

fn append_video_args_with_pixel_format(
    command: &mut ProcessCommand,
    policy: &EncoderPolicy,
    profile: &VideoProfile,
    keyframe_interval: Option<u32>,
    pixel_format: &str,
) -> Result<(), String> {
    for argument in build_video_args(policy, profile, keyframe_interval, pixel_format)? {
        command.arg(argument);
    }
    Ok(())
}

/// Builds the complete video encoder argument vector for every encoded video
/// operation. Keep all codec/preset/quality/rate-control/pixel-format/GOP
/// decisions in this function; capabilities must not emit these flags directly.
fn build_video_args(
    policy: &EncoderPolicy,
    profile: &VideoProfile,
    keyframe_interval: Option<u32>,
    pixel_format: &str,
) -> Result<Vec<String>, String> {
    let requested_codec = policy.codec.trim().to_ascii_lowercase();
    if requested_codec.is_empty() {
        return Err("ENCODER_POLICY_REQUIRED: codec is required for encoded media".to_string());
    }
    if policy.preset.trim().is_empty() {
        return Err("ENCODER_POLICY_REQUIRED: preset is required for encoded media".to_string());
    }
    if policy.crf <= 0 {
        return Err("ENCODER_POLICY_REQUIRED: quality is required for encoded media".to_string());
    }

    let codec = canonical_codec(&requested_codec)?;
    let nvenc = is_nvenc_codec(&codec);
    let preset = normalize_preset(&codec, &policy.preset)?;
    let gop = keyframe_interval
        .filter(|value| *value > 0)
        .unwrap_or(profile.keyframe_interval);
    if gop == 0 {
        return Err("ENCODER_POLICY_REQUIRED: GOP/keyframe interval must be positive".to_string());
    }

    let mut args = vec![
        // Deterministic container: strip all source metadata (creation_time,
        // encoder/comment tags, chapters) so a re-encode of identical video
        // bytes always produces an identical MP4. Without these, FFmpeg copies
        // per-download metadata (e.g. a rotating signed-URL creation_time) into
        // the output, so the same clip re-downloaded yields a different hash.
        "-map_metadata".to_string(),
        "-1".to_string(),
        "-map_chapters".to_string(),
        "-1".to_string(),
        "-c:v".to_string(),
        codec,
        "-preset".to_string(),
        preset,
        "-pix_fmt".to_string(),
        pixel_format.to_string(),
        "-vsync".to_string(),
        "cfr".to_string(),
    ];
    if nvenc {
        // NVENC quality is controlled by CQ under VBR. CRF is a libx264
        // concept and must never be emitted for an NVENC codec.
        args.extend([
            "-rc".to_string(),
            "vbr".to_string(),
            "-cq".to_string(),
            policy.crf.to_string(),
        ]);
    } else {
        // Software H.264 quality is controlled by CRF. Do not emit NVENC
        // rate-control flags for libx264 or other software encoders.
        args.extend(["-crf".to_string(), policy.crf.to_string()]);
    }
    args.extend(["-g".to_string(), gop.to_string()]);
    Ok(args)
}

fn canonical_codec(codec: &str) -> Result<String, String> {
    match codec {
        "nvenc" => Ok("h264_nvenc".to_string()),
        value if value.ends_with("_nvenc") => Ok(value.to_string()),
        value => Ok(value.to_string()),
    }
}

fn is_nvenc_codec(codec: &str) -> bool {
    codec.ends_with("_nvenc")
}

fn normalize_preset(codec: &str, preset: &str) -> Result<String, String> {
    let preset = preset.trim().to_ascii_lowercase();
    if !is_nvenc_codec(codec) {
        return Ok(preset);
    }

    let normalized = match preset.as_str() {
        "p1" | "p2" | "p3" | "p4" | "p5" | "p6" | "p7" => preset,
        // Go's canonical software-oriented presets are translated to their
        // NVENC equivalents at this technical execution boundary.
        "ultrafast" | "superfast" | "veryfast" | "faster" | "fast" => "p1".to_string(),
        "medium" => "p4".to_string(),
        "slow" | "slower" | "veryslow" => "p7".to_string(),
        _ => {
            return Err(format!(
                "INVALID_ENCODER_PRESET: unsupported NVENC preset {preset}"
            ))
        }
    };
    Ok(normalized)
}

/// Resolves the Go-owned request policy and appends the complete encoder
/// contract. Encoding capabilities must use this entry point rather than
/// constructing codec, quality, or GOP arguments themselves.
pub(crate) fn append_video_options(
    command: &mut ProcessCommand,
    request: &Request,
) -> Result<(), String> {
    let encoder = request.media.encoder()?;
    let profile = request.media.profile()?;
    append_video_args(command, &encoder, &profile, None)?;
    if let Some(duration) = request.media.duration_sec.filter(|value| *value > 0.0) {
        command.args(["-t", &duration.to_string()]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_video_args, canonical_codec, normalize_preset};
    use crate::config::{EncoderPolicy, VideoProfile};

    fn policy(codec: &str, preset: &str, quality: i32) -> EncoderPolicy {
        EncoderPolicy {
            codec: codec.to_string(),
            preset: preset.to_string(),
            crf: quality,
        }
    }

    fn profile() -> VideoProfile {
        VideoProfile {
            width: 1920,
            height: 1080,
            fps_num: 24,
            fps_den: 1,
            keyframe_interval: 48,
            audio_codec: "aac".to_string(),
            audio_bitrate: "128k".to_string(),
            sample_rate: 48000,
            channels: 2,
        }
    }

    fn pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == flag && window[1] == value)
    }

    #[test]
    fn libx264_uses_crf_and_never_nvenc_rate_control() {
        let args = build_video_args(
            &policy("libx264", "veryfast", 23),
            &profile(),
            None,
            "yuv420p",
        )
        .unwrap();

        assert!(pair(&args, "-c:v", "libx264"));
        assert!(pair(&args, "-preset", "veryfast"));
        assert!(pair(&args, "-crf", "23"));
        assert!(pair(&args, "-pix_fmt", "yuv420p"));
        assert!(pair(&args, "-vsync", "cfr"));
        assert!(pair(&args, "-g", "48"));
        assert!(pair(&args, "-map_metadata", "-1"));
        assert!(pair(&args, "-map_chapters", "-1"));
        assert!(!args.iter().any(|arg| arg == "-rc" || arg == "-cq"));
    }

    #[test]
    fn nvenc_uses_cq_vbr_and_never_libx264_crf() {
        let args = build_video_args(
            &policy("h264_nvenc", "p1", 23),
            &profile(),
            Some(60),
            "yuv420p",
        )
        .unwrap();

        assert!(pair(&args, "-c:v", "h264_nvenc"));
        assert!(pair(&args, "-preset", "p1"));
        assert!(pair(&args, "-rc", "vbr"));
        assert!(pair(&args, "-cq", "23"));
        assert!(pair(&args, "-g", "60"));
        assert!(!args.iter().any(|arg| arg == "-crf"));
    }

    #[test]
    fn cuda_output_keeps_hardware_pixel_format() {
        let args =
            build_video_args(&policy("h264_nvenc", "p1", 23), &profile(), None, "cuda").unwrap();
        assert!(pair(&args, "-pix_fmt", "cuda"));
        assert!(!pair(&args, "-pix_fmt", "yuv420p"));
    }

    #[test]
    fn nvenc_alias_is_canonicalized_to_h264_nvenc() {
        let args =
            build_video_args(&policy("nvenc", "p1", 23), &profile(), None, "yuv420p").unwrap();

        assert!(pair(&args, "-c:v", "h264_nvenc"));
        assert!(!pair(&args, "-c:v", "nvenc"));
        assert!(pair(&args, "-rc", "vbr"));
        assert!(pair(&args, "-cq", "23"));
    }

    #[test]
    fn nvenc_translates_software_preset_without_changing_quality_semantics() {
        let args = build_video_args(
            &policy("H264_NVENC", "veryfast", 27),
            &profile(),
            None,
            "yuv420p",
        )
        .unwrap();

        assert!(pair(&args, "-c:v", "h264_nvenc"));
        assert!(pair(&args, "-preset", "p1"));
        assert!(pair(&args, "-cq", "27"));
        assert!(!args.iter().any(|arg| arg == "veryfast" || arg == "-crf"));
    }

    #[test]
    fn canonical_codec_preserves_explicit_codec_names() {
        assert_eq!(canonical_codec("libx264").unwrap(), "libx264");
        assert_eq!(canonical_codec("h264_nvenc").unwrap(), "h264_nvenc");
    }

    #[test]
    fn unsupported_nvenc_preset_fails_closed() {
        let error = normalize_preset("h264_nvenc", "custom").unwrap_err();
        assert!(error.contains("INVALID_ENCODER_PRESET"));
    }

    #[test]
    fn incomplete_policy_fails_before_building_arguments() {
        for policy in [
            policy("", "p1", 23),
            policy("libx264", "", 23),
            policy("libx264", "veryfast", 0),
        ] {
            let error = build_video_args(&policy, &profile(), None, "yuv420p").unwrap_err();
            assert!(error.contains("ENCODER_POLICY_REQUIRED"));
        }
    }

    #[test]
    fn zero_override_uses_profile_gop() {
        let args = build_video_args(
            &policy("libx264", "veryfast", 23),
            &profile(),
            Some(0),
            "yuv420p",
        )
        .unwrap();
        assert!(pair(&args, "-g", "48"));
    }

    #[test]
    fn profile_values_are_not_replaced_by_encoder_builder_defaults() {
        let mut custom = profile();
        custom.keyframe_interval = 72;
        let args =
            build_video_args(&policy("libx264", "veryfast", 23), &custom, None, "yuv420p").unwrap();
        assert!(pair(&args, "-g", "72"));
    }
}
