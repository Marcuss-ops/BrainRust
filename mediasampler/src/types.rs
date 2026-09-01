//! Value types for the semantic MediaSampler. The sampler ranks candidate
//! assets for one scene against a hard-constraint + soft-scoring pipeline
//! and is the deterministic replacement for the LIVE-test bug where Artlist
//! returned a boxing clip for the Greek Salad segment.

use serde::{Deserialize, Serialize};

/// A scene whose assets are being sampled. Mirrors the SceneIR compact
/// profile (subject + visual terms) plus the ownership identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// The canonical segment id that owns the scene; a candidate bound to
    /// another scene is rejected (ASSET OWNERSHIP / cross-scene reuse).
    pub id: String,
    /// The single canonical subject the winner must be compatible with.
    pub subject: String,
    /// The concrete, source-grounded visual terms that should appear.
    pub terms: Vec<String>,
}

/// A candidate asset under consideration for a scene. `generic_similarity`
/// is the raw provider/vector similarity; the sampler does NOT pick the
/// highest generic similarity — it applies hard constraints first, then a
/// semantic score, then diversity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Stable candidate id (used for cross-scene reuse detection).
    pub id: String,
    /// Human-facing label (entity / query / title) used for subject match.
    pub label: String,
    /// Raw provider/vector similarity in `[0.0, 1.0]`. NOT the final score.
    #[serde(default)]
    pub generic_similarity: f32,
    /// The segment id this candidate was originally discovered for. When
    /// non-empty and different from the sampling scene id, the candidate
    /// fails ownership validation.
    #[serde(default)]
    pub owner_segment_id: String,
}

/// RejectionReason explains why a candidate did not win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    /// The candidate's owner segment does not match the sampling scene.
    OwnerMismatch,
    /// The candidate was already bound to another scene and reuse is
    /// forbidden (allow_reuse = false).
    AlreadyBound,
    /// The candidate's subject is not compatible with the scene subject
    /// (e.g. boxing for Greek Salad).
    SubjectMismatch,
}

impl RejectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RejectionReason::OwnerMismatch => "owner_mismatch",
            RejectionReason::AlreadyBound => "already_bound",
            RejectionReason::SubjectMismatch => "subject_mismatch",
        }
    }
}

/// SampleResult is the outcome for one candidate: either it is rejected
/// with a reason, or it receives a final score and may become the winner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleResult {
    pub candidate_id: String,
    pub label: String,
    /// Final semantic score in `[0.0, 1.0]` when accepted; 0.0 when
    /// rejected. Higher is better.
    pub score: f32,
    /// Rejection reason when the candidate was rejected; `None` when
    /// accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<RejectionReason>,
    /// Human-facing explanation of the accept/reject decision.
    pub reason: String,
}

impl SampleResult {
    pub fn accepted(candidate_id: &str, label: &str, score: f32, reason: &str) -> Self {
        Self {
            candidate_id: candidate_id.to_string(),
            label: label.to_string(),
            score,
            rejection: None,
            reason: reason.to_string(),
        }
    }

    pub fn rejected(
        candidate_id: &str,
        label: &str,
        reason_kind: RejectionReason,
        reason: &str,
    ) -> Self {
        Self {
            candidate_id: candidate_id.to_string(),
            label: label.to_string(),
            score: 0.0,
            rejection: Some(reason_kind),
            reason: reason.to_string(),
        }
    }

    pub fn is_accepted(&self) -> bool {
        self.rejection.is_none()
    }
}

/// Sampler options. `allow_reuse` controls the cross-scene reuse check.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SampleOptions {
    /// When false, a candidate already bound to another scene is rejected
    /// with `AlreadyBound`. When true, reuse is allowed.
    #[serde(default)]
    pub allow_reuse: bool,
    /// Number of images to select per scene (image fanout). 0 disables.
    #[serde(default)]
    pub images_per_scene: usize,
}
