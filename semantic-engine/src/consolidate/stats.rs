//! Consolidation metrics and per-scene statistics.

use std::collections::{BTreeMap, HashMap, HashSet};

use semantic_core::{
    ConsolidatedFact, ConsolidationMetrics, ConsolidationSceneStats, ProductionCandidate,
    ReferenceNumber, SceneManifest, SearchFeedbackEntry,
};

pub(super) fn canonical_micro_count(scenes: &[SceneManifest], facts: &[ConsolidatedFact]) -> usize {
    let raw_to_canonical: HashMap<&str, &str> = facts
        .iter()
        .flat_map(|fact| {
            fact.fact_ids
                .iter()
                .map(move |raw_id| (raw_id.as_str(), fact.id.as_str()))
        })
        .collect();
    let mut seen = HashSet::new();
    for scene in scenes {
        for summary in &scene.micro_summaries {
            let mut canonical: Vec<&str> = summary
                .evidence_fact_ids
                .iter()
                .filter_map(|raw_id| raw_to_canonical.get(raw_id.as_str()).copied())
                .collect();
            canonical.sort_unstable();
            canonical.dedup();
            // Deduplicate by the claim, not by its phrasing: subject,
            // predicate, value and period are already encoded in the canonical
            // fact ids and the normalized date. "Andrea Sempio: indagato" and
            // "Sempio: indagato" in two scenes are one canonical summary. Only
            // a summary without fact evidence falls back to its own text.
            let key = if canonical.is_empty() {
                format!(
                    "text|{}|{}|{}",
                    summary.subject_entity_id.as_deref().unwrap_or(""),
                    summary.normalized_date.as_deref().unwrap_or(""),
                    summary.text.to_lowercase()
                )
            } else {
                format!(
                    "{}|{}|{}",
                    summary.subject_entity_id.as_deref().unwrap_or(""),
                    summary.normalized_date.as_deref().unwrap_or(""),
                    canonical.join(",")
                )
            };
            seen.insert(key);
        }
    }
    seen.len()
}

pub(super) fn scene_rank(index: &HashMap<&str, usize>, scene_id: &str) -> usize {
    index.get(scene_id).copied().unwrap_or(usize::MAX)
}

pub(super) fn metrics(
    scenes: &[SceneManifest],
    search_entries: &[SearchFeedbackEntry],
    identities: &BTreeMap<String, IdentityState>,
    facts: &[ConsolidatedFact],
    candidates: &[ProductionCandidate],
    references: &[ReferenceNumber],
) -> ConsolidationMetrics {
    let raw_proposals = scenes
        .iter()
        .map(|scene| scene.visual_proposals.len())
        .sum::<usize>();
    let raw_queries = search_entries
        .iter()
        .map(|entry| entry.queries.len())
        .sum::<usize>();
    let production_queries = candidates
        .iter()
        .map(|candidate| candidate.queries.len())
        .sum::<usize>();
    let media_keys: HashSet<&str> = candidates
        .iter()
        .filter(|candidate| candidate.requires_media)
        .map(|candidate| candidate.asset_key.as_str())
        .collect();
    let shared_asset_reuses = candidates
        .iter()
        .filter(|candidate| {
            !candidate.requires_media && media_keys.contains(candidate.asset_key.as_str())
        })
        .count();
    let covered = facts
        .iter()
        .filter(|fact| fact.representation.is_some())
        .count();
    ConsolidationMetrics {
        raw_proposals,
        production_candidates: candidates.len(),
        raw_search_entries: search_entries.len(),
        production_search_candidates: candidates
            .iter()
            .filter(|candidate| candidate.requires_media)
            .count(),
        raw_queries,
        production_queries,
        duplicate_entity_mentions_collapsed: scenes
            .iter()
            .map(|scene| scene.entities.len())
            .sum::<usize>()
            .saturating_sub(identities.len()),
        duplicate_facts_merged: scenes
            .iter()
            .map(|scene| scene.entity_facts.len())
            .sum::<usize>()
            .saturating_sub(facts.len()),
        duplicate_queries_removed: raw_queries.saturating_sub(production_queries),
        duplicate_proposals_removed: raw_proposals.saturating_sub(candidates.len()),
        estimated_search_jobs: media_keys.len(),
        shared_asset_reuses,
        canonical_facts: facts.len(),
        covered_facts: covered,
        uncovered_facts: facts.len().saturating_sub(covered),
        reference_numbers: references.len(),
    }
}

pub(super) fn scene_stats(
    scenes: &[SceneManifest],
    search_entries: &[SearchFeedbackEntry],
    facts: &[ConsolidatedFact],
    candidates: &[ProductionCandidate],
) -> Vec<ConsolidationSceneStats> {
    let mut out = Vec::new();
    for scene in scenes {
        let raw_queries = search_entries
            .iter()
            .filter(|entry| entry.scene_id == scene.scene_id)
            .map(|entry| entry.queries.len())
            .sum::<usize>();
        let scene_facts: Vec<&ConsolidatedFact> = facts
            .iter()
            .filter(|fact| fact.scene_ids.iter().any(|id| id == &scene.scene_id))
            .collect();
        let covered = scene_facts
            .iter()
            .filter(|fact| fact.representation.is_some())
            .count();
        let production = candidates
            .iter()
            .filter(|candidate| candidate.scene_id == scene.scene_id)
            .count();
        let timelines = candidates
            .iter()
            .filter(|candidate| {
                candidate.scene_id == scene.scene_id && candidate.recipe == "timeline"
            })
            .count();
        out.push(ConsolidationSceneStats {
            scene_id: scene.scene_id.clone(),
            raw_proposals: scene.visual_proposals.len(),
            production_candidates: production,
            raw_queries,
            canonical_facts: scene_facts.len(),
            covered_facts: covered,
            timeline_candidates: timelines,
        });
    }
    out
}

use super::IdentityState;
