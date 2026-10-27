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
use semantic_core::{
    ConsolidatedEventLink, ConsolidationInventory, EntityFact, FactRepresentation,
    ProductionCandidate, SceneInput, SceneManifest, SearchFeedbackEntry, SearchQuery,
    SemanticGroup, VisualConsolidation, VISUAL_CONSOLIDATION_SCHEMA,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

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
    push_named_entities(scenes, &identities, &mut candidates);
    push_visual_concepts(language, scenes, &identities, &mut candidates);
    push_fact_cards(
        &facts,
        &identities,
        &mut fact_representations,
        &mut candidates,
    );
    enrich_contextual_search_queries(language, sources, scenes, &mut candidates);
    apply_visual_query_priority(language, &identities, &mut candidates);

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

/// Rank a source-grounded scene query above a bare identity lookup. The query
/// ladder is already attached by `enrich_contextual_search_queries`; this pass
/// changes only production ordering and leaves the inventory and fallback rung
/// intact.
fn apply_visual_query_priority(
    language: &str,
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut [ProductionCandidate],
) {
    let lexicon = semantic_core::lexicon::lexicon();
    let object_terms = lexicon.object_terms(language);
    for candidate in candidates
        .iter_mut()
        .filter(|candidate| candidate.recipe == "entity_search" && candidate.requires_media)
    {
        let has_contextual_query = candidate
            .queries
            .iter()
            .any(|query| query.origin == "derived_from_source_context");
        let component_count = if has_contextual_query {
            candidate
                .queries
                .iter()
                .find(|query| query.origin == "derived_from_source_context")
                .map_or(0, |query| query.components.len())
        } else {
            0
        };
        let kind = candidate
            .entity_ids
            .first()
            .and_then(|id| identities.get(id))
            .map(|identity| identity.kind.as_str())
            .unwrap_or("");
        let is_document_anchor = candidate
            .rationale
            .iter()
            .any(|reason| reason == "global_document_anchor");
        let bare_object = matches!(kind, "OBJECT" | "PRODUCT" | "FACILITY" | "VISUAL_CONCEPT")
            && is_generic_object_surface(&candidate.title, object_terms, lexicon);
        candidate.priority = visual_search_priority(
            candidate.priority,
            candidate.title.split_whitespace().count(),
            bare_object,
            component_count,
            is_document_anchor,
        );
    }
}

fn is_generic_object_surface(
    surface: &str,
    object_terms: &[String],
    lexicon: &semantic_core::lexicon::Lexicon,
) -> bool {
    let normalized = surface.to_lowercase();
    if lexicon
        .gazetteer
        .object_words
        .iter()
        .any(|term| term == &normalized)
        || object_terms.iter().any(|term| term == &normalized)
    {
        return true;
    }
    let words: Vec<String> = normalized
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_string()
        })
        .filter(|word| !word.is_empty())
        .collect();
    let has_object_head = words.iter().any(|word| {
        lexicon
            .gazetteer
            .object_words
            .iter()
            .any(|term| term == word)
            || object_terms.iter().any(|term| term == word)
            || lexicon
                .facility
                .facility_nouns
                .iter()
                .any(|term| term == word)
    });
    let unqualified_common_phrase = surface
        .split_whitespace()
        .all(|word| word.chars().next().is_none_or(char::is_lowercase));
    has_object_head && words.len() <= 3 && unqualified_common_phrase
}

fn visual_search_priority(
    priority: f32,
    query_words: usize,
    bare_object: bool,
    contextual_components: usize,
    is_document_anchor: bool,
) -> f32 {
    if contextual_components >= 2 {
        let bonus = if contextual_components >= 3 {
            0.18
        } else {
            0.12
        };
        return (priority + bonus).min(0.97);
    }
    // The document anchor is a deliberate exception: it is selected by
    // cross-scene recurrence and gazetteer evidence, not by name type alone.
    if is_document_anchor {
        return priority;
    }
    if bare_object {
        priority * 0.45
    } else if query_words == 1 {
        priority * 0.82
    } else {
        priority
    }
}

#[cfg(test)]
mod visual_query_priority_tests {
    use super::{is_generic_object_surface, visual_search_priority};

    #[test]
    fn a_grounded_composition_outranks_a_bare_identity() {
        let bare_identity = visual_search_priority(0.8, 1, false, 0, false);
        let composed = visual_search_priority(0.68, 1, false, 3, false);
        assert!(composed > bare_identity);
    }

    #[test]
    fn generic_object_search_is_penalized_but_document_anchor_is_preserved() {
        let generic_object = visual_search_priority(0.75, 1, true, 0, false);
        let topic_anchor = visual_search_priority(0.93, 1, false, 0, true);
        assert!(generic_object < 0.4);
        assert_eq!(topic_anchor, 0.93);
    }

