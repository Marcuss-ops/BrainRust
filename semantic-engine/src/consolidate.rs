//! Semantic Consolidation v1 — the selective production view over the complete
//! semantic inventory.
//!
//! The raw manifest answers *what does the text contain?* and intentionally
//! keeps every grounded candidate. This module answers the second question the
//! Overlay Factory needs: *which compositions are worth producing, and what is
//! still uncovered?*
//!
//! Rules:
//! * every raw collection stays in the scene manifests (nothing is deleted);
//! * one canonical identity gets at most one search asset;
//! * one canonical fact gets at most one representation;
//! * one query ladder is emitted per production candidate, never per mention;
//! * every canonical fact is either represented or carries an explicit reason.
//!
//! The projection is deterministic: ordering is derived from source position,
//! importance and stable keys only.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use semantic_core::{
    ConsolidatedEventLink, ConsolidationInventory, EntityFact, FactRepresentation,
    ProductionCandidate, QueryComponent, SceneInput, SceneManifest, SearchFeedbackEntry,
    SearchQuery, SemanticGroup, VisualConsolidation, VISUAL_CONSOLIDATION_SCHEMA,
};
use serde::Deserialize;

#[derive(Debug, Clone)]
struct IdentityState {
    id: String,
    canonical: String,
    kind: String,
    mentions: Vec<MentionRef>,
    max_score: f32,
}

#[derive(Debug, Clone)]
struct MentionRef {
    scene_id: String,
    scene_index: usize,
    surface: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct TimelineGroup<'a> {
    group: &'a SemanticGroup,
    scene_index: usize,
    scene_id: String,
    covered: BTreeSet<String>,
    preference: u8,
}

#[derive(Debug, Clone)]
struct RawFact {
    scene_index: usize,
    scene_id: String,
    fact: EntityFact,
}

mod concepts;
mod identities;
mod producers;
mod stats;
mod timeline;

use concepts::{push_fact_cards, push_visual_concepts};
use identities::{canonical_facts, collect_identities, is_named_kind};
use producers::{push_charts, push_multiple_entities, push_named_entities, reference_numbers};
use stats::{canonical_micro_count, metrics, scene_rank, scene_stats};
use timeline::{canonical_dates, push_timeline, selected_timeline_groups, timeline_groups};

