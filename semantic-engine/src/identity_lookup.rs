//! Per-scene bridge from local entity indices to canonical identity ids.
//!
//! The engine analyses scene by scene, but identity is script-wide: the same
//! person in three scenes must keep one canonical id. Each scene registers its
//! grounded entities with the shared [`IdentityRegistry`] and remembers the
//! local-index -> canonical-id mapping so proposals can reference ids instead
//! of scene-local positions.

use semantic_core::IdentityRegistry;
use visualner::VisualEntity;

/// Maps scene-local entity indices to canonical identity ids.
pub struct IdentityLookup {
    ids: Vec<String>,
}

impl IdentityLookup {
    /// Registers every grounded entity of one scene and builds the lookup.
    pub fn build(
        registry: &mut IdentityRegistry,
        scene_id: &str,
        entities: &[VisualEntity],
    ) -> Self {
        let ids = entities
            .iter()
            .map(|entity| {
                registry.register(
                    &entity.r#type,
                    &entity.text,
                    scene_id,
                    entity.start,
                    entity.end,
                )
            })
            .collect();
        Self { ids }
    }

    /// Canonical id for a scene-local entity index. Indices are always in
    /// range (proposals carry indices from the same list), but this stays
    /// total so a stale index degrades to a placeholder instead of panicking.
    pub fn id_of(&self, index: usize) -> String {
        self.ids
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("entity_{:02}", index + 1))
    }
}
