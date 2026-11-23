use serde::Deserialize;

use crate::config;
use crate::error::MuscleError;

use super::response::MediaMetadata;
use super::VELOX_ASSEMBLY_READY_V2;

/// CopyCertification is the copy-only certification of an overlay artifact:
/// it declares that the artifact is a self-contained, correctly-encoded video
/// segment (closed GOP, first frame keyframe) that can be assembled by
/// packet-copy with zero decode/encode.
///
/// PRODUCTION STATUS (2026-09-23): this certification and the gate below are
/// the VeloxEditing-side half of a SHARED contract and the LIVE assembly path
/// for the durable video.create workflow (see transform_assemble.rs).
/// RenderingGen's `ParentFinalizer` -> daemon `ASSEMBLE_SEGMENTS`, whose
/// `queue.ValidateChildren` enforces the same three facts (copy_eligible,
/// closed_gop, first_frame_keyframe) before assembling, keeps assembling the
/// chunks of ONE render job — so the two paths cannot disagree about what is
/// assemblable.
///
/// Assembly-ready gate fields (added Wave 5, Aug 2026):
///   - contract_id: MUST be "VELOX_ASSEMBLY_READY_V1"
///   - video_extradata_sha256: all inputs in a batch MUST share the exact avcC identity
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
    /// Assembly-ready contract identity ("VELOX_ASSEMBLY_READY_V1").
    /// The assembly gate rejects a missing, empty, or unknown contract, so
    /// every cert that reaches execution carries the canonical identity.
    #[serde(default)]
    pub contract_id: Option<String>,
    #[serde(default)]
    pub video_extradata_sha256: Option<String>,
    #[serde(default)]
    pub audio_extradata_sha256: Option<String>,
}

impl CopyCertification {
    /// Validates the copy-only certification. It fails closed: a missing or
    /// false field rejects the packet-copy path so the encoded composite path
    /// remains responsible for anything uncertified.
    pub fn validate(&self) -> Result<(), MuscleError> {
        if !self.copy_eligible {
            return Err(MuscleError::copy_not_eligible("copy_eligible is false"));
        }
        if self.profile_id.as_deref().unwrap_or("").trim().is_empty() {
            return Err(MuscleError::certification_required(
                "profile_id is required",
            ));
        }
        if self.codec.as_deref().unwrap_or("").trim().is_empty() {
            return Err(MuscleError::certification_required("codec is required"));
        }
        if self.width.unwrap_or(0) == 0 || self.height.unwrap_or(0) == 0 {
            return Err(MuscleError::certification_required(
                "width/height are required",
            ));
        }
        if self.fps_num.unwrap_or(0) == 0 || self.fps_den.unwrap_or(0) == 0 {
            return Err(MuscleError::certification_required(
                "fps_num/fps_den are required",
            ));
        }
        if self
            .video_extradata_sha256
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            return Err(MuscleError::certification_required(
                "video_extradata_sha256 is required for every copy certification",
            ));
        }
        if self.contract_id.as_deref() == Some(VELOX_ASSEMBLY_READY_V2) {
            if self.width != Some(1920) || self.height != Some(1080) {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 requires 1920x1080",
                ));
            }
            if self.fps_num != Some(24) || self.fps_den != Some(1) {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 requires fps=24/1",
                ));
            }
            if self.codec.as_deref() != Some("h264")
                || self.codec_profile.as_deref() != Some("high")
            {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 requires h264 High",
                ));
            }
            if self
                .audio_extradata_sha256
                .as_deref()
                .unwrap_or("")
                .is_empty()
            {
                return Err(MuscleError::certification_required(
                    "V2 requires an audio extradata hash",
                ));
            }
        }
        if !self.closed_gop.unwrap_or(false) {
            return Err(MuscleError::certification_required(
                "closed_gop must be true",
            ));
        }
        if !self.first_frame_keyframe.unwrap_or(false) {
            return Err(MuscleError::certification_required(
                "first_frame_keyframe must be true",
            ));
        }
        Ok(())
    }

    /// Verifies that probed media matches the certified copy-only profile.
    pub fn verify_metadata(&self, metadata: &MediaMetadata) -> Result<(), MuscleError> {
        if !metadata.has_video {
            return Err(MuscleError::copy_certification_violation(
                "input has no video stream",
            ));
        }
        let width = self.width.unwrap_or(0);
        let height = self.height.unwrap_or(0);
        if metadata.width != width || metadata.height != height {
            return Err(MuscleError::copy_certification_violation(format!(
                "geometry {}x{} != certified {}x{}",
                metadata.width, metadata.height, width, height
            )));
        }
        let expected_num = self.fps_num.unwrap_or(0);
        let expected_den = self.fps_den.unwrap_or(0);
        if expected_num == 0
            || expected_den == 0
            || metadata.fps_num == 0
            || metadata.fps_den == 0
            || metadata.fps_num * expected_den != expected_num * metadata.fps_den
        {
            return Err(MuscleError::copy_certification_violation(format!(
                "fps {}/{} != certified {}/{}",
                metadata.fps_num, metadata.fps_den, expected_num, expected_den
            )));
        }
        if metadata.video_extradata_sha256.as_deref() != self.video_extradata_sha256.as_deref() {
            return Err(MuscleError::copy_certification_violation(format!(
                "video extradata {:?} != certified {:?}",
                metadata.video_extradata_sha256, self.video_extradata_sha256
            )));
        }
        if self.contract_id.as_deref() == Some(VELOX_ASSEMBLY_READY_V2) {
            let checks = [
                (
                    metadata.video_profile.as_deref(),
                    Some("high"),
                    "video profile",
                ),
                (metadata.video_level.as_deref(), Some("4.1"), "video level"),
                (
                    metadata.pixel_format.as_deref(),
                    Some("yuv420p"),
                    "pixel format",
                ),
            ];
            for (actual, expected, label) in checks {
                if actual != expected {
                    return Err(MuscleError::output_contract_mismatch(label));
                }
            }
            if metadata.video_time_base_num != Some(1)
                || metadata.video_time_base_den != Some(90000)
                || metadata.sar_num != Some(1)
                || metadata.sar_den != Some(1)
                || metadata.field_order.as_deref() != Some("progressive")
                || metadata.keyframe_interval != Some(48)
                || metadata.b_frames != Some(0)
                || metadata.closed_gop != Some(true)
            {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 video timing/GOP properties",
                ));
            }
            if metadata.audio_codec.as_deref() != Some("aac")
                || metadata.audio_profile.as_deref() != Some("LC")
                || metadata.audio_time_base_num != Some(1)
                || metadata.audio_time_base_den != Some(config::CANONICAL_SAMPLE_RATE as u32)
                || metadata.sample_rate != Some(config::CANONICAL_SAMPLE_RATE as u32)
                || metadata.channels != Some(2)
                || metadata.channel_layout.as_deref() != Some("stereo")
                || metadata.audio_bitrate != Some(192000)
            {
                return Err(MuscleError::output_contract_mismatch("V2 audio properties"));
            }
            if metadata.audio_extradata_sha256.as_deref() != self.audio_extradata_sha256.as_deref()
            {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 audio extradata hash",
                ));
            }
        }
        if !copy_codec_matches(
            metadata.video_codec.as_deref(),
            self.codec.as_deref().unwrap_or(""),
        ) {
            return Err(MuscleError::copy_certification_violation(format!(
                "codec {:?} != certified {}",
                metadata.video_codec,
                self.codec.as_deref().unwrap_or("")
            )));
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
