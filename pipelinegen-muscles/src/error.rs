//! Typed errors for the muscles crate.
//!
//! Every fallible operation returns `Result<T, MuscleError>` instead of
//! `Result<T, String>`. The `Display` rendering of each variant is
//! byte-identical to the historical `"CODE: detail"` strings the Go bridge
//! and the test suite match on, so typing the errors changes no wire
//! output — it only lets callers match on the failure kind (see
//! [`MuscleError::code`]) instead of substring-scanning a message.

/// Machine-readable failure code. The wire rendering keeps the historical
/// `"CODE: ..."` prefixes verbatim; [`ErrorCode::Generic`] renders the
/// bare message with no prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    CopyNotEligible,
    ProfileRequired,
    EncoderPolicyRequired,
    InvalidEncoderPreset,
    CertificationRequired,
    OutputContractMismatch,
    CopyCertificationViolation,
    AssemblyInputContractMismatch,
    FinalMuxDurationMismatch,
    FinalMuxAudioShorterThanVideo,
    FinalMuxAudioNotCopyEligible,
    Generic,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CopyNotEligible => "COPY_NOT_ELIGIBLE",
            Self::ProfileRequired => "PROFILE_REQUIRED",
            Self::EncoderPolicyRequired => "ENCODER_POLICY_REQUIRED",
            Self::InvalidEncoderPreset => "INVALID_ENCODER_PRESET",
            Self::CertificationRequired => "CERTIFICATION_REQUIRED",
            Self::OutputContractMismatch => "OUTPUT_CONTRACT_MISMATCH",
            Self::CopyCertificationViolation => "COPY_CERTIFICATION_VIOLATION",
            Self::AssemblyInputContractMismatch => "ASSEMBLY_INPUT_CONTRACT_MISMATCH",
            Self::FinalMuxDurationMismatch => "FINAL_MUX_DURATION_MISMATCH",
            Self::FinalMuxAudioShorterThanVideo => "FINAL_MUX_AUDIO_SHORTER_THAN_VIDEO",
            Self::FinalMuxAudioNotCopyEligible => "FINAL_MUX_AUDIO_NOT_COPY_ELIGIBLE",
            Self::Generic => "",
        }
    }
}

/// Typed muscle failure. Each variant renders exactly like the historical
/// string error, so the JSON wire format is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MuscleError {
    #[error("COPY_NOT_ELIGIBLE: {0}")]
    CopyNotEligible(String),
    #[error("PROFILE_REQUIRED: {0}")]
    ProfileRequired(String),
    #[error("ENCODER_POLICY_REQUIRED: {0}")]
    EncoderPolicyRequired(String),
    #[error("INVALID_ENCODER_PRESET: {0}")]
    InvalidEncoderPreset(String),
    #[error("CERTIFICATION_REQUIRED: {0}")]
    CertificationRequired(String),
    #[error("OUTPUT_CONTRACT_MISMATCH: {0}")]
    OutputContractMismatch(String),
    #[error("COPY_CERTIFICATION_VIOLATION: {0}")]
    CopyCertificationViolation(String),
    #[error("ASSEMBLY_INPUT_CONTRACT_MISMATCH: {0}")]
    AssemblyInputContractMismatch(String),
    #[error("FINAL_MUX_DURATION_MISMATCH: {0}")]
    FinalMuxDurationMismatch(String),
    #[error("FINAL_MUX_AUDIO_SHORTER_THAN_VIDEO: {0}")]
    FinalMuxAudioShorterThanVideo(String),
    #[error("FINAL_MUX_AUDIO_NOT_COPY_ELIGIBLE: {0}")]
    FinalMuxAudioNotCopyEligible(String),
    #[error("{0}")]
    Generic(String),
}

impl From<String> for MuscleError {
    fn from(message: String) -> Self {
        Self::Generic(message)
    }
}

impl From<&str> for MuscleError {
    fn from(message: &str) -> Self {
        Self::Generic(message.to_string())
    }
}

