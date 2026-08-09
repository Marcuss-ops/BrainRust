//! Resolved execution contracts supplied by the Go application.
//!
//! Go owns canonical media values and defaults. Rust owns only validation and
//! FFmpeg mechanics; it must never manufacture a profile when the payload is
//! incomplete.

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MediaConfig {
    pub codec: Option<String>,
    pub preset: Option<String>,
    pub crf: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub keyframe_interval: Option<u32>,
    pub audio_codec: Option<String>,
    pub audio_bitrate: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub duration_sec: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct VideoProfile {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub keyframe_interval: u32,
    pub audio_codec: String,
    pub audio_bitrate: String,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Clone, Debug)]
pub struct EncoderPolicy {
    pub codec: String,
    pub preset: String,
    pub crf: i32,
}

impl MediaConfig {
    pub fn profile(&self) -> Result<VideoProfile, String> {
        let width = required_positive(self.width, "width")?;
        let height = required_positive(self.height, "height")?;
        let fps = required_positive(self.fps, "fps")?;
        let keyframe_interval = required_positive(self.keyframe_interval, "keyframe_interval")?;
        let sample_rate = required_positive(self.sample_rate, "sample_rate")?;
        let channels = required_positive(self.channels, "channels")?;
        let audio_codec = required_string(self.audio_codec.as_deref(), "audio_codec")?;
        let audio_bitrate = required_string(self.audio_bitrate.as_deref(), "audio_bitrate")?;
        Ok(VideoProfile {
            width,
            height,
            fps,
            keyframe_interval,
            audio_codec,
            audio_bitrate,
            sample_rate,
            channels,
        })
    }

    pub fn encoder(&self) -> Result<EncoderPolicy, String> {
        let codec = required_string(self.codec.as_deref(), "codec")?;
        let preset = required_string(self.preset.as_deref(), "preset")?;
        let crf = self.crf.filter(|value| *value > 0).ok_or_else(|| {
            "ENCODER_POLICY_REQUIRED: quality is required for encoded media".to_string()
        })?;
        Ok(EncoderPolicy { codec, preset, crf })
    }
}

fn required_positive(value: Option<u32>, field: &str) -> Result<u32, String> {
    value.filter(|value| *value > 0).ok_or_else(|| {
        format!("PROFILE_REQUIRED: {field} is required for encoded media")
    })
}

fn required_string(value: Option<&str>, field: &str) -> Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("PROFILE_REQUIRED: {field} is required for encoded media"))
}
