//! Canonical entity identity registry and the v3 linked-entity projection.
//!
//! The same person mentioned in three scenes must keep ONE canonical id, and
//! two distinct subjects must never be merged. The registry assigns ids by a
//! normalized identity key (Unicode NFC, apostrophe-folded, lowercased and
//! whitespace-collapsed surface plus its semantic type). Surfaces that differ
//! only in case, spacing or apostrophe form map to one canonical id; surfaces
//! with different types (`PERSON` vs `ORG`) are kept apart even when they share
//! a surface string.
//!
//! `canonical_id` remains the human-readable v2 surface slug. `surface_key`
//! (v3) adds a deterministic short hash of the full typed key, so two distinct
//! keys that slug to the same text can never collide in a cache.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// One canonical identity shared by every occurrence of a subject across the
/// script. `mentions` records every (scene_id, surface) pair so grounding and
/// the planner can trace the id back to source evidence.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CanonicalIdentity {
    /// Document-local id for references within this manifest (e.g. `org_02`).
    pub id: String,
    /// Surface-derived stable identity key, independent of document order.
    /// This is a typed surface slug, not an ontology link. The v3 `entities`
    /// projection carries the linked knowledge-base id separately.
    #[serde(default)]
    pub canonical_id: String,
    /// The surface used as the canonical display form (first-seen surface).
    pub canonical_surface: String,
    /// Closed semantic type (`PERSON`, `ORG`, `LOCATION`, `DATE`, ...).
    #[serde(rename = "type")]
    pub kind: String,
    /// Every (scene_id, surface) mention that resolved to this identity.
    pub mentions: Vec<Mention>,
}

/// A single occurrence of a canonical identity in one scene.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Mention {
    pub scene_id: String,
    pub surface: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Resolution state of one entity against the local knowledge index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    Linked,
    Ambiguous,
    Unlinked,
}

/// Whether an identity is a named subject or a generic visual concept.
/// The VLM identity gate applies only to `NamedEntity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityRole {
    NamedEntity,
    VisualConcept,
}

/// One typed, ordered retrieval intent for an entity.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SearchIntent {
    /// `identity` (who/what this is) or `contextual` (B-roll / scene context).
    pub kind: String,
    /// Short query string. Never a raw token window or a full sentence.
    pub query: String,
    /// BCP-47 language of the query.
    pub lang: String,
}

/// One auditable candidate considered by the local entity linker.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct LinkCandidate {
    pub kb_id: String,
    pub name: String,
    pub score: i32,
    pub features: Vec<LinkFeature>,
}

/// A deterministic scoring feature and its verbatim evidence locations.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct LinkFeature {
    pub name: String,
    pub value: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terms: Vec<String>,
    /// UTF-8 byte spans in the original scene text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<LinkEvidenceSpan>,
}

/// Source location for an explanation feature.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct LinkEvidenceSpan {
    pub scene_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// The v3 entity projection consumed by the resolver. It is additive: the v2
/// scene-level `identities` collection remains available for old consumers.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct LinkedEntity {
    /// Document-local identity id (`org_01`), used by manifest references.
    pub local_id: String,
    /// Collision-safe document-scoped key. It is not a KB id.
    pub surface_key: String,
    /// Canonical display surface from the source.
    pub canonical_surface: String,
    /// Linked knowledge-base id (`wd:Q42262`) only when `link_status` is
    /// `linked`; `None` for `ambiguous` and `unlinked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
    pub link_status: LinkStatus,
    /// Resolved KB name for a linked entity, otherwise the source surface.
    pub canonical_name: String,
    pub entity_type: String,
    pub role: EntityRole,
    /// Short disambiguation gloss from the index; linked entities only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disambiguation_gloss: Option<String>,
    /// Verifier exclusions from the index; linked entities only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_constraints: Vec<String>,
    /// Every source mention, with verbatim surface and UTF-8 byte offsets.
    pub mentions: Vec<Mention>,
    /// Ordered intents: identity first, then at most two contextual intents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_intents: Vec<SearchIntent>,
    /// Top three candidates and evidence for auditing disambiguation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_candidates: Vec<LinkCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_margin: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_rule: Option<String>,
    /// The surface after the linker's editorial repair. Consumers project this
    /// value; they must not re-run the repair rules in Go.
    #[serde(default)]
    pub editorial_surface: String,
    /// The annotation type after the linker's editorial repair.
    #[serde(default)]
    pub editorial_type: String,
    /// True when the surface is an instruction/template echo, never an entity.
    #[serde(default)]
    pub prompt_echo: bool,
    /// Admission decision for an identity image slot. Asset verification is
    /// still projected by the consumer; the entity-level policy lives here.
    #[serde(default)]
    pub visual_admission: bool,
}