/// Builds the whole-script consolidation projection.
pub fn build(
    language: &str,
    sources: &[SceneInput],
    scenes: &[SceneManifest],
    search_entries: &[SearchFeedbackEntry],
) -> VisualConsolidation {
    let scene_index: HashMap<&str, usize> = sources
        .iter()
        .enumerate()
        .map(|(index, scene)| (scene.scene_id.as_str(), index))
        .collect();
    let identities = collect_identities(scenes, &scene_index);
    let (mut facts, raw_facts) = canonical_facts(scenes, &scene_index);
    let dates = canonical_dates(scenes);
    let canonical_micro_summaries = canonical_micro_count(scenes, &facts);

    let mut candidates: Vec<ProductionCandidate> = Vec::new();
    let mut fact_representations: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut event_links: Vec<ConsolidatedEventLink> = Vec::new();

    // One timeline per canonical fact set. A step sequence wins over a
    // chronology of the same evidence; overlapping groups are suppressed.
    let timeline_groups = timeline_groups(scenes, &facts);
    for group in selected_timeline_groups(timeline_groups) {
        push_timeline(
            &group,
            &facts,
            &mut candidates,
            &mut fact_representations,
            &mut event_links,
        );
    }

    let references = reference_numbers(sources, scenes);
    let reference_keys: HashSet<(String, usize, usize)> = references
        .iter()
        .map(|reference| {
            (
                reference.scene_id.clone(),
                reference.start_byte,
                reference.end_byte,
            )
        })
        .collect();

    push_charts(scenes, &identities, &reference_keys, &mut candidates);
    push_multiple_entities(scenes, &identities, &mut candidates);
    push_named_entities(&identities, &mut candidates);
    push_visual_concepts(scenes, &identities, &mut candidates);
    push_fact_cards(
        &facts,
        &identities,
        &mut fact_representations,
        &mut candidates,
    );
    enrich_contextual_search_queries(language, sources, scenes, &mut candidates);

    // Rank once, then assign stable production ids and resolve the fact
    // representations against the final candidate ids.
    candidates.sort_by(|left, right| {
        right
            .priority
            .partial_cmp(&left.priority)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                scene_rank(&scene_index, &left.scene_id)
                    .cmp(&scene_rank(&scene_index, &right.scene_id))
            })
            .then_with(|| left.start_byte.cmp(&right.start_byte))
            .then_with(|| left.dedup_key.cmp(&right.dedup_key))
    });
    candidates.dedup_by(|a, b| a.dedup_key == b.dedup_key);
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = index + 1;
        candidate.id = format!("vc_{:04}", index + 1);
    }
    let candidate_ids: HashMap<String, String> = candidates
        .iter()
        .map(|candidate| (candidate.dedup_key.clone(), candidate.id.clone()))
        .collect();
    for fact in &mut facts {
        if let Some((dedup_key, recipe)) = fact_representations.get(&fact.id) {
            if let Some(candidate_id) = candidate_ids.get(dedup_key) {
                fact.representation = Some(FactRepresentation {
                    candidate_id: candidate_id.clone(),
                    recipe: recipe.clone(),
                });
            }
        }
    }
    // A fact is covered exactly when a production candidate represents it.
    // Anything else must say why, rather than disappearing silently.
    for fact in &mut facts {
        if fact.representation.is_none() {
            fact.uncovered_reason =
                Some("no grounded production recipe was selected for this claim".to_string());
        }
    }

    let metrics = metrics(
        scenes,
        search_entries,
        &identities,
        &facts,
        &candidates,
        &references,
    );
    let scene_stats = scene_stats(scenes, search_entries, &facts, &candidates);

    VisualConsolidation {
        schema: VISUAL_CONSOLIDATION_SCHEMA.to_string(),
        inventory: ConsolidationInventory {
            entity_mentions: scenes
                .iter()
                .map(|scene| scene.entities.len())
                .sum::<usize>(),
            unique_identities: identities.len(),
            named_identities: identities
                .values()
                .filter(|identity| is_named_kind(&identity.kind))
                .count(),
            facts: raw_facts.len(),
            canonical_facts: facts.len(),
            proposals: scenes
                .iter()
                .map(|scene| scene.visual_proposals.len())
                .sum::<usize>(),
            search_entries: search_entries.len(),
            queries: search_entries
                .iter()
                .map(|entry| entry.queries.len())
                .sum::<usize>(),
            temporal_facts: scenes
                .iter()
                .map(|scene| scene.temporal_facts.len())
                .sum::<usize>(),
            canonical_dates: dates.len(),
            micro_summaries: scenes
                .iter()
                .map(|scene| scene.micro_summaries.len())
                .sum::<usize>(),
            canonical_micro_summaries,
            groups: scenes
                .iter()
                .map(|scene| scene.semantic_groups.len())
                .sum::<usize>(),
            timelines: candidates
                .iter()
                .filter(|candidate| candidate.recipe == "timeline")
                .count(),
        },
        production_candidates: candidates,
        facts,
        dates,
        reference_numbers: references,
        event_links,
        scenes: scene_stats,
        metrics,
    }
}