    #[test]
    fn a_generic_facility_phrase_is_not_mistaken_for_a_qualified_subject() {
        let lexicon = semantic_core::lexicon::lexicon();
        let terms = lexicon.object_terms("en");
        assert!(is_generic_object_surface("factory work", terms, lexicon));
        assert!(!is_generic_object_surface("Ford factory", terms, lexicon));
    }
}

/// Adds one compact contextual search rung to non-person entity searches when
/// another grounded visual entity occurs in the same complete source sentence.
/// A person can combine with a non-person visual concept, while person-person
/// and other named-entity pairs require an asserted relation.
fn enrich_contextual_search_queries(
    _language: &str,
    sources: &[SceneInput],
    scenes: &[SceneManifest],
    candidates: &mut [ProductionCandidate],
) {
    let policy = image_query_policy();
    let sources_by_scene: HashMap<&str, &SceneInput> = sources
        .iter()
        .map(|source| (source.scene_id.as_str(), source))
        .collect();
    let mut entities_by_span: HashMap<(String, String, usize, usize), &semantic_core::Candidate> =
        HashMap::new();
    let mut identity_by_span: HashMap<(String, usize, usize), String> = HashMap::new();
    let mut entities_by_scene: HashMap<&str, Vec<&semantic_core::Candidate>> = HashMap::new();
    let mut contexts_by_scene: HashMap<&str, Vec<&semantic_core::ContextualImageQuery>> =
        HashMap::new();
    for scene in scenes {
        for entity in &scene.entities {
            entities_by_scene
                .entry(scene.scene_id.as_str())
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
            identity_by_span.insert(
                (scene.scene_id.clone(), entity.start_byte, entity.end_byte),
                entity.id.clone(),
            );
        }
        contexts_by_scene.insert(
            &scene.scene_id,
            scene.contextual_image_queries.iter().collect(),
        );
    }
    let searchable_entity_ids: HashSet<(String, String)> = candidates
        .iter()
        .filter(|candidate| candidate.recipe == "entity_search" && candidate.entity_ids.len() == 1)
        .map(|candidate| (candidate.scene_id.clone(), candidate.entity_ids[0].clone()))
        .collect();

    for candidate in candidates
        .iter_mut()
        .filter(|candidate| candidate.recipe == "entity_search" && candidate.entity_ids.len() == 1)
    {
        let Some(source) = sources_by_scene.get(candidate.scene_id.as_str()).copied() else {
            continue;
        };
        let Some(contexts) = contexts_by_scene.get(candidate.scene_id.as_str()) else {
            continue;
        };
        let identity = candidate.entity_ids[0].as_str();
        let anchor_start = candidate.start_byte.min(source.text.len());
        let anchor_end = candidate.end_byte.min(source.text.len());
        let sentence_start = source.text[..anchor_start]
            .rfind(['.', '!', '?', '\n'])
            .map_or(0, |index| index + 1);
        let sentence_end = source.text[anchor_end..]
            .find(['.', '!', '?', '\n'])
            .map_or(source.text.len(), |index| anchor_end + index + 1);
        let candidate_text = source
            .text
            .get(sentence_start..sentence_end)
            .unwrap_or("")
            .to_lowercase();
        let non_subject_object = policy
            .non_subject_object_terms
            .iter()
            .any(|term| candidate.title.eq_ignore_ascii_case(term));
        if non_subject_object {
            candidate.requires_media = false;
            candidate.queries.clear();
            continue;
        }
        let disconnected_meta_sentence = policy
            .link_disclaimer_cues
            .iter()
            .any(|cue| candidate_text.contains(cue.to_lowercase().as_str()));
        let explicit_comparison_context = contexts.iter().any(|context| {
            context.evidence_start_byte <= candidate.start_byte
                && candidate.start_byte <= context.evidence_end_byte
                && context.components.iter().any(|component| {
                    component.component_type == "ACTION"
                        && policy
                            .comparison_cues
                            .iter()
                            .any(|cue| component.text.eq_ignore_ascii_case(cue))
                })
        });
        if disconnected_meta_sentence && !explicit_comparison_context {
            candidate.requires_media = false;
            candidate.queries.clear();
            continue;
        }
        let context_owner = |context: &semantic_core::ContextualImageQuery| {
            context
                .components
                .iter()
                .filter_map(|component| {
                    let id = identity_by_span.get(&(
                        candidate.scene_id.clone(),
                        component.start_byte,
                        component.end_byte,
                    ))?;
                    let entity = entities_by_span.get(&(
                        candidate.scene_id.clone(),
                        id.clone(),
                        component.start_byte,
                        component.end_byte,
                    ))?;
                    Some((visual_query_owner_rank(&entity.kind), id.as_str()))
                })
                .min_by_key(|(rank, _)| *rank)
                .map(|(_, id)| id)
        };
        let effective_context_owner = |context: &semantic_core::ContextualImageQuery| {
            let preferred = context_owner(context);
            if preferred.is_some_and(|id| {
                searchable_entity_ids.contains(&(candidate.scene_id.clone(), id.to_string()))
            }) {
                return preferred;
            }
            context
                .components
                .iter()
                .filter_map(|component| {
                    let id = identity_by_span.get(&(
                        candidate.scene_id.clone(),
                        component.start_byte,
                        component.end_byte,
                    ))?;
                    if !searchable_entity_ids.contains(&(candidate.scene_id.clone(), id.clone())) {
                        return None;
                    }
                    let entity = entities_by_span.get(&(
                        candidate.scene_id.clone(),
                        id.clone(),
                        component.start_byte,
                        component.end_byte,
                    ))?;
                    Some((visual_query_owner_rank(&entity.kind), id.as_str()))
                })
                .min_by_key(|(rank, _)| *rank)
                .map(|(_, id)| id)
        };
        let owns_any_context = contexts
            .iter()
            .any(|context| effective_context_owner(context) == Some(identity));
        let covered_by_context = contexts.iter().any(|context| {
            effective_context_owner(context).is_some_and(|owner| owner != identity)
                && context.components.iter().any(|component| {
                    let component_identity = identity_by_span.get(&(
                        candidate.scene_id.clone(),
                        component.start_byte,
                        component.end_byte,
                    ));
                    component_identity.is_some_and(|id| id == identity)
                        || (component.start_byte <= candidate.start_byte
                            && candidate.end_byte <= component.end_byte)
                })
        });
        let candidate_kind = candidate
            .entity_ids
            .first()
            .and_then(|id| {
                entities_by_span
                    .iter()
                    .find_map(|((scene, entity_id, _, _), entity)| {
                        (scene == &candidate.scene_id && entity_id == id)
                            .then_some(entity.kind.as_str())
                    })
            })
            .unwrap_or("");
        let unlinked_person_in_visual_scene = candidate_kind == "PERSON"
            && !contexts.is_empty()
            && !contexts.iter().any(|context| {
                context.components.iter().any(|component| {
                    identity_by_span
                        .get(&(
                            candidate.scene_id.clone(),
                            component.start_byte,
                            component.end_byte,
                        ))
                        .is_some_and(|id| id == identity)
                })
            });
        if unlinked_person_in_visual_scene {
            candidate.requires_media = false;
            candidate.queries.clear();
            continue;
        }
        if covered_by_context && !owns_any_context {
            // One contextual event gets one provider request. Its named
            // subjects remain in the manifest, but do not each issue a
            // separate exact-name search for the same scene event.
            candidate.requires_media = false;
            candidate.queries.clear();
            continue;
        }
        let mut added = 0;
        let mut contextual_queries = Vec::new();
        for context in contexts
            .iter()
            .copied()
            .filter(|context| effective_context_owner(context) == Some(identity))
        {
            if added >= policy.max_contextual_queries_per_candidate {
                break;
            }
            let contains_anchor = context.components.iter().any(|component| {
                entities_by_span
                    .get(&(
                        candidate.scene_id.clone(),
                        candidate.entity_ids[0].clone(),
                        component.start_byte,
                        component.end_byte,
                    ))
                    .is_some_and(|entity| entity.text == component.text)
            });
            if !contains_anchor || context.components.len() < 2 {
                continue;
            }
            let mut components = context.components.clone();
            let Some(existing_location) = components
                .iter()
                .filter(|component| component.component_type == "LOCATION")
                .max_by_key(|component| component.end_byte)
            else {
                continue;
            };
            let evidence_text = source
                .text
                .get(context.evidence_start_byte..context.evidence_end_byte)
                .unwrap_or_default();
            if let Some(nested_location) = entities_by_scene
                .get(candidate.scene_id.as_str())
                .into_iter()
                .flatten()
                .filter(|entity| entity.kind == "LOCATION")
                .filter(|entity| {
                    entity.start_byte > existing_location.end_byte
                        && context.evidence_start_byte <= entity.start_byte
                        && entity.end_byte <= context.evidence_end_byte
                })
                .find(|entity| {
                    let Some(gap) = source
                        .text
                        .get(existing_location.end_byte..entity.start_byte)
                    else {
                        return false;
                    };
                    let gap = gap.to_lowercase();
                    policy
                        .location_anchor_cues
                        .iter()
                        .any(|cue| gap.ends_with(&cue.to_lowercase()))
                })
            {
                if !components.iter().any(|component| {
                    component.start_byte == nested_location.start_byte
                        && component.end_byte == nested_location.end_byte
                }) {
                    components.push(semantic_core::QueryComponent {
                        kind: "location".to_string(),
                        component_type: nested_location.kind.clone(),
                        text: nested_location.text.clone(),
                        start_byte: nested_location.start_byte,
                        end_byte: nested_location.end_byte,
                    });
                    components.sort_by_key(|component| component.start_byte);
                }
            }
            let query = components
                .iter()
                .map(|component| component.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            if query.split_whitespace().count() > policy.max_query_words
                || query.chars().count() > policy.max_query_chars
                || candidate
                    .queries
                    .iter()
                    .any(|prior| prior.text.eq_ignore_ascii_case(&query))
            {
                continue;
            }
            let evidence_valid = context.evidence_start_byte <= context.evidence_end_byte
                && evidence_text == context.evidence;
            let components_valid = components.iter().all(|component| {
                component.start_byte <= component.end_byte
                    && context.evidence_start_byte <= component.start_byte
                    && component.end_byte <= context.evidence_end_byte
                    && source
                        .text
                        .get(component.start_byte..component.end_byte)
                        .is_some_and(|text| text == component.text)
            });
            let original_query = context
                .components
                .iter()
                .map(|component| component.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            if !evidence_valid || !components_valid || original_query != context.query {
                continue;
            }
            contextual_queries.push(SearchQuery::contextual(
                query,
                context.evidence.clone(),
                context.evidence_start_byte,
                context.evidence_end_byte,
                components,
                false,
            ));
            added += 1;
        }
        if !contextual_queries.is_empty() {
            // Search the source-supported event first. The canonical identity
            // query remains the fallback if the composed search has no useful
            // hit; it must not consume the first provider request by default.
            if candidate.queries.is_empty() {
                candidate
                    .queries
                    .push(SearchQuery::literal(candidate.title.clone()));
            }
            candidate.requires_media = true;
            contextual_queries.append(&mut candidate.queries);
            candidate.queries = contextual_queries;
        }
    }
}

fn visual_query_owner_rank(entity_type: &str) -> u8 {
    match entity_type {
        "PRODUCT" => 0,
        "OBJECT" => 1,
        "FACILITY" | "STRUCTURE" => 2,
        "BRAND" | "ORG" | "ORGANIZATION" => 3,
        "PERSON" => 4,
        "LOCATION" => 5,
        _ => 6,
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct ImageQueryPolicy {
    pub(crate) max_contextual_queries_per_scene: usize,
    pub(crate) max_contextual_queries_per_candidate: usize,
    pub(crate) max_query_words: usize,
    pub(crate) max_query_chars: usize,
    pub(crate) visual_action_cues: Vec<String>,
    pub(crate) scene_reference_cues: Vec<String>,
    pub(crate) anaphoric_object_terms: Vec<String>,
    pub(crate) location_inheritance_cues: Vec<String>,
    pub(crate) visual_clause_separators: Vec<String>,
    pub(crate) linked_clause_visual_cues: Vec<String>,
    pub(crate) location_preceding_subject_cues: Vec<String>,
    pub(crate) comparison_cues: Vec<String>,
    pub(crate) link_disclaimer_cues: Vec<String>,
    pub(crate) irrealis_visual_cues: Vec<String>,
    pub(crate) passive_owner_cues: Vec<String>,
    pub(crate) non_subject_object_terms: Vec<String>,
    pub(crate) location_anchor_cues: Vec<String>,
    pub(crate) nonvisual_action_context_phrases: Vec<String>,
    pub(crate) nonvisual_concept_phrases: Vec<String>,
}

impl Default for ImageQueryPolicy {
    fn default() -> Self {
        Self {
            max_contextual_queries_per_candidate: 3,
            max_contextual_queries_per_scene: 10,
            max_query_words: 7,
            max_query_chars: 64,
            visual_action_cues: Vec::new(),
            scene_reference_cues: Vec::new(),
            anaphoric_object_terms: Vec::new(),
            location_inheritance_cues: Vec::new(),
            visual_clause_separators: Vec::new(),
            linked_clause_visual_cues: Vec::new(),
            location_preceding_subject_cues: Vec::new(),
            comparison_cues: Vec::new(),
            link_disclaimer_cues: Vec::new(),
            irrealis_visual_cues: Vec::new(),
            passive_owner_cues: Vec::new(),
            non_subject_object_terms: Vec::new(),
            location_anchor_cues: Vec::new(),
            nonvisual_action_context_phrases: Vec::new(),
            nonvisual_concept_phrases: Vec::new(),
        }
    }
}

pub(crate) fn image_query_policy() -> &'static ImageQueryPolicy {
    static POLICY: OnceLock<ImageQueryPolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(include_str!("../config/image_query_policy.json"))
            .expect("image_query_policy.json must match the semantic query policy schema")
    })
}