/// The editorial decision attached to one linked identity. It is pure and
/// deterministic so the same rule runs for every manifest producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorialDecision {
    pub surface: String,
    pub kind: String,
    pub prompt_echo: bool,
    pub visual_admission: bool,
}

/// Repairs recurring NER errors and rejects prompt echoes before an identity
/// can reserve a text or visual slot. This is the single owner of those rules;
/// PipelineGen only projects the returned decision.
pub fn editorial_decision(surface: &str, kind: &str) -> EditorialDecision {
    let value = surface.split_whitespace().collect::<Vec<_>>().join(" ");
    let key = normalize_surface(&value)
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '.' | ','
                        | ';'
                        | ':'
                        | '!'
                        | '?'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '"'
                        | '\''
                        | '“'
                        | '”'
                )
        })
        .to_string();
    let mut kind = kind.to_string();

    if matches!(key.as_str(), "dopo" | "capire")
        && matches!(kind.as_str(), "LOCATION" | "GPE" | "PLACE")
    {
        return EditorialDecision {
            surface: value,
            kind: String::new(),
            prompt_echo: false,
            visual_admission: false,
        };
    }
    if key == "dna di chiara" {
        kind = "CONCEPT".to_string();
    }
    if is_prompt_echo(&key) {
        return EditorialDecision {
            surface: value,
            kind: String::new(),
            prompt_echo: true,
            visual_admission: false,
        };
    }
    if kind == "PERSON" && starts_with_acronym(&value) {
        kind = "CONCEPT".to_string();
    }
    let visual_admission =
        role_of(&kind) == EntityRole::NamedEntity && !starts_with_acronym(&value);
    EditorialDecision {
        surface: value,
        kind,
        prompt_echo: false,
        visual_admission,
    }
}

fn is_prompt_echo(name: &str) -> bool {
    matches!(name, "overlay check" | "data documentata")
        || name.starts_with("overlay:")
        || name.starts_with("scheda entit")
        || name.starts_with("mappa pertinente")
        || name.starts_with("frase breve")
        || name.starts_with("frase lunga")
        || name.starts_with("immagine (")
}

fn starts_with_acronym(value: &str) -> bool {
    let mut fields = value.split_whitespace();
    let Some(first) = fields.next() else {
        return false;
    };
    if fields.next().is_none() {
        return false;
    }
    let letters = first
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    (2..=6).contains(&letters)
        && first
            .chars()
            .all(|character| !character.is_alphabetic() || character.is_uppercase())
}

