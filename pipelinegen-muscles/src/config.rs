//! Shared execution policy for every Rust media capability.
//!
//! Go remains the application configuration owner. This module is the single
//! Rust representation and defaulting point for the technical media contract;
//! capabilities must consume it instead of embedding profile/encoder values.

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MediaConfig {
    pub codec: Option<String>,
    pub preset: Option<String>,
    pub crf: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub duration_sec: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub struct VideoProfile {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub keyframe_interval: u32,
    pub audio_codec: &'static str,
    pub audio_bitrate: &'static str,
}

impl Default for VideoProfile {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 24,
            keyframe_interval: 48,
            audio_codec: "aac",
            audio_bitrate: "128k",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncoderPolicy {
    pub codec: String,
    pub preset: String,
    pub crf: i32,
}

impl MediaConfig {
    pub fn profile(&self) -> VideoProfile {
        let defaults = VideoProfile::default();
        VideoProfile {
            width: self.width.unwrap_or(defaults.width),
            height: self.height.unwrap_or(defaults.height),
            fps: self.fps.unwrap_or(defaults.fps),
            ..defaults
        }
    }

    pub fn encoder(&self) -> EncoderPolicy {
        EncoderPolicy {
            codec: self.codec.clone().unwrap_or_else(|| "libx264".to_string()),
            preset: self
                .preset
                .clone()
                .unwrap_or_else(|| "veryfast".to_string()),
            crf: self.crf.filter(|value| *value > 0).unwrap_or(23),
        }
    }
}
