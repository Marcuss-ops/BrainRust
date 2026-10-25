//! Per-run asset binding ledger (`BoundAssets`): cross-scene reuse
//! detection and the overlap evidence consumed by the diversity stage.

use super::tokens::{label_tokens, Jaccard};

/// BoundAssets tracks asset ids already bound to scenes so the sampler can
/// reject cross-scene reuse when `allow_reuse = false`. It is the
/// per-run binding ledger the sampler consults before accepting a winner.
/// Each entry also carries the label the asset won under, which powers the
/// V2 diversity stage (semantic overlap with already-bound winners).
#[derive(Debug, Clone, Default)]
pub struct BoundAssets {
    /// Map of asset id -> (segment id that bound it first, label).
    pub(super) bound: std::collections::HashMap<String, (String, String)>,
}

impl BoundAssets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when `asset_id` is already bound to a scene other than
    /// `scene_id`.
    pub fn is_reuse(&self, asset_id: &str, scene_id: &str) -> bool {
        match self.bound.get(asset_id) {
            Some((owner, _)) => owner != scene_id,
            None => false,
        }
    }

    /// Records that `scene_id` has bound `asset_id` (no label; the diversity
    /// stage then receives no overlap evidence from this entry).
    pub fn bind(&mut self, asset_id: &str, scene_id: &str) {
        self.bind_label(asset_id, scene_id, "");
    }

    /// `bind` plus the winning label — the caller-facing constructor the
    /// winner path uses so diversity has real overlap evidence.
    pub fn bind_label(&mut self, asset_id: &str, scene_id: &str, label: &str) {
        self.bound.insert(
            asset_id.to_string(),
            (scene_id.to_string(), label.to_string()),
        );
    }

    /// Highest token-overlap (Jaccard in `[0.0, 1.0]`) between `tokens` and
    /// any already-bound label. Empty ledger → 0.0 (no diversity context).
    pub(crate) fn best_label_overlap(&self, tokens: &std::collections::BTreeSet<String>) -> f32 {
        self.bound
            .values()
            .filter(|(_, label)| !label.trim().is_empty())
            .map(|(_, label)| label_tokens(label).jaccard(tokens))
            .fold(0.0_f32, f32::max)
    }

    /// True when at least one bound entry carries a winning label, i.e. when
    /// the diversity stage HAS overlap evidence to reason about.
    pub(crate) fn has_any_label(&self) -> bool {
        self.bound
            .values()
            .any(|(_, label)| !label.trim().is_empty())
    }
}
