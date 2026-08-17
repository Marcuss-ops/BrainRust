use serde::{Deserialize, Serialize};

use crate::config;

pub const PROTOCOL_VERSION: &str = "mediaexec.v1";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Health,
    Probe,
    CutBatch,
    Normalize,
    CutCopy,
    CutAndNormalize,
    Watermark,
    ExtractFrame,
    GenerateProxy,
    GenerateStoryboard,
    RemuxHls,
    Trim,
    RenderStock,
    AdminRender,
    MergeInputs,
    RemoveSilence,
    RenderAudioPlan,
    MuxAudioCopy,
    AssembleCopy,
}

impl Operation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Probe => "probe",
            Self::CutBatch => "cut_batch",
            Self::Normalize => "normalize",
            Self::CutCopy => "cut_copy",
            Self::CutAndNormalize => "cut_and_normalize",
            Self::Watermark => "watermark",
            Self::ExtractFrame => "extract_frame",
            Self::GenerateProxy => "generate_proxy",
            Self::GenerateStoryboard => "generate_storyboard",
            Self::RemuxHls => "remux_hls",
            Self::Trim => "trim",
            Self::RenderStock => "render_stock",
            Self::AdminRender => "admin_render",
            Self::MergeInputs => "merge_inputs",
            Self::RemoveSilence => "remove_silence",
            Self::RenderAudioPlan => "render_audio_plan",
            Self::MuxAudioCopy => "mux_audio_copy",
            Self::AssembleCopy => "assemble_copy",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Request {
    pub version: String,
    pub operation: Operation,
    pub ffmpeg_path: Option<String>,
    pub source_path: Option<String>,
    pub output_path: Option<String>,
    pub timestamp_sec: Option<f64>,
    pub start_sec: Option<f64>,
    pub end_sec: Option<f64>,
    pub interval_frames: Option<u32>,
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub jobs: Option<Vec<CutJob>>,
    #[serde(flatten)]
    pub media: config::MediaConfig,
    pub no_audio: Option<bool>,
    pub keep_audio: Option<bool>,
    pub overlay_path: Option<String>,
    pub opacity: Option<f64>,
    pub input_paths: Option<Vec<String>>,
    pub no_transitions: Option<bool>,
    pub clip_duration_sec: Option<i32>,
    pub transitions: Option<Vec<RenderTransition>>,
    // Legacy selection inputs are accepted only so the protocol can reject
    // unresolved requests explicitly; Rust never uses them to choose assets.
    pub transition_every: Option<i32>,
    pub effects_dir: Option<String>,
    pub effect_every: Option<i32>,
    pub effect_index_hint: Option<i32>,
    pub no_effects: Option<bool>,
    pub effect_paths: Option<Vec<RenderEffectPath>>,
    pub overlay_opacity: Option<f64>,
    pub font: Option<String>,
    pub effects: Option<Vec<RenderEffect>>,
    pub overlays: Option<Vec<RenderOverlay>>,
    pub max_duration_sec: Option<f64>,
    pub audio_plan: Option<serde_json::Value>,
    pub audio_assets: Option<Vec<AudioAsset>>,
    pub copy_certification: Option<CopyCertification>,
    // The Go boundary sends a sealed render plan. Rust treats it as an
    // audited contract and never derives timing or asset selection from it.
    pub render_plan: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AudioAsset {
    pub asset_id: String,
    pub path: String,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != PROTOCOL_VERSION {
            return Err(format!("unsupported protocol version: {}", self.version));
        }
        let require_source = || {
            if self.source_path.as_deref().unwrap_or("").trim().is_empty() {
                Err("source_path is required".to_string())
            } else {
                Ok(())
            }
        };
        let require_output = || {
            if self.output_path.as_deref().unwrap_or("").trim().is_empty() {
                Err("output_path is required".to_string())
            } else {
                Ok(())
            }
        };
        match self.operation {
            Operation::Health => Ok(()),
            Operation::Probe => require_source(),
            Operation::CutBatch => {
                require_source()?;
                if self.jobs.as_ref().map_or(true, Vec::is_empty) {
                    return Err("jobs are required".to_string());
                }
                Ok(())
            }
            Operation::RenderStock => {
                if self.input_paths.as_ref().map_or(true, Vec::is_empty) {
                    return Err("input_paths are required".to_string());
                }
                require_output()
            }
            Operation::AdminRender
            | Operation::CutCopy
            | Operation::Normalize
            | Operation::CutAndNormalize
            | Operation::Watermark
            | Operation::ExtractFrame
            | Operation::GenerateProxy
            | Operation::GenerateStoryboard
            | Operation::RemuxHls
            | Operation::Trim => {
                require_source()?;
                require_output()
            }
            Operation::RemoveSilence => {
                require_source()?;
                require_output()
            }
            Operation::MergeInputs => {
                if self.input_paths.as_ref().map_or(true, Vec::is_empty) {
                    return Err("input_paths are required".to_string());
                }
                require_output()
            }
            Operation::RenderAudioPlan => {
                require_output()?;
                if self.audio_plan.is_none() {
                    return Err("audio_plan is required".to_string());
                }
                if self.audio_assets.as_ref().map_or(true, Vec::is_empty) {
                    return Err("audio_assets are required".to_string());
                }
                Ok(())
            }
            Operation::MuxAudioCopy => {
                require_output()?;
                if self
                    .input_paths
                    .as_ref()
                    .map_or(true, |paths| paths.len() != 2)
                {
                    return Err("exactly video and final audio inputs are required".to_string());
                }
                Ok(())
            }
            Operation::AssembleCopy => {
                if self.input_paths.as_ref().map_or(true, Vec::is_empty) {
                    return Err("input_paths are required".to_string());
                }
                require_output()?;
                if self.copy_certification.is_none() {
                    return Err("copy_certification is required".to_string());
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct RenderTransition {
    pub clip_index: usize,
    pub segment: String,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RenderEffectPath {
    pub clip_index: usize,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RenderEffect {
    pub path: String,
    pub delay_ms: i32,
    pub duration: f64,
    pub volume: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RenderOverlay {
    pub text: String,
    pub start: String,
    pub end: String,
    pub size: String,
    pub y: String,
    pub color: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CutJob {
    pub job_id: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub output_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub ok: bool,
    pub operation: String,
    pub source_path: Option<String>,
    pub items: Vec<CutItem>,
    pub metadata: Option<MediaMetadata>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MediaMetadata {
    pub duration_sec: f64,
    pub bitrate: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: Option<String>,
    pub pixel_format: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_profile: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub start_pts: Option<i64>,
    pub has_video: bool,
    pub has_audio: bool,
    // Stage timings populated only by the combined-audio render path
    // (render_audio_plan). None everywhere else, and omitted on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aac_encode_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_ms: Option<i64>,
    // FFmpeg wall time for the media encode subprocess, measured natively by
    // render_stock (canonical + legacy composite) so callers get the encode
    // wall time without an external timing shim. None elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffmpeg_ms: Option<i64>,
    // SHA256 of the published final audio, computed by Rust so the Go adapter
    // never re-hashes the output it just rendered (single ownership of the
    // hash operation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_audio_sha256: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CutItem {
    pub job_id: String,
    pub output_path: Option<String>,
    pub status: String,
    pub size_bytes: u64,
    pub duration_sec: f64,
    pub error: Option<String>,
}

/// CopyCertification is the copy-only certification of an overlay artifact:
/// it declares that the artifact is a self-contained, correctly-encoded video
/// segment (closed GOP, first frame keyframe) that can be assembled by
/// packet-copy with zero decode/encode.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CopyCertification {
    pub copy_eligible: bool,
    pub profile_id: Option<String>,
    pub codec: Option<String>,
    pub codec_profile: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps_num: Option<u32>,
    pub fps_den: Option<u32>,
    pub closed_gop: Option<bool>,
    pub first_frame_keyframe: Option<bool>,
}

impl CopyCertification {
    /// Validates the copy-only certification. It fails closed: a missing or
    /// false field rejects the packet-copy path so the encoded composite path
    /// remains responsible for anything uncertified.
    pub fn validate(&self) -> Result<(), String> {
        if !self.copy_eligible {
            return Err("COPY_NOT_ELIGIBLE: copy_eligible is false".to_string());
        }
        if self.profile_id.as_deref().unwrap_or("").trim().is_empty() {
            return Err("CERTIFICATION_REQUIRED: profile_id is required".to_string());
        }
        if self.codec.as_deref().unwrap_or("").trim().is_empty() {
            return Err("CERTIFICATION_REQUIRED: codec is required".to_string());
        }
        if self.width.unwrap_or(0) == 0 || self.height.unwrap_or(0) == 0 {
            return Err("CERTIFICATION_REQUIRED: width/height are required".to_string());
        }
        if self.fps_num.unwrap_or(0) == 0 || self.fps_den.unwrap_or(0) == 0 {
            return Err("CERTIFICATION_REQUIRED: fps_num/fps_den are required".to_string());
        }
        if !self.closed_gop.unwrap_or(false) {
            return Err("CERTIFICATION_REQUIRED: closed_gop must be true".to_string());
        }
        if !self.first_frame_keyframe.unwrap_or(false) {
            return Err("CERTIFICATION_REQUIRED: first_frame_keyframe must be true".to_string());
        }
        Ok(())
    }

    /// Verifies that probed media matches the certified copy-only profile.
    pub fn verify_metadata(&self, metadata: &MediaMetadata) -> Result<(), String> {
        if !metadata.has_video {
            return Err("COPY_CERTIFICATION_VIOLATION: input has no video stream".to_string());
        }
        let width = self.width.unwrap_or(0);
        let height = self.height.unwrap_or(0);
        if metadata.width != width || metadata.height != height {
            return Err(format!(
                "COPY_CERTIFICATION_VIOLATION: geometry {}x{} != certified {}x{}",
                metadata.width, metadata.height, width, height
            ));
        }
        let fps = self.fps_num.unwrap_or(0) as f64 / self.fps_den.unwrap_or(1) as f64;
        if (metadata.fps - fps).abs() > 0.5 {
            return Err(format!(
                "COPY_CERTIFICATION_VIOLATION: fps {:.3} != certified {:.3}",
                metadata.fps, fps
            ));
        }
        if !copy_codec_matches(
            metadata.video_codec.as_deref(),
            self.codec.as_deref().unwrap_or(""),
        ) {
            return Err(format!(
                "COPY_CERTIFICATION_VIOLATION: codec {:?} != certified {}",
                metadata.video_codec,
                self.codec.as_deref().unwrap_or("")
            ));
        }
        Ok(())
    }
}

fn copy_codec_matches(actual: Option<&str>, expected: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    match actual {
        Some(codec) if expected == "h264" || expected == "libx264" || expected == "h264_nvenc" => {
            codec == "h264"
        }
        Some(codec) => codec == expected,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::CopyCertification;
    use crate::protocol::MediaMetadata;

    fn certified() -> CopyCertification {
        CopyCertification {
            copy_eligible: true,
            profile_id: Some("velox-h264-copy-v1".to_string()),
            codec: Some("h264".to_string()),
            codec_profile: Some("high".to_string()),
            width: Some(1920),
            height: Some(1080),
            fps_num: Some(30),
            fps_den: Some(1),
            closed_gop: Some(true),
            first_frame_keyframe: Some(true),
        }
    }

    fn metadata() -> MediaMetadata {
        MediaMetadata {
            duration_sec: 18.2,
            bitrate: None,
            width: 1920,
            height: 1080,
            fps: 30.0,
            video_codec: Some("h264".to_string()),
            pixel_format: Some("yuv420p".to_string()),
            audio_codec: None,
            audio_profile: None,
            sample_rate: None,
            channels: None,
        start_pts: None,
        has_video: true,
        has_audio: false,        mix_ms: None,
        aac_encode_ms: None,
        probe_ms: None,
        hash_ms: None,
        ffmpeg_ms: None,
        final_audio_sha256: None,
    }
}

    #[test]
    fn certified_metadata_matches_copy_only_profile() {
        assert!(certified().validate().is_ok());
        assert!(certified().verify_metadata(&metadata()).is_ok());
    }

    #[test]
    fn copy_not_eligible_fails_closed() {
        let mut cert = certified();
        cert.copy_eligible = false;
        assert!(cert.validate().unwrap_err().contains("COPY_NOT_ELIGIBLE"));
    }

    #[test]
    fn missing_certification_fields_fail_closed() {
        for cert in [
            CopyCertification {
                profile_id: None,
                ..certified()
            },
            CopyCertification {
                codec: None,
                ..certified()
            },
            CopyCertification {
                width: None,
                ..certified()
            },
            CopyCertification {
                fps_num: None,
                ..certified()
            },
            CopyCertification {
                closed_gop: None,
                ..certified()
            },
            CopyCertification {
                first_frame_keyframe: None,
                ..certified()
            },
        ] {
            assert!(cert.validate().is_err(), "expected certification to fail");
        }
    }

    #[test]
    fn metadata_violations_fail_closed() {
        let cert = certified();

        let mut wrong_geometry = metadata();
        wrong_geometry.width = 1280;
        assert!(cert
            .verify_metadata(&wrong_geometry)
            .unwrap_err()
            .contains("geometry"));

        let mut wrong_fps = metadata();
        wrong_fps.fps = 24.0;
        assert!(cert
            .verify_metadata(&wrong_fps)
            .unwrap_err()
            .contains("fps"));

        let mut wrong_codec = metadata();
        wrong_codec.video_codec = Some("hevc".to_string());
        assert!(cert
            .verify_metadata(&wrong_codec)
            .unwrap_err()
            .contains("codec"));

        let mut no_video = metadata();
        no_video.has_video = false;
        assert!(cert
            .verify_metadata(&no_video)
            .unwrap_err()
            .contains("no video"));
    }
}
