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

pub(super) fn named_priority(
    kind: &str,
    mentions: usize,
    score: f32,
    asserted_relations: usize,
    asserted_facts: usize,
) -> f32 {
    // A weak one-off or lightly repeated extraction is inventory, not a
    // production image search. Recurrence or an asserted link can rescue a
    // low-confidence mention; type alone cannot.
    if score < 0.5 && mentions < 5 && asserted_relations == 0 && asserted_facts == 0 {
        return 0.0;
    }
    let base = match kind {
        // Type is only a small visual prior. Recurrence and asserted links
        // carry more weight, so an incidental person does not outrank the
        // place or facility the script is actually about.
        "PERSON" => 0.52,
        "ORG" | "ORGANIZATION" => 0.58,
        "LOCATION" => 0.64,
        "FACILITY" => 0.68,
        "OBJECT" | "PRODUCT" | "BRAND" => 0.62,
        "ROLE" => 0.5,
        "EVENT" | "WORK" => 0.6,
        _ => 0.55,
    };
    let recurrence = (mentions.saturating_sub(1).min(8) as f32) * 0.018;
    let extraction_support = score.clamp(0.0, 1.0) * 0.05;
    let relation_support = (asserted_relations.min(3) as f32) * 0.045;
    let fact_support = (asserted_facts.min(4) as f32) * 0.035;
    (base + recurrence + extraction_support + relation_support + fact_support).min(0.95)
}

#[cfg(test)]
mod priority_tests {
    use super::named_priority;

    #[test]
    fn an_incidental_person_does_not_outrank_a_central_place_or_facility() {
        let incidental_person = named_priority("PERSON", 1, 1.0, 0, 0);
        let recurring_location = named_priority("LOCATION", 4, 0.6, 0, 0);
        let facility = named_priority("FACILITY", 1, 0.7, 0, 0);

        assert!(recurring_location > incidental_person);
        assert!(facility > incidental_person);
    }

    #[test]
    fn explicit_relations_and_facts_raise_a_recurrent_subject() {
        let isolated = named_priority("PERSON", 1, 0.8, 0, 0);
        let grounded = named_priority("PERSON", 3, 0.8, 2, 2);

        assert!(grounded > isolated);
        assert!(grounded <= 0.95);
    }

    #[test]
    fn weak_infrequent_identity_stays_in_inventory_without_becoming_a_search() {
        assert_eq!(named_priority("LOCATION", 2, 0.3, 0, 0), 0.0);
        assert!(named_priority("LOCATION", 5, 0.3, 0, 0) > 0.0);
        assert!(named_priority("LOCATION", 1, 0.3, 1, 0) > 0.0);
    }
}
