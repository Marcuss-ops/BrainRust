use crate::config::{EncoderPolicy, VideoProfile};
use std::process::Command;

/// The only builder allowed to emit video encoder arguments.
pub fn append_video_args(
    command: &mut Command,
    policy: &EncoderPolicy,
    profile: VideoProfile,
    keyframe_interval: Option<u32>,
) {
    command.args([
        "-c:v",
        policy.codec.as_str(),
        "-pix_fmt",
        "yuv420p",
        "-vsync",
        "cfr",
    ]);
    if !policy.preset.is_empty() {
        command.args(["-preset", policy.preset.as_str()]);
    }
    if policy.codec.contains("nvenc") {
        command.args(["-rc", "vbr", "-cq", &policy.crf.to_string()]);
    } else if policy.crf > 0 {
        command.args(["-crf", &policy.crf.to_string()]);
    }
    command.args([
        "-g",
        &keyframe_interval
            .unwrap_or(profile.keyframe_interval)
            .to_string(),
    ]);
}