/// Named kinds are eligible for an identity lookup and the VLM identity gate.
/// Generic visual concepts and objects are B-roll/stock subjects instead.
/// The editorial role a kind plays in a decision: a named identity gets an
/// identity card, a visual concept only gets a frame.
///
/// This is deliberately narrower than `entity_capabilities::entity_capabilities`'
/// `named_kind` flag. That flag answers "does the identity registry key this
/// kind as a name?" and includes the TICKET-43 additive types (ANIMAL, FOOD,
/// VEHICLE, ...), which are pictured and registered but are not identities: a
/// tiger is not a person and must not get a name card. MEDIA_OUTLET is the
/// opposite case: an outlet is an organization for identity and image binding,
/// so it belongs here even though it is not a pictureable *subject* in
/// `NAMED_SUBJECTS`.
pub fn role_of(kind: &str) -> EntityRole {
    match kind {
        "PERSON" | "ORG" | "ORGANIZATION" | "LOCATION" | "FACILITY" | "BRAND" | "PRODUCT"
        | "WORK" | "EVENT" | "ROLE" | "MEDIA_OUTLET" => EntityRole::NamedEntity,
        _ => EntityRole::VisualConcept,
    }
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

    /// Registers one mention whose `surface` may be an alias or inflected form
    /// of `canonical_surface` ("Stasi" for "Alberto Stasi", or Polish
    /// "pokoju" for "pokój"). Identity keys and ids use the canonical form;
    /// the public canonical surface stays the first grounded source mention.
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
                let canonical_id = format!("{prefix}:{}", stable_slug(canonical_surface));
                self.identities.push(CanonicalIdentity {
                    id: format!("{prefix}_{:02}", index + 1),
                    canonical_id,
                    canonical_surface: surface.to_string(),
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

/// Normalizes a surface for matching only: Unicode NFC, apostrophe folding,
/// lowercasing and whitespace collapsing. The stored surface and every source
/// span stay verbatim.
pub fn normalize_surface(surface: &str) -> String {
    let normalized: String = surface.nfc().collect();
    let mut out = String::with_capacity(normalized.len());
    let mut last_space = false;
    for character in normalized.trim().chars() {
        let character = match character {
            '\u{2018}' | '\u{2019}' | '\u{02bb}' | '\u{02bc}' | '\u{0060}' | '\u{ff07}' => '\'',
            other => other,
        };
        if character.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            for lower in character.to_lowercase() {
                out.push(lower);
            }
            last_space = false;
        }
    }
    out
}

/// The identity key: type + normalized surface. Two surfaces that differ only
/// in case, spacing, Unicode composition or apostrophe form share a key;
/// different types do not, so `PERSON "Mark"` and `ORG "Mark"` stay distinct.
fn identity_key(kind: &str, surface: &str) -> String {
    format!("{kind}\u{1f}{}", normalize_surface(surface))
}

fn id_prefix(kind: &str) -> &'static str {
    match kind {
        "PERSON" => "person",
        // A media outlet is an organization for every identity consumer; the
        // specific extraction type must not split its canonical id.
        "ORG" | "ORGANIZATION" | "MEDIA_OUTLET" => "org",
        "LOCATION" => "location",
        "DATE" => "date",
        "PERCENT" | "NUMBER" | "MONEY" | "MEASUREMENT" => "number",
        _ => "entity",
    }
}

/// Builds an order-independent cache identity without claiming that a
/// surface-based key has resolved a real-world homonym. Unicode letters and
/// numbers are retained; punctuation and repeated whitespace become one
/// hyphen. Type remains part of the key, so `person:mark` and `org:mark` differ.
fn stable_slug(surface: &str) -> String {
    let mut slug = String::with_capacity(surface.len());
    let mut separator = false;
    for character in normalize_surface(surface)
        .chars()
        .flat_map(char::to_lowercase)
    {
        if character.is_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.push(character);
        } else {
            separator = true;
        }
    }
    if slug.is_empty() {
        "unresolved".to_string()
    } else {
        slug
    }
}

/// Collision-safe document-scoped key for the v3 projection. The hash covers
/// the full typed key, so `person:anne-marie` and `person:anne marie` never
/// share a cache entry even though they slug to the same text.
pub fn stable_surface_key(kind: &str, surface: &str) -> String {
    let key = identity_key(kind, surface);
    format!(
        "{}:{}-{:08x}",
        id_prefix(kind),
        stable_slug(surface),
        fnv1a32(key.as_bytes())
    )
}

fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
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
    fn identity_key_uses_lemma_but_surface_remains_grounded() {
        let mut registry = IdentityRegistry::new();
        let id = registry.register_mention("VISUAL_CONCEPT", "pokój", "pokoju", "s1", 0, 6);

        assert_eq!(id, "entity_01");
        let identity = &registry.identities()[0];
        assert_eq!(
            identity.canonical_id,
            format!("entity:{}", stable_slug("pokój"))
        );
        assert_eq!(identity.canonical_surface, "pokoju");
        assert_eq!(identity.mentions[0].surface, "pokoju");
        assert_eq!(identity.mentions[0].start_byte, 0);
        assert_eq!(identity.mentions[0].end_byte, 6);
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

    #[test]
    fn canonical_id_does_not_depend_on_registration_order() {
        let mut paragraph = IdentityRegistry::new();
        paragraph.register("ORG", "NASA", "s1", 0, 4);
        let expected = paragraph.register("ORG", "ANSA", "s1", 9, 13);

        let mut isolated = IdentityRegistry::new();
        let actual = isolated.register("ORG", "ANSA", "s1", 0, 4);

        assert_ne!(expected, actual, "local IDs remain document scoped");
        assert_eq!(
            paragraph.identities()[1].canonical_id,
            isolated.identities()[0].canonical_id
        );
        assert_eq!(isolated.identities()[0].canonical_id, "org:ansa");
    }

    #[test]
    fn composed_and_decomposed_unicode_share_one_identity() {
        let mut composed = IdentityRegistry::new();
        composed.register("PERSON", "Café", "s1", 0, 5);
        let mut decomposed = IdentityRegistry::new();
        decomposed.register("PERSON", "Cafe\u{301}", "s1", 0, 6);

        assert_eq!(
            composed.identities()[0].canonical_id,
            decomposed.identities()[0].canonical_id
        );
        assert_eq!(
            stable_surface_key("PERSON", "Café"),
            stable_surface_key("PERSON", "Cafe\u{301}")
        );
    }

    #[test]
    fn apostrophe_variants_share_one_identity() {
        for variant in [
            "O'Brien",
            "O\u{2019}Brien",
            "O\u{02bc}Brien",
            "O\u{0060}Brien",
        ] {
            assert_eq!(
                stable_surface_key("PERSON", "O'Brien"),
                stable_surface_key("PERSON", variant),
                "apostrophe variant {variant:?} must fold"
            );
        }
        assert_eq!(normalize_surface("L\u{2019}Aquila"), "l'aquila");
        assert_eq!(normalize_surface("d\u{2019}Artagnan"), "d'artagnan");
    }

    #[test]
    fn distinct_keys_that_slug_collide_keep_distinct_surface_keys() {
        let hyphenated = stable_surface_key("PERSON", "Anne-Marie");
        let spaced = stable_surface_key("PERSON", "Anne Marie");
        assert_ne!(hyphenated, spaced);
        assert!(hyphenated.starts_with("person:anne-marie-"));
        assert!(spaced.starts_with("person:anne-marie-"));
    }

    #[test]
    fn named_and_generic_kinds_get_the_right_role() {
        assert_eq!(role_of("PERSON"), EntityRole::NamedEntity);
        assert_eq!(role_of("ORG"), EntityRole::NamedEntity);
        assert_eq!(role_of("BRAND"), EntityRole::NamedEntity);
        assert_eq!(role_of("MEDIA_OUTLET"), EntityRole::NamedEntity);
        assert_eq!(role_of("VISUAL_CONCEPT"), EntityRole::VisualConcept);
        assert_eq!(role_of("OBJECT"), EntityRole::VisualConcept);
        assert_eq!(role_of("NUMBER"), EntityRole::VisualConcept);
        // The SSOT's named_kind includes the TICKET-43 additive visual types;
        // the editorial role does not. Keep the difference explicit so a future
        // "why not read named_kind?" does not silently create name cards for
        // animals, diseases or artworks.
        for kind in [
            "ANIMAL",
            "FOOD",
            "VEHICLE",
            "LAW",
            "COURT_CASE",
            "NORP",
            "DISEASE",
            "TECHNOLOGY",
            "ARTWORK",
            "CELESTIAL",
        ] {
            assert!(
                crate::entity_capabilities::entity_capabilities(kind).named_kind,
                "{kind}: the SSOT keys it as a name"
            );
            assert_eq!(
                role_of(kind),
                EntityRole::VisualConcept,
                "{kind}: a visual type is not an editorial identity"
            );
        }
        assert!(
            crate::entity_capabilities::entity_capabilities("MEDIA_OUTLET").named_kind,
            "MEDIA_OUTLET: the SSOT keys it as a name"
        );
    }

    #[test]
    fn editorial_repair_owns_ner_and_prompt_echo_policy() {
        let dna = editorial_decision("DNA di Chiara", "PERSON");
        assert_eq!(dna.kind, "CONCEPT");
        assert!(!dna.visual_admission);

        let verb = editorial_decision("  dopo  ", "LOCATION");
        assert!(verb.kind.is_empty());
        assert!(!verb.visual_admission);

        let echo = editorial_decision("Mappa Pertinente Andrea Sempio", "PERSON");
        assert!(echo.prompt_echo);
        assert!(echo.kind.is_empty());

        let acronym = editorial_decision("ANSA news agency", "ORG");
        assert!(!acronym.visual_admission);
    }
}
