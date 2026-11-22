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

#[path = "consolidate/contextual_queries.rs"]
mod contextual_queries;
pub(crate) use contextual_queries::ImageQueryPolicy;

pub(crate) fn image_query_policy() -> &'static ImageQueryPolicy {
    contextual_queries::image_query_policy()
}

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

/// The single construction point for the `entity_search` recipe: one search
/// asset per canonical identity, grounded on the anchor mention.
///
/// Two lanes emit this recipe (generic visual concepts and named identities)
/// and they used to carry the whole 24-field literal each. Keeping the shape
/// here means a new `ProductionCandidate` field is added once, not per lane,
/// and the `requires_media == queries are non-empty` invariant cannot drift
/// between them: a candidate without a query ladder has nothing to search
/// for, so it must not ask the media layer for an asset.
fn entity_search_candidate(
    identity: &IdentityState,
    anchor: &MentionRef,
    priority: f32,
    scene_ids: Vec<String>,
    queries: Vec<SearchQuery>,
    rationale: Vec<String>,
) -> ProductionCandidate {
    ProductionCandidate {
        id: String::new(),
        recipe: "entity_search".to_string(),
        rank: 0,
        priority,
        scene_id: anchor.scene_id.clone(),
        scene_ids,
        entity_ids: vec![identity.id.clone()],
        fact_ids: Vec::new(),
        group_ids: Vec::new(),
        title: identity.canonical.clone(),
        evidence: anchor.surface.clone(),
        start_byte: anchor.start,
        end_byte: anchor.end,
        suggested_visual: "search_asset".to_string(),
        chart_items: Vec::new(),
        chart_meta: None,
        visual_payload: None,
        temporal_surfaces: Vec::new(),
        asset_key: format!("entity:{}", identity.id),
        requires_media: !queries.is_empty(),
        dedup_key: format!("entity:{}:search", identity.id),
        covered_fact_ids: Vec::new(),
        queries,
        rationale,
    }
}

#[derive(Debug, Clone)]
struct TimelineGroup<'a> {
    group: &'a SemanticGroup,
    scene_index: usize,
    scene_id: String,
    covered: BTreeSet<String>,
    date_keys: BTreeSet<String>,
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
mod recipes;
mod stats;
mod timeline;

use concepts::{push_fact_cards, push_visual_concepts};
use identities::{canonical_facts, collect_identities, is_named_kind};
use producers::{
    push_charts, push_multiple_entities, push_named_entities, push_ranking_bars, reference_numbers,
};
use recipes::push_visual_recipes;
use stats::{canonical_micro_count, metrics, scene_rank, scene_stats};
use timeline::{canonical_dates, push_timeline, selected_timeline_groups, timeline_groups};

/// Builds the whole-script consolidation projection.
///
/// `scene_query_seeds` carries the visual-context query seeds per scene. They
/// feed `contextual_search` production candidates but are not part of the
/// manifest contract; a scene without an entry falls back to its manifest
/// `contextual_image_queries` so callers that predate the split keep their
/// behavior.
pub fn build(
    language: &str,
    sources: &[SceneInput],
    scenes: &[SceneManifest],
    events: &[semantic_core::EventFrame],
    search_entries: &[SearchFeedbackEntry],
    scene_query_seeds: &[(String, Vec<semantic_core::ContextualImageQuery>)],
) -> VisualConsolidation {
    let query_seeds: HashMap<&str, &[semantic_core::ContextualImageQuery]> = scene_query_seeds
        .iter()
        .map(|(scene_id, seeds)| (scene_id.as_str(), seeds.as_slice()))
        .collect();
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
    push_ranking_bars(sources, scenes, &mut candidates);
    push_visual_recipes(sources, scenes, events, &identities, &mut candidates);
    push_multiple_entities(scenes, &identities, &mut candidates);
    push_named_entities(scenes, &identities, &mut candidates);
    push_visual_concepts(language, scenes, &identities, &mut candidates);
    push_fact_cards(
        &facts,
        &identities,
        &mut fact_representations,
        &mut candidates,
    );
    contextual_queries::enrich_contextual_search_queries(
        language,
        sources,
        scenes,
        &query_seeds,
        &mut candidates,
    );
    contextual_queries::push_contextual_search_candidates(
        language,
        sources,
        scenes,
        &query_seeds,
        &identities,
        &mut candidates,
    );
    contextual_queries::apply_visual_query_priority(language, &identities, &mut candidates);

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

#[cfg(test)]
#[path = "consolidate_tests.rs"]
mod visual_query_priority_tests;
