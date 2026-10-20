//! Resolved execution contracts supplied by the Go application.
//!
//! Go owns canonical media values and defaults. Rust owns only validation and
//! FFmpeg mechanics; it must never manufacture a profile when the payload is
//! incomplete.

use crate::error::MuscleError;
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MediaConfig {
    pub codec: Option<String>,
    pub preset: Option<String>,
    pub crf: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps_num: Option<u32>,
    pub fps_den: Option<u32>,
    pub keyframe_interval: Option<u32>,
    pub audio_codec: Option<String>,
    pub audio_bitrate: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub duration_sec: Option<f64>,
    /// Parallel cut wave fanout (Go-owned capacity policy). Absent keeps the
    /// historical bound; 0 or above the ceiling fails closed (see
    /// [`MediaConfig::parallel_cuts`]).
    pub max_parallel_cuts: Option<u32>,
}

/// Historical cut fanout bound. Default when the Go policy omits
/// `max_parallel_cuts`, so the wire contract stays backward-compatible.
pub(crate) const DEFAULT_MAX_PARALLEL_CUTS: usize = 3;

/// Hard ceiling on the cut wave fanout: above this, one batch would spawn
/// more concurrent FFmpeg processes than any shared worker survives, so the
/// mistake fails closed instead of hammering the box.
pub(crate) const MAX_PARALLEL_CUTS_CEILING: usize = 64;

#[derive(Clone, Debug)]
pub struct VideoProfile {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub keyframe_interval: u32,
    pub audio_codec: String,
    pub audio_bitrate: String,
    pub sample_rate: u32,
    pub channels: u32,
}

impl VideoProfile {
    /// FPS as f64 — projection for logs/UI only.
    pub fn fps_float(&self) -> f64 {
        self.fps_num as f64 / self.fps_den as f64
    }
    /// Rounded integer FPS — for ffmpeg filter compatibility. Prefer fps_float().
    pub fn fps_round(&self) -> u32 {
        (self.fps_num as f64 / self.fps_den as f64).round() as u32
    }
    /// Exact rational equality — no float epsilon.
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
    pub fn profile(&self) -> Result<VideoProfile, MuscleError> {
        let width = required_positive(self.width, "width")?;
        let height = required_positive(self.height, "height")?;
        let fps_num = required_positive(self.fps_num, "fps_num")?;
        let fps_den = required_positive(self.fps_den, "fps_den")?;
        let keyframe_interval = required_positive(self.keyframe_interval, "keyframe_interval")?;
        let sample_rate = required_positive(self.sample_rate, "sample_rate")?;
        let channels = required_positive(self.channels, "channels")?;
        let audio_codec = required_string(self.audio_codec.as_deref(), "audio_codec")?;
        let audio_bitrate = required_string(self.audio_bitrate.as_deref(), "audio_bitrate")?;
        Ok(VideoProfile {
            width,
            height,
            fps_num,
            fps_den,
            keyframe_interval,
            audio_codec,
            audio_bitrate,
            sample_rate,
            channels,
        })
    }

    pub fn encoder(&self) -> Result<EncoderPolicy, MuscleError> {
        let codec = required_encoder_string(self.codec.as_deref(), "codec")?;
        let preset = required_encoder_string(self.preset.as_deref(), "preset")?;
        let crf = self.crf.filter(|value| *value > 0).ok_or_else(|| {
            "ENCODER_POLICY_REQUIRED: quality is required for encoded media".to_string()
        })?;
        Ok(EncoderPolicy { codec, preset, crf })
    }

    /// Parallel cut wave fanout. `None` keeps the historical
    /// [`DEFAULT_MAX_PARALLEL_CUTS`] bound; zero or a value above the
    /// [`MAX_PARALLEL_CUTS_CEILING`] fails closed (zero would panic
    /// `chunks`, and an absurd fanout is a thundering herd on shared
    /// workers).
    pub(crate) fn parallel_cuts(&self) -> Result<usize, MuscleError> {
        match self.max_parallel_cuts {
            None => Ok(DEFAULT_MAX_PARALLEL_CUTS),
            Some(value) if (value as usize) > MAX_PARALLEL_CUTS_CEILING => {
                Err(MuscleError::generic(format!(
                    "PROFILE_REQUIRED: max_parallel_cuts exceeds the {} wave ceiling",
                    MAX_PARALLEL_CUTS_CEILING
                )))
            }
            Some(0) => Err(MuscleError::generic(
                "PROFILE_REQUIRED: max_parallel_cuts must be positive when supplied",
            )),
            Some(value) => Ok(value as usize),
        }
    }
}

fn required_positive(value: Option<u32>, field: &str) -> Result<u32, MuscleError> {
    value.filter(|value| *value > 0).ok_or_else(|| {
        MuscleError::generic(format!(
            "PROFILE_REQUIRED: {field} is required for encoded media"
        ))
    })
}

fn required_string(value: Option<&str>, field: &str) -> Result<String, MuscleError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            MuscleError::generic(format!(
                "PROFILE_REQUIRED: {field} is required for encoded media"
            ))
        })
}

fn required_encoder_string(value: Option<&str>, field: &str) -> Result<String, MuscleError> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            MuscleError::generic(format!(
                "ENCODER_POLICY_REQUIRED: {field} is required for encoded media"
            ))
        })
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
            fps_num: Some(24),
            fps_den: Some(1),
            keyframe_interval: Some(48),
            audio_codec: Some("aac".to_string()),
            audio_bitrate: Some("128k".to_string()),
            sample_rate: Some(48000),
            channels: Some(2),
            duration_sec: None,
            max_parallel_cuts: None,
        }
    }

    #[test]
    fn complete_payload_materializes_without_rust_defaults() {
        let media = complete();
        let profile = media.profile().unwrap();
        assert_eq!(profile.width, 1920);
        assert_eq!(profile.height, 1080);
        assert_eq!(profile.fps_round(), 24);
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
        media.fps_num = None;
        media.fps_den = None;
        let error = media.profile().unwrap_err();
        assert!(error.contains("PROFILE_REQUIRED: fps_num"));
    }

    #[test]
    fn missing_parallel_cuts_policy_keeps_the_historical_bound() {
        assert_eq!(complete().parallel_cuts().unwrap(), 3);
    }

    #[test]
    fn parallel_cuts_policy_is_honored_when_supplied() {
        let mut media = complete();
        media.max_parallel_cuts = Some(8);
        assert_eq!(media.parallel_cuts().unwrap(), 8);
    }

    #[test]
    fn parallel_cuts_policy_fails_closed_on_zero_and_over_ceiling() {
        let mut media = complete();
        media.max_parallel_cuts = Some(0);
        let error = media.parallel_cuts().unwrap_err();
        assert!(error.contains("must be positive"), "zero: {error}");

        media.max_parallel_cuts = Some((super::MAX_PARALLEL_CUTS_CEILING as u32) + 1);
        let error = media.parallel_cuts().unwrap_err();
        assert!(error.contains("wave ceiling"), "over-ceiling: {error}");

        // The ceiling itself stays legal.
        media.max_parallel_cuts = Some(super::MAX_PARALLEL_CUTS_CEILING as u32);
        assert_eq!(
            media.parallel_cuts().unwrap(),
            super::MAX_PARALLEL_CUTS_CEILING
        );
    }
}
