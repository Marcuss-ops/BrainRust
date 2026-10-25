//! Canonical entity identity registry.
//!
//! The same person mentioned in three scenes must keep ONE canonical id, and
//! two distinct subjects must never be merged. The registry assigns ids by a
//! normalized identity key (lowercased, whitespace-collapsed surface plus its
//! semantic type). Surfaces that differ only in case or spacing map to one
//! canonical id; surfaces with different types (`PERSON` vs `ORG`) are kept
//! apart even when they share a surface string.

use std::collections::HashMap;

use serde::Serialize;

/// One canonical identity shared by every occurrence of a subject across the
/// script. `mentions` records every (scene_id, surface) pair so grounding and
/// the planner can trace the id back to source evidence.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CanonicalIdentity {
    /// Stable id for the subject (e.g. `person_01`, `org_02`).
    pub id: String,
    /// The surface used as the canonical display form (first-seen surface).
    pub canonical_surface: String,
    /// Closed semantic type (`PERSON`, `ORG`, `LOCATION`, `DATE`, ...).
    #[serde(rename = "type")]
    pub kind: String,
    /// Every (scene_id, surface) mention that resolved to this identity.
    pub mentions: Vec<Mention>,
}

/// A single occurrence of a canonical identity in one scene.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mention {
    pub scene_id: String,
    pub surface: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Assigns canonical ids across scenes. Ids are deterministic: the numbering
/// follows first-registration order, so the same input always produces the
/// same ids (idempotent replay).
#[derive(Debug, Default)]
pub struct IdentityRegistry {
    by_key: HashMap<String, usize>,
    identities: Vec<CanonicalIdentity>,
}

impl IdentityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one occurrence and returns the canonical id it resolves to.
    pub fn register(
        &mut self,
        kind: &str,
        surface: &str,
        scene_id: &str,
        start_byte: usize,
        end_byte: usize,
    ) -> String {
        self.register_mention(kind, surface, surface, scene_id, start_byte, end_byte)
    }

    /// Registers one mention whose `surface` is an alias of the canonical
    /// `canonical_surface` ("Stasi" for "Alberto Stasi"). The identity is keyed
    /// and displayed by the canonical surface, while the mention keeps the
    /// text that is actually in the scene, so evidence stays grounded and a
    /// surname never becomes a second person.
    pub fn register_mention(
        &mut self,
        kind: &str,
        canonical_surface: &str,
        surface: &str,
        scene_id: &str,
        start_byte: usize,
        end_byte: usize,
    ) -> String {
        let key = identity_key(kind, canonical_surface);
        let index = match self.by_key.get(&key) {
            Some(index) => *index,
            None => {
                let index = self.identities.len();
                self.by_key.insert(key, index);
                let prefix = id_prefix(kind);
                self.identities.push(CanonicalIdentity {
                    id: format!("{prefix}_{:02}", index + 1),
                    canonical_surface: canonical_surface.to_string(),
                    kind: kind.to_string(),
                    mentions: Vec::new(),
                });
                index
            }
        };
        self.identities[index].mentions.push(Mention {
            scene_id: scene_id.to_string(),
            surface: surface.to_string(),
            start_byte,
            end_byte,
        });
        self.identities[index].id.clone()
    }

    /// Every canonical identity in registration order.
    pub fn identities(&self) -> &[CanonicalIdentity] {
        &self.identities
    }
}

/// The identity key: type + whitespace-collapsed lowercase surface. Two
/// surfaces that differ only in case/spacing share a key; different types do
/// not, so `PERSON "Mark"` and `ORG "Mark"` stay distinct subjects.
fn identity_key(kind: &str, surface: &str) -> String {
    let mut collapsed = String::with_capacity(surface.len());
    let mut last_space = false;
    for ch in surface.trim().chars() {
        if ch.is_whitespace() {
            if !last_space {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(ch.to_lowercase().next().unwrap_or(ch));
            last_space = false;
        }
    }
    format!("{kind}\u{1f}{collapsed}")
}

fn id_prefix(kind: &str) -> &'static str {
    match kind {
        "PERSON" => "person",
        "ORG" => "org",
        "LOCATION" => "location",
        "DATE" => "date",
        "PERCENT" | "NUMBER" | "MONEY" => "number",
        _ => "entity",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_subject_across_scenes_shares_one_id() {
        let mut registry = IdentityRegistry::new();
        let a = registry.register("PERSON", "Alberto Stasi", "scene_01", 0, 12);
        let b = registry.register("PERSON", "alberto  stasi", "scene_02", 4, 16);
        assert_eq!(a, b);
        let identities = registry.identities();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].mentions.len(), 2);
    }

    #[test]
    fn distinct_types_never_merge() {
        let mut registry = IdentityRegistry::new();
        let person = registry.register("PERSON", "Mark", "scene_01", 0, 4);
        let org = registry.register("ORG", "Mark", "scene_01", 10, 14);
        assert_ne!(person, org);
        assert_eq!(registry.identities().len(), 2);
    }

    #[test]
    fn distinct_subjects_never_merge() {
        let mut registry = IdentityRegistry::new();
        let a = registry.register("PERSON", "Alberto Stasi", "scene_01", 0, 12);
        let b = registry.register("PERSON", "Andrea Sempio", "scene_01", 20, 33);
        assert_ne!(a, b);
    }

    #[test]
    fn ids_are_deterministic_across_runs() {
        let build = || {
            let mut registry = IdentityRegistry::new();
            let a = registry.register("PERSON", "Alberto Stasi", "scene_01", 0, 12);
            let b = registry.register("PERSON", "Andrea Sempio", "scene_02", 0, 13);
            (a, b)
        };
        assert_eq!(build(), build());
    }
}