/// Adds one compact contextual search rung to non-person entity searches when
/// another grounded visual entity occurs in the same complete source sentence.
/// A person can combine with a non-person visual concept, while person-person
/// and other named-entity pairs require an asserted relation.
fn enrich_contextual_search_queries(
    language: &str,
    sources: &[SceneInput],
    scenes: &[SceneManifest],
    candidates: &mut [ProductionCandidate],
) {
    let policy = image_query_policy();
    let language_key = language.split(['-', '_']).next().unwrap_or(language);
    let sources_by_id: HashMap<&str, &SceneInput> = sources
        .iter()
        .map(|source| (source.scene_id.as_str(), source))
        .collect();
    let mut entities_by_sentence: HashMap<(String, usize), Vec<&semantic_core::Candidate>> =
        HashMap::new();
    let mut entities_by_span: HashMap<(String, String, usize, usize), &semantic_core::Candidate> =
        HashMap::new();
    let mut phrases_by_sentence: HashMap<(String, usize), Vec<&semantic_core::Candidate>> =
        HashMap::new();
    let mut asserted_links: HashSet<(String, String, String)> = HashSet::new();
    for scene in scenes {
        for entity in &scene.entities {
            entities_by_sentence
                .entry((scene.scene_id.clone(), entity.sentence_index))
                .or_default()
                .push(entity);
            entities_by_span.insert(
                (
                    scene.scene_id.clone(),
                    entity.id.clone(),
                    entity.start_byte,
                    entity.end_byte,
                ),
                entity,
            );
        }
        for phrase in scene.short_phrases.iter().chain(scene.long_phrases.iter()) {
            phrases_by_sentence
                .entry((scene.scene_id.clone(), phrase.sentence_index))
                .or_default()
                .push(phrase);
        }
        for fact in &scene.entity_facts {
            if fact.status == "asserted" {
                if let Some(target) = &fact.target_entity_id {
                    asserted_links.insert((
                        scene.scene_id.clone(),
                        fact.subject_entity_id.clone(),
                        target.clone(),
                    ));
                    asserted_links.insert((
                        scene.scene_id.clone(),
                        target.clone(),
                        fact.subject_entity_id.clone(),
                    ));
                }
            }
        }
    }
    for entities in entities_by_sentence.values_mut() {
        entities.sort_by_key(|entity| (entity.start_byte, entity.end_byte));
    }
    for candidate in candidates.iter_mut().filter(|candidate| {
        candidate.recipe == "entity_search"
            && candidate.requires_media
            && candidate.entity_ids.len() == 1
    }) {
        let Some(source) = sources_by_id.get(candidate.scene_id.as_str()).copied() else {
            continue;
        };
        let Some(anchor) = entities_by_span
            .get(&(
                candidate.scene_id.clone(),
                candidate.entity_ids[0].clone(),
                candidate.start_byte,
                candidate.end_byte,
            ))
            .copied()
        else {
            continue;
        };
        let sentence_key = (candidate.scene_id.clone(), anchor.sentence_index);
        let phrase = phrases_by_sentence
            .get(&sentence_key)
            .into_iter()
            .flatten()
            .filter(|phrase| {
                phrase.start_byte <= anchor.start_byte
                    && anchor.end_byte <= phrase.end_byte
                    && source
                        .text
                        .get(phrase.start_byte..phrase.end_byte)
                        .is_some_and(|text| text == phrase.text)
            })
            .min_by_key(|phrase| phrase.end_byte.saturating_sub(phrase.start_byte));
        let Some(phrase) = phrase else {
            continue;
        };
        let mut nearby: Vec<_> = entities_by_sentence
            .get(&sentence_key)
            .into_iter()
            .flatten()
            .filter(|entity| {
                let named_pair = policy.named_entity_types.contains(&anchor.kind)
                    && policy.named_entity_types.contains(&entity.kind);
                let relation_asserted = asserted_links.contains(&(
                    candidate.scene_id.clone(),
                    anchor.id.clone(),
                    entity.id.clone(),
                ));
                entity.id != anchor.id
                    && policy.visual_context_types.contains(&entity.kind)
                    && (!named_pair || relation_asserted)
                    && phrase.start_byte <= entity.start_byte
                    && entity.end_byte <= phrase.end_byte
                    && (entity.end_byte <= anchor.start_byte
                        || anchor.end_byte <= entity.start_byte)
                    && entity.start_byte.abs_diff(anchor.start_byte)
                        <= policy.max_context_distance_bytes
            })
            .collect();
        nearby.sort_by(|left, right| {
            let distance = |entity: &semantic_core::Candidate| {
                if entity.end_byte <= anchor.start_byte {
                    anchor.start_byte - entity.end_byte
                } else {
                    entity.start_byte.saturating_sub(anchor.end_byte)
                }
            };
            distance(left)
                .cmp(&distance(right))
                .then_with(|| right.score.total_cmp(&left.score))
                .then_with(|| left.start_byte.cmp(&right.start_byte))
        });
        nearby.truncate(policy.max_context_entities);
        if nearby.is_empty() {
            continue;
        }

        let mut components = vec![QueryComponent {
            kind: "entity".to_string(),
            component_type: anchor.kind.clone(),
            text: anchor.text.clone(),
            start_byte: anchor.start_byte,
            end_byte: anchor.end_byte,
        }];
        components.extend(nearby.into_iter().map(|entity| QueryComponent {
            kind: "visual_context".to_string(),
            component_type: entity.kind.clone(),
            text: entity.text.clone(),
            start_byte: entity.start_byte,
            end_byte: entity.end_byte,
        }));
        components.sort_by_key(|component| component.start_byte);
        let query = components
            .iter()
            .map(|component| component.text.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let words = query.split_whitespace().count();
        if words < policy.min_query_words
            || words > policy.max_query_words
            || query.chars().count() > policy.max_query_chars
            || candidate
                .queries
                .iter()
                .any(|prior| prior.text.eq_ignore_ascii_case(&query))
        {
            continue;
        }
        let Some(evidence) = source.text.get(phrase.start_byte..phrase.end_byte) else {
            continue;
        };
        let illustrative = !components
            .iter()
            .any(|component| component.component_type == "PERSON")
            && components.iter().any(|component| {
                policy
                    .illustrative_types
                    .contains(&component.component_type)
            });
        candidate.queries.push(SearchQuery::contextual(
            query.clone(),
            evidence.to_string(),
            phrase.start_byte,
            phrase.end_byte,
            components.clone(),
            illustrative,
        ));
        if let Some(expansion) = policy
            .semantic_expansions
            .get(language_key)
            .and_then(|by_type| by_type.get(&anchor.kind))
        {
            let context = components
                .iter()
                .filter(|component| component.kind == "visual_context")
                .map(|component| component.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            if !context.is_empty() {
                let semantic_query = expansion
                    .template
                    .replace("{anchor}", anchor.text.trim())
                    .replace("{context}", &context)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let word_count = semantic_query.split_whitespace().count();
                if word_count >= policy.min_query_words
                    && word_count <= policy.max_query_words
                    && semantic_query.chars().count() <= policy.max_query_chars
                    && expansion
                        .terms
                        .iter()
                        .all(|term| semantic_query.to_lowercase().contains(&term.to_lowercase()))
                    && !candidate
                        .queries
                        .iter()
                        .any(|prior| prior.text.eq_ignore_ascii_case(&semantic_query))
                {
                    candidate.queries.push(SearchQuery::semantic_context(
                        semantic_query,
                        evidence,
                        phrase.start_byte,
                        phrase.end_byte,
                        components,
                        format!("{language_key}.{}", anchor.kind),
                        expansion.terms.clone(),
                    ));
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct ImageQueryPolicy {
    max_context_entities: usize,
    max_context_distance_bytes: usize,
    min_query_words: usize,
    max_query_words: usize,
    max_query_chars: usize,
    visual_context_types: Vec<String>,
    named_entity_types: Vec<String>,
    illustrative_types: Vec<String>,
    semantic_expansions: BTreeMap<String, BTreeMap<String, ImageQueryExpansion>>,
}

#[derive(Debug, Deserialize)]
struct ImageQueryExpansion {
    template: String,
    terms: Vec<String>,
}

impl Default for ImageQueryPolicy {
    fn default() -> Self {
        Self {
            max_context_entities: 2,
            max_context_distance_bytes: 96,
            min_query_words: 3,
            max_query_words: 8,
            max_query_chars: 96,
            visual_context_types: vec![
                "OBJECT".into(),
                "VISUAL_CONCEPT".into(),
                "ORG".into(),
                "ORGANIZATION".into(),
                "LOCATION".into(),
                "FACILITY".into(),
            ],
            named_entity_types: vec![
                "PERSON".into(),
                "ORG".into(),
                "ORGANIZATION".into(),
                "LOCATION".into(),
                "FACILITY".into(),
                "BRAND".into(),
                "PRODUCT".into(),
                "WORK".into(),
            ],
            illustrative_types: vec!["OBJECT".into(), "VISUAL_CONCEPT".into()],
            semantic_expansions: BTreeMap::new(),
        }
    }
}

fn image_query_policy() -> &'static ImageQueryPolicy {
    static POLICY: OnceLock<ImageQueryPolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(include_str!("../config/image_query_policy.json"))
            .expect("image_query_policy.json must match the semantic query policy schema")
    })
}
