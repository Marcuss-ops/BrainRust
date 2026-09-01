//! VisualEntity and supporting value types for the deterministic
//! VisualNER extractor. The struct mirrors the Go
//! `internal/kernel/sceneir.VisualEntity` shape so the two layers can pass
//! entities across the FFI/JSON boundary without a translation layer.

use serde::{Deserialize, Serialize};

/// A single source-grounded visual entity extracted from a scene's
/// source text. V1 is deterministic: NO EVIDENCE → NO ENTITY — if the
/// entity text cannot be demonstrated as a verbatim substring of the
/// source text, it is rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualEntity {
    /// The entity surface form (e.g. "feta cheese"). Preserves the
    /// original case of the source text.
    pub text: String,
    /// Closed semantic type shared with the Go SceneIR wire contract.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The deterministic visualness score in `[0.0, 1.0]`; higher is
    /// more visually concrete. NOT a provider relevance score.
    pub score: f32,
    /// The inclusive byte offset into the source text where the
    /// entity's evidence begins.
    pub start: usize,
    /// The exclusive byte offset into the source text where the
    /// entity's evidence ends.
    pub end: usize,
    /// The verbatim source text excerpt at `[start, end)`. Proves the
    /// entity is grounded in the canonical source (NO EVIDENCE → NO ENTITY).
    #[serde(default)]
    pub evidence: String,
}

/// Options for the extractor. `entity_count` is the requested maximum
/// named-entity count for the segment (the `top N`); zero uses the
/// extractor's safe default (3).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractOptions {
    #[serde(default)]
    pub entity_count: usize,
}

impl ExtractOptions {
    /// Returns the effective top-N to return. Zero falls back to 3
    /// (the VidRush Mediterranean golden fixture's expected entity count).
    pub fn top_n(&self) -> usize {
        if self.entity_count == 0 {
            3
        } else {
            self.entity_count
        }
    }
}