impl MuscleError {
    /// Compatibility helper for callers migrating from string errors.
    /// New code should match [`Self::code`] or inspect the typed variant.
    pub fn contains(&self, needle: &str) -> bool {
        self.to_string().contains(needle)
    }

    /// Machine-readable code for match-based handling (no substring scans).
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::CopyNotEligible(_) => ErrorCode::CopyNotEligible,
            Self::ProfileRequired(_) => ErrorCode::ProfileRequired,
            Self::EncoderPolicyRequired(_) => ErrorCode::EncoderPolicyRequired,
            Self::InvalidEncoderPreset(_) => ErrorCode::InvalidEncoderPreset,
            Self::CertificationRequired(_) => ErrorCode::CertificationRequired,
            Self::OutputContractMismatch(_) => ErrorCode::OutputContractMismatch,
            Self::CopyCertificationViolation(_) => ErrorCode::CopyCertificationViolation,
            Self::AssemblyInputContractMismatch(_) => ErrorCode::AssemblyInputContractMismatch,
            Self::FinalMuxDurationMismatch(_) => ErrorCode::FinalMuxDurationMismatch,
            Self::FinalMuxAudioShorterThanVideo(_) => ErrorCode::FinalMuxAudioShorterThanVideo,
            Self::FinalMuxAudioNotCopyEligible(_) => ErrorCode::FinalMuxAudioNotCopyEligible,
            Self::Generic(_) => ErrorCode::Generic,
        }
    }

    pub fn copy_not_eligible(detail: impl Into<String>) -> Self {
        Self::CopyNotEligible(detail.into())
    }

    pub fn profile_required(detail: impl Into<String>) -> Self {
        Self::ProfileRequired(detail.into())
    }

    pub fn encoder_policy_required(detail: impl Into<String>) -> Self {
        Self::EncoderPolicyRequired(detail.into())
    }

    pub fn invalid_encoder_preset(detail: impl Into<String>) -> Self {
        Self::InvalidEncoderPreset(detail.into())
    }

    pub fn certification_required(detail: impl Into<String>) -> Self {
        Self::CertificationRequired(detail.into())
    }

    pub fn output_contract_mismatch(detail: impl Into<String>) -> Self {
        Self::OutputContractMismatch(detail.into())
    }

    pub fn copy_certification_violation(detail: impl Into<String>) -> Self {
        Self::CopyCertificationViolation(detail.into())
    }

    pub fn assembly_input_contract_mismatch(detail: impl Into<String>) -> Self {
        Self::AssemblyInputContractMismatch(detail.into())
    }

    pub fn final_mux_duration_mismatch(detail: impl Into<String>) -> Self {
        Self::FinalMuxDurationMismatch(detail.into())
    }

    pub fn final_mux_audio_shorter_than_video(detail: impl Into<String>) -> Self {
        Self::FinalMuxAudioShorterThanVideo(detail.into())
    }

    pub fn final_mux_audio_not_copy_eligible(detail: impl Into<String>) -> Self {
        Self::FinalMuxAudioNotCopyEligible(detail.into())
    }

    /// Unprefixed failure (invalid input, I/O context, decode errors).
    /// Renders as the bare message, exactly as before.
    pub fn generic(detail: impl Into<String>) -> Self {
        Self::Generic(detail.into())
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, MuscleError};

    #[test]
    fn wire_rendering_is_byte_identical_to_historical_strings() {
        assert_eq!(
            MuscleError::copy_not_eligible("copy_eligible is false").to_string(),
            "COPY_NOT_ELIGIBLE: copy_eligible is false"
        );
        assert_eq!(
            MuscleError::profile_required("fps_num is required for encoded media").to_string(),
            "PROFILE_REQUIRED: fps_num is required for encoded media"
        );
        assert_eq!(
            MuscleError::generic("source_path is required").to_string(),
            "source_path is required"
        );
    }

    #[test]
    fn code_is_matchable_without_substring_scans() {
        let error = MuscleError::output_contract_mismatch("V2 requires 1920x1080");
        assert_eq!(error.code(), ErrorCode::OutputContractMismatch);
    }
}
