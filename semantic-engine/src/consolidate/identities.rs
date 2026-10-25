//! Canonical identity and fact consolidation: mention grouping, canonical
//! fact keys and the identity ranking shared by every producer.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use semantic_core::{ConsolidatedFact, EntityFact, SceneManifest};

use super::{IdentityState, MentionRef, RawFact};

pub(super) fn collect_identities(
    scenes: &[SceneManifest],
    scene_index: &HashMap<&str, usize>,
) -> BTreeMap<String, IdentityState> {
    let mut identities: BTreeMap<String, IdentityState> = BTreeMap::new();
    for scene in scenes {
        for identity in &scene.identities {
            let entry = identities
                .entry(identity.id.clone())
                .or_insert_with(|| IdentityState {
                    id: identity.id.clone(),
                    canonical: identity.canonical_surface.clone(),
                    kind: identity.kind.clone(),
                    mentions: Vec::new(),
                    max_score: 0.0,
                });
            for mention in &identity.mentions {
                let index = scene_index
                    .get(mention.scene_id.as_str())
                    .copied()
                    .unwrap_or(0);
                if entry.mentions.iter().any(|existing| {
                    existing.scene_id == mention.scene_id
                        && existing.start == mention.start_byte
                        && existing.end == mention.end_byte
                }) {
                    continue;
                }
                entry.mentions.push(MentionRef {
                    scene_id: mention.scene_id.clone(),
                    scene_index: index,
                    surface: mention.surface.clone(),
                    start: mention.start_byte,
                    end: mention.end_byte,
                });
            }
        }
        for entity in &scene.entities {
            if let Some(entry) = identities.get_mut(&entity.id) {
                entry.max_score = entry.max_score.max(entity.score);
            }
        }
    }
    for identity in identities.values_mut() {
        identity.mentions.sort_by(|left, right| {
            left.scene_index
                .cmp(&right.scene_index)
                .then_with(|| left.start.cmp(&right.start))
                .then_with(|| left.end.cmp(&right.end))
        });
    }
    identities
}

pub(super) fn canonical_facts(
    scenes: &[SceneManifest],
    scene_index: &HashMap<&str, usize>,
) -> (Vec<ConsolidatedFact>, Vec<RawFact>) {
    let mut raw_facts = Vec::new();
    let mut grouped: BTreeMap<String, Vec<RawFact>> = BTreeMap::new();
    for scene in scenes {
        let index = scene_index
            .get(scene.scene_id.as_str())
            .copied()
            .unwrap_or(0);
        for fact in &scene.entity_facts {
            let raw = RawFact {
                scene_index: index,
                scene_id: scene.scene_id.clone(),
                fact: fact.clone(),
            };
            grouped.entry(fact_key(fact)).or_default().push(raw.clone());
            raw_facts.push(raw);
        }
    }
    let mut facts: Vec<ConsolidatedFact> = grouped
        .into_iter()
        .map(|(key, mut group)| {
            group.sort_by(|left, right| {
                left.scene_index
                    .cmp(&right.scene_index)
                    .then_with(|| left.fact.start_byte.cmp(&right.fact.start_byte))
            });
            let first = &group[0];
            let mut scene_ids = BTreeSet::new();
            let mut fact_ids = Vec::new();
            let mut importance: f32 = 0.0;
            for raw in &group {
                scene_ids.insert(raw.scene_id.clone());
                fact_ids.push(raw.fact.id.clone());
                importance = importance.max(raw.fact.importance);
            }
            ConsolidatedFact {
                id: String::new(),
                fact_key: key,
                fact_ids,
                subject_entity_id: first.fact.subject_entity_id.clone(),
                predicate: first.fact.predicate.clone(),
                normalized_value: first.fact.normalized_value.clone(),
                normalized_date: first.fact.normalized_date.clone(),
                status: first.fact.status.clone(),
                importance,
                scene_ids: scene_ids.into_iter().collect(),
                sentence_index: first.fact.sentence_index,
                evidence: first.fact.evidence.clone(),
                start_byte: first.fact.start_byte,
                end_byte: first.fact.end_byte,
                evidence_start_byte: first.fact.evidence_start_byte,
                evidence_end_byte: first.fact.evidence_end_byte,
                representation: None,
                uncovered_reason: None,
            }
        })
        .collect();
    facts.sort_by(|left, right| {
        right
            .importance
            .partial_cmp(&left.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.scene_ids.first().cmp(&right.scene_ids.first()))
            .then_with(|| left.start_byte.cmp(&right.start_byte))
            .then_with(|| left.fact_key.cmp(&right.fact_key))
    });
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.id = format!("cf_{:04}", index + 1);
    }
    (facts, raw_facts)
}

fn fact_key(fact: &EntityFact) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        fact.subject_entity_id,
        fact.predicate,
        fact.normalized_value
            .clone()
            .unwrap_or_else(|| fact.value.to_lowercase()),
        fact.normalized_date.clone().unwrap_or_default(),
        fact.status,
        fact.target_entity_id.clone().unwrap_or_default()
    )
}

/// Pick the mention that best anchors one identity. Prefer the canonical full
/// surface when the script states it (so the query is both precise and an exact
/// slice of the anchor scene); otherwise use the longest surface, then source
/// order. This keeps every production query grounded in the scene it points at.
pub(super) fn identity_anchor(identity: &IdentityState) -> Option<&MentionRef> {
    identity
        .mentions
        .iter()
        .filter(|mention| mention.surface == identity.canonical)
        .min_by(|left, right| {
            left.scene_index
                .cmp(&right.scene_index)
                .then_with(|| left.start.cmp(&right.start))
        })
        .or_else(|| {
            identity.mentions.iter().max_by(|left, right| {
                left.surface
                    .chars()
                    .count()
                    .cmp(&right.surface.chars().count())
                    .then_with(|| right.scene_index.cmp(&left.scene_index))
                    .then_with(|| right.start.cmp(&left.start))
            })
        })
}

pub(super) fn is_named_kind(kind: &str) -> bool {
    matches!(
        kind,
        "PERSON"
            | "ORG"
            | "ORGANIZATION"
            | "LOCATION"
            | "FACILITY"
            | "OBJECT"
            | "BRAND"
            | "PRODUCT"
            | "WORK"
            | "EVENT"
            | "ROLE"
    )
}

pub(super) fn named_priority(kind: &str, mentions: usize, score: f32) -> f32 {
    let base = match kind {
        "PERSON" => 0.95,
        "ORG" | "ORGANIZATION" => 0.86,
        "LOCATION" => 0.82,
        "FACILITY" => 0.78,
        "OBJECT" => 0.7,
        "ROLE" => 0.72,
        _ => 0.68,
    };
    (base + (mentions.min(8) as f32 * 0.01) + score.min(0.1)).min(0.99)
}
