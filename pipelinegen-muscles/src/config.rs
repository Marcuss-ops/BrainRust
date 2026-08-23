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
    pub fps_num: Option<u32>,
    pub fps_den: Option<u32>,
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
    pub fps: u32, // deprecated: use fps_num/fps_den
    pub fps_num: u32,
    pub fps_den: u32,
    pub keyframe_interval: u32,
    pub audio_codec: String,
    pub audio_bitrate: String,
    pub sample_rate: u32,
    pub channels: u32,
}

impl VideoProfile {
    pub fn fps_float(&self) -> f64 {
        self.fps_num as f64 / self.fps_den as f64
    }
    pub fn fps_equal(&self, other: &VideoProfile) -> bool {
        self.fps_num * other.fps_den == other.fps_num * self.fps_den
    }
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
        let (fps_num, fps_den) = match (self.fps_num, self.fps_den) {
            (Some(n), Some(d)) if n > 0 && d > 0 => (n, d),
            _ => {
                let fps = required_positive(self.fps, "fps")?;
                (fps, 1)
            }
        };
        let fps = (fps_num as f64 / fps_den as f64).round() as u32;
        let keyframe_interval = required_positive(self.keyframe_interval, "keyframe_interval")?;
        let sample_rate = required_positive(self.sample_rate, "sample_rate")?;
        let channels = required_positive(self.channels, "channels")?;
        let audio_codec = required_string(self.audio_codec.as_deref(), "audio_codec")?;
        let audio_bitrate = required_string(self.audio_bitrate.as_deref(), "audio_bitrate")?;
        Ok(VideoProfile {
            width,
            height,
            fps,
            fps_num,
            fps_den,
            keyframe_interval,
            audio_codec,
            audio_bitrate,
            sample_rate,
            channels,
        })
    }

    pub fn encoder(&self) -> Result<EncoderPolicy, String> {
        let codec = required_encoder_string(self.codec.as_deref(), "codec")?;
        let preset = required_encoder_string(self.preset.as_deref(), "preset")?;
        let crf = self.crf.filter(|value| *value > 0).ok_or_else(|| {
            "ENCODER_POLICY_REQUIRED: quality is required for encoded media".to_string()
        })?;
        Ok(EncoderPolicy { codec, preset, crf })
    }
}

fn required_positive(value: Option<u32>, field: &str) -> Result<u32, String> {
    value
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("PROFILE_REQUIRED: {field} is required for encoded media"))
}

fn required_string(value: Option<&str>, field: &str) -> Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("PROFILE_REQUIRED: {field} is required for encoded media"))
}

fn required_encoder_string(value: Option<&str>, field: &str) -> Result<String, String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("ENCODER_POLICY_REQUIRED: {field} is required for encoded media"))
}

#[cfg(test)]
mod tests {
    use super::MediaConfig;

    fn complete() -> MediaConfig {
        MediaConfig {
            codec: Some("h264_nvenc".to_string()),
            preset: Some("p1".to_string()),
            crf: Some(23),
            width: Some(1920),
            height: Some(1080),
            fps: Some(24),
            fps_num: None,
            fps_den: None,
            keyframe_interval: Some(48),
            audio_codec: Some("aac".to_string()),
            audio_bitrate: Some("128k".to_string()),
            sample_rate: Some(48000),
            channels: Some(2),
            duration_sec: None,
        }
    }

    #[test]
    fn complete_payload_materializes_without_rust_defaults() {
        let media = complete();
        let profile = media.profile().unwrap();
        assert_eq!(profile.width, 1920);
        assert_eq!(profile.height, 1080);
        assert_eq!(profile.fps, 24);
        assert_eq!(profile.fps_num, 24);
        assert_eq!(profile.fps_den, 1);
        assert_eq!(profile.keyframe_interval, 48);
        assert_eq!(profile.audio_codec, "aac");
        assert_eq!(profile.audio_bitrate, "128k");
        assert_eq!(profile.sample_rate, 48000);
        assert_eq!(profile.channels, 2);
        assert_eq!(media.encoder().unwrap().codec, "h264_nvenc");
    }

    #[test]
    fn incomplete_profile_fails_closed_instead_of_defaulting() {
        let mut media = complete();
        media.fps = None;
        let error = media.profile().unwrap_err();
        assert!(error.contains("PROFILE_REQUIRED: fps"));
    }
}
