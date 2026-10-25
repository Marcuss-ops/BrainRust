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

use semantic_core::{
    Candidate, ChartItem, ConsolidatedDate, ConsolidatedDateSpan, ConsolidatedEventLink,
    ConsolidatedFact, ConsolidationInventory, ConsolidationMetrics, ConsolidationSceneStats,
    EntityFact, FactRepresentation, ProductionCandidate, ReferenceNumber, SceneInput,
    SceneManifest, SearchFeedbackEntry, SearchQuery, SemanticGroup, VisualConsolidation,
    VISUAL_CONSOLIDATION_SCHEMA,
};

/// Maximum generic visual concepts kept per scene. Named identities are never
/// capped; this bound only stops thousands of generic nouns from becoming
/// production candidates. The complete list stays in `important_words`.
const MAX_GENERIC_CONCEPTS_PER_SCENE: usize = 4;

/// Below this deterministic specificity score a generic phrase is inventory
/// only. A scene whose nouns are all abstract gets no stock-image search
/// instead of four weak ones; the planner can still use the inventory.
const MIN_CONCEPT_PRIORITY: f32 = 0.45;

/// High-frequency abstract nouns that are poor image queries. They are not
/// removed from the inventory; they are down-ranked when the per-scene concept
/// budget is applied.
const GENERIC_VISUAL_NOUNS: &[&str] = &[
    "atti",
    "caso",
    "casa",
    "centro",
    "compito",
    "conclusione",
    "contesto",
    "cronaca",
    "data",
    "decisione",
    "differenza",
    "dolore",
    "diritto",
    "elementi",
    "fase",
    "fatto",
    "fatti",
    "indagine",
    "innocenza",
    "linguaggio",
    "luoghi",
    "merito",
    "modo",
    "motivazioni",
    "oggetti",
    "parole",
    "parte",
    "parti",
    "passaggio",
    "persona",
    "posizione",
    "presenza",
    "procedimento",
    "prova",
    "pubblico",
    "qualità",
    "racconto",
    "reclusione",
    "responsabilità",
    "resto",
    "ricostruzioni",
    "rigore",
    "rispetto",
    "sede",
    "sequenza",
    "tempi",
    "tempo",
    "testimonianze",
    "verità",
    "vicenda",
    "violenza",
    "vita",
];

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

/// Builds the whole-script consolidation projection.
pub fn build(
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

fn collect_identities(
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

fn canonical_facts(
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

fn canonical_dates(scenes: &[SceneManifest]) -> Vec<ConsolidatedDate> {
    let mut grouped: BTreeMap<String, Vec<ConsolidatedDateSpan>> = BTreeMap::new();
    for scene in scenes {
        for fact in &scene.temporal_facts {
            let Some(normalized) = fact.normalized.as_ref() else {
                continue;
            };
            let spans = grouped.entry(normalized.clone()).or_default();
            if spans.iter().any(|span| {
                span.scene_id == scene.scene_id
                    && span.start_byte == fact.start_byte
                    && span.end_byte == fact.end_byte
            }) {
                continue;
            }
            spans.push(ConsolidatedDateSpan {
                scene_id: scene.scene_id.clone(),
                surface: fact.surface.clone(),
                start_byte: fact.start_byte,
                end_byte: fact.end_byte,
            });
        }
    }
    grouped
        .into_iter()
        .map(|(normalized_date, mut spans)| {
            spans.sort_by(|left, right| {
                left.scene_id
                    .cmp(&right.scene_id)
                    .then_with(|| left.start_byte.cmp(&right.start_byte))
            });
            let mut surfaces = Vec::new();
            for span in &spans {
                if !surfaces.contains(&span.surface) {
                    surfaces.push(span.surface.clone());
                }
            }
            ConsolidatedDate {
                normalized_date,
                count: spans.len(),
                surfaces,
                spans,
            }
        })
        .collect()
}

fn canonical_micro_count(scenes: &[SceneManifest], facts: &[ConsolidatedFact]) -> usize {
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

fn timeline_groups<'a>(
    scenes: &'a [SceneManifest],
    facts: &[ConsolidatedFact],
) -> Vec<TimelineGroup<'a>> {
    let mut groups = Vec::new();
    for (scene_index, scene) in scenes.iter().enumerate() {
        for group in &scene.semantic_groups {
            let preference = match group.group_type.as_str() {
                "step_sequence" => 3,
                "chronological" => 2,
                "timeline_summary" => 1,
                _ => continue,
            };
            if group.members.len() < 2 {
                continue;
            }
            let mut covered = BTreeSet::new();
            for fact in facts {
                if fact.normalized_date.is_none() {
                    continue;
                }
                if !fact
                    .scene_ids
                    .iter()
                    .any(|scene_id| scene_id == &scene.scene_id)
                {
                    continue;
                }
                if group.members.iter().any(|member| {
                    member.scene_id == scene.scene_id
                        && member.sentence_index == fact.sentence_index
                }) {
                    covered.insert(fact.id.clone());
                }
            }
            if covered.is_empty() {
                continue;
            }
            groups.push(TimelineGroup {
                group,
                scene_index,
                scene_id: scene.scene_id.clone(),
                covered,
                preference,
            });
        }
    }
    groups
}

fn selected_timeline_groups(mut groups: Vec<TimelineGroup<'_>>) -> Vec<TimelineGroup<'_>> {
    groups.sort_by(|left, right| {
        right
            .preference
            .cmp(&left.preference)
            .then_with(|| right.covered.len().cmp(&left.covered.len()))
            .then_with(|| left.scene_index.cmp(&right.scene_index))
            .then_with(|| left.group.id.cmp(&right.group.id))
    });
    let mut selected: Vec<TimelineGroup<'_>> = Vec::new();
    for group in groups {
        if selected.iter().any(|existing| {
            existing
                .covered
                .iter()
                .any(|fact| group.covered.contains(fact))
        }) {
            continue;
        }
        selected.push(group);
    }
    selected
}

fn push_timeline(
    group: &TimelineGroup<'_>,
    facts: &[ConsolidatedFact],
    candidates: &mut Vec<ProductionCandidate>,
    representations: &mut BTreeMap<String, (String, String)>,
    event_links: &mut Vec<ConsolidatedEventLink>,
) {
    let Some(anchor) = group
        .group
        .members
        .iter()
        .min_by_key(|member| member.start_byte)
    else {
        return;
    };
    let covered: Vec<String> = group.covered.iter().cloned().collect();
    let mut entity_ids = BTreeSet::new();
    for fact in facts.iter().filter(|fact| group.covered.contains(&fact.id)) {
        entity_ids.insert(fact.subject_entity_id.clone());
        representations.insert(
            fact.id.clone(),
            (timeline_dedup_key(&covered), "timeline".to_string()),
        );
    }
    let mut temporal_surfaces = Vec::new();
    for member in &group.group.members {
        if let Some(surface) = member.temporal_surface.as_ref() {
            if !temporal_surfaces.contains(surface) {
                temporal_surfaces.push(surface.clone());
            }
        }
    }
    let priority = (0.92 + (group.group.members.len() as f32 * 0.01)).min(0.99);
    candidates.push(ProductionCandidate {
        id: String::new(),
        recipe: "timeline".to_string(),
        rank: 0,
        priority,
        scene_id: group.scene_id.clone(),
        scene_ids: vec![group.scene_id.clone()],
        entity_ids: entity_ids.into_iter().collect(),
        fact_ids: covered.clone(),
        group_ids: vec![group.group.id.clone()],
        title: group.group.title.clone(),
        evidence: anchor.evidence.clone(),
        start_byte: anchor.start_byte,
        end_byte: anchor.end_byte,
        suggested_visual: "timeline".to_string(),
        chart_items: Vec::new(),
        temporal_surfaces,
        asset_key: format!("timeline:{}", group.group.id),
        queries: Vec::new(),
        requires_media: false,
        dedup_key: timeline_dedup_key(&covered),
        covered_fact_ids: covered.clone(),
        rationale: vec![
            format!(
                "one {} composition represents {} canonical facts",
                group.group.group_type,
                covered.len()
            ),
            "duplicate or nested timelines over the same facts are suppressed".to_string(),
        ],
    });
    for members in group.group.members.windows(2) {
        let from = &members[0];
        let to = &members[1];
        if from.candidate_id.is_empty() || to.candidate_id.is_empty() {
            continue;
        }
        event_links.push(ConsolidatedEventLink {
            id: format!("el_{:04}", event_links.len() + 1),
            group_id: group.group.id.clone(),
            scene_id: to.scene_id.clone(),
            from_candidate_id: from.candidate_id.clone(),
            to_candidate_id: to.candidate_id.clone(),
            kind: "precedes".to_string(),
            evidence: to.evidence.clone(),
            start_byte: to.start_byte,
            end_byte: to.end_byte,
        });
    }
}

fn timeline_dedup_key(covered: &[String]) -> String {
    format!("timeline:{}", covered.join(","))
}

fn reference_numbers(sources: &[SceneInput], scenes: &[SceneManifest]) -> Vec<ReferenceNumber> {
    let texts: HashMap<&str, &str> = sources
        .iter()
        .map(|scene| (scene.scene_id.as_str(), scene.text.as_str()))
        .collect();
    let mut out = Vec::new();
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        for fact in &scene.numeric_facts {
            if fact.kind != "NUMBER" {
                continue;
            }
            let Some(prefix) = text.get(..fact.start_byte) else {
                continue;
            };
            let tail: String = prefix
                .chars()
                .rev()
                .take(24)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let tail = tail.trim_end().to_lowercase();
            let is_reference = tail.ends_with("numero")
                || tail.ends_with("n.")
                || tail.ends_with("n°")
                || tail.ends_with("n.");
            if !is_reference {
                continue;
            }
            out.push(ReferenceNumber {
                scene_id: scene.scene_id.clone(),
                surface: fact.surface.clone(),
                reason: "explicit reference number, not a quantitative value".to_string(),
                start_byte: fact.start_byte,
                end_byte: fact.end_byte,
            });
        }
    }
    out
}

fn push_charts(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    references: &HashSet<(String, usize, usize)>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    let mut seen = HashSet::new();
    for scene in scenes {
        for proposal in &scene.visual_proposals {
            if proposal.proposal_type != "chart" {
                continue;
            }
            let items: Vec<ChartItem> = proposal
                .chart_items
                .iter()
                .filter(|item| {
                    !references.contains(&(scene.scene_id.clone(), item.start_byte, item.end_byte))
                })
                .cloned()
                .collect();
            if items.is_empty() {
                continue;
            }
            let mut spans: Vec<String> = items
                .iter()
                .map(|item| format!("{}:{}:{}", item.kind, item.start_byte, item.end_byte))
                .collect();
            spans.sort();
            let key = format!("chart:{}:{}", scene.scene_id, spans.join(","));
            if !seen.insert(key.clone()) {
                continue;
            }
            let entity_ids: Vec<String> = proposal
                .entity_ids
                .iter()
                .filter(|id| identities.contains_key(*id))
                .cloned()
                .collect();
            candidates.push(ProductionCandidate {
                id: String::new(),
                recipe: "chart".to_string(),
                rank: 0,
                priority: 0.88,
                scene_id: scene.scene_id.clone(),
                scene_ids: vec![scene.scene_id.clone()],
                entity_ids,
                fact_ids: Vec::new(),
                group_ids: Vec::new(),
                title: "Confronto dati".to_string(),
                evidence: proposal.evidence.clone(),
                start_byte: proposal.start_byte,
                end_byte: proposal.end_byte,
                suggested_visual: proposal.suggested_visual.clone(),
                chart_items: items,
                temporal_surfaces: Vec::new(),
                asset_key: key.clone(),
                queries: Vec::new(),
                requires_media: false,
                dedup_key: key,
                covered_fact_ids: Vec::new(),
                rationale: vec![
                    "one grounded comparison for the whole value set".to_string(),
                    "reference numbers are excluded from quantitative charts".to_string(),
                ],
            });
        }
    }
}

fn push_multiple_entities(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    let mut seen = HashSet::new();
    for scene in scenes {
        for proposal in &scene.visual_proposals {
            if proposal.proposal_type != "multiple_entities" {
                continue;
            }
            let mut ids: Vec<String> = proposal
                .entity_ids
                .iter()
                .filter(|id| identities.contains_key(*id))
                .cloned()
                .collect();
            ids.sort();
            ids.dedup();
            if ids.len() < 2 {
                continue;
            }
            let key = format!("multi:{}", ids.join(","));
            if !seen.insert(key.clone()) {
                continue;
            }
            let title = ids
                .iter()
                .map(|id| {
                    identities
                        .get(id)
                        .map(|identity| identity.canonical.as_str())
                        .unwrap_or(id.as_str())
                })
                .collect::<Vec<_>>()
                .join(" · ");
            candidates.push(ProductionCandidate {
                id: String::new(),
                recipe: "multiple_entities".to_string(),
                rank: 0,
                priority: (0.64 + ids.len() as f32 * 0.03).min(0.8),
                scene_id: scene.scene_id.clone(),
                scene_ids: vec![scene.scene_id.clone()],
                entity_ids: ids.clone(),
                fact_ids: Vec::new(),
                group_ids: Vec::new(),
                title,
                evidence: proposal.evidence.clone(),
                start_byte: proposal.start_byte,
                end_byte: proposal.end_byte,
                suggested_visual: proposal.suggested_visual.clone(),
                chart_items: Vec::new(),
                temporal_surfaces: Vec::new(),
                asset_key: key.clone(),
                queries: Vec::new(),
                requires_media: false,
                dedup_key: key,
                covered_fact_ids: Vec::new(),
                rationale: vec![
                    "one composition reuses the individual entity assets".to_string(),
                    "duplicate co-mention sets are collapsed".to_string(),
                ],
            });
        }
    }
}

fn push_named_entities(
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for identity in identities.values() {
        if !is_named_kind(&identity.kind) {
            continue;
        }
        let Some(anchor) = identity_anchor(identity) else {
            continue;
        };
        let priority = named_priority(&identity.kind, identity.mentions.len(), identity.max_score);
        let mut scene_ids = BTreeSet::new();
        for mention in &identity.mentions {
            scene_ids.insert(mention.scene_id.clone());
        }
        candidates.push(ProductionCandidate {
            id: String::new(),
            recipe: "entity_search".to_string(),
            rank: 0,
            priority,
            scene_id: anchor.scene_id.clone(),
            scene_ids: scene_ids.iter().cloned().collect(),
            entity_ids: vec![identity.id.clone()],
            fact_ids: Vec::new(),
            group_ids: Vec::new(),
            title: identity.canonical.clone(),
            evidence: anchor.surface.clone(),
            start_byte: anchor.start,
            end_byte: anchor.end,
            suggested_visual: "search_asset".to_string(),
            chart_items: Vec::new(),
            temporal_surfaces: Vec::new(),
            asset_key: format!("entity:{}", identity.id),
            queries: vec![SearchQuery {
                kind: "exact_mention".to_string(),
                text: anchor.surface.clone(),
            }],
            requires_media: true,
            dedup_key: format!("entity:{}:search", identity.id),
            covered_fact_ids: Vec::new(),
            rationale: vec![
                format!(
                    "{} named identity collapsed from {} mentions",
                    identity.kind,
                    identity.mentions.len()
                ),
                "one asset search per canonical identity".to_string(),
            ],
        });
        candidates.push(ProductionCandidate {
            id: String::new(),
            recipe: "entity_card".to_string(),
            rank: 0,
            priority: (priority - 0.12).max(0.2),
            scene_id: anchor.scene_id.clone(),
            scene_ids: scene_ids.into_iter().collect(),
            entity_ids: vec![identity.id.clone()],
            fact_ids: Vec::new(),
            group_ids: Vec::new(),
            title: identity.canonical.clone(),
            evidence: anchor.surface.clone(),
            start_byte: anchor.start,
            end_byte: anchor.end,
            suggested_visual: "entity_card".to_string(),
            chart_items: Vec::new(),
            temporal_surfaces: Vec::new(),
            asset_key: format!("entity:{}", identity.id),
            queries: Vec::new(),
            requires_media: false,
            dedup_key: format!("entity:{}:card", identity.id),
            covered_fact_ids: Vec::new(),
            rationale: vec![
                "card reuses the entity search asset".to_string(),
                "no second external asset search".to_string(),
            ],
        });
    }
}

fn push_visual_concepts(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    // The complete concept inventory stays in `important_words`; only the
    // per-scene best few enter production.
    let mut selected: BTreeSet<String> = BTreeSet::new();
    for scene in scenes {
        let named_spans: Vec<(usize, usize)> = scene
            .entities
            .iter()
            .filter(|entity| is_named_kind(&entity.kind))
            .map(|entity| (entity.start_byte, entity.end_byte))
            .collect();
        let mut concepts: Vec<&Candidate> = scene
            .entities
            .iter()
            .filter(|entity| entity.kind == "VISUAL_CONCEPT")
            .filter(|entity| {
                // A fragment of a longer named entity ("d'Assise d'Appello"
                // inside "Corte d'Assise d'Appello") is inventory, not a
                // second production candidate.
                !named_spans.iter().any(|(start, end)| {
                    *start <= entity.start_byte
                        && entity.end_byte <= *end
                        && (*start < entity.start_byte || entity.end_byte < *end)
                })
            })
            .collect();
        concepts.sort_by(|left, right| {
            concept_priority(right)
                .partial_cmp(&concept_priority(left))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.text.len().cmp(&left.text.len()))
                .then_with(|| left.start_byte.cmp(&right.start_byte))
        });
        for concept in concepts
            .into_iter()
            .filter(|concept| concept_priority(concept) >= MIN_CONCEPT_PRIORITY)
            .take(MAX_GENERIC_CONCEPTS_PER_SCENE)
        {
            selected.insert(concept.id.clone());
        }
    }
    for id in selected {
        let Some(identity) = identities.get(&id) else {
            continue;
        };
        if identity.kind != "VISUAL_CONCEPT" {
            continue;
        }
        let priority = concept_identity_priority(identity);
        if priority < MIN_CONCEPT_PRIORITY {
            continue;
        }
        let Some(anchor) = identity_anchor(identity) else {
            continue;
        };
        let mut scene_ids = BTreeSet::new();
        for mention in &identity.mentions {
            scene_ids.insert(mention.scene_id.clone());
        }
        candidates.push(ProductionCandidate {
            id: String::new(),
            recipe: "entity_search".to_string(),
            rank: 0,
            priority,
            scene_id: anchor.scene_id.clone(),
            scene_ids: scene_ids.into_iter().collect(),
            entity_ids: vec![identity.id.clone()],
            fact_ids: Vec::new(),
            group_ids: Vec::new(),
            title: identity.canonical.clone(),
            evidence: anchor.surface.clone(),
            start_byte: anchor.start,
            end_byte: anchor.end,
            suggested_visual: "search_asset".to_string(),
            chart_items: Vec::new(),
            temporal_surfaces: Vec::new(),
            asset_key: format!("entity:{}", identity.id),
            queries: vec![SearchQuery {
                kind: "exact_mention".to_string(),
                text: anchor.surface.clone(),
            }],
            requires_media: true,
            dedup_key: format!("entity:{}:search", identity.id),
            covered_fact_ids: Vec::new(),
            rationale: vec![
                "selected among the scene's most specific visual concepts".to_string(),
                "generic abstract nouns are down-ranked, not deleted from inventory".to_string(),
            ],
        });
    }
}

fn push_fact_cards(
    facts: &[ConsolidatedFact],
    identities: &BTreeMap<String, IdentityState>,
    representations: &mut BTreeMap<String, (String, String)>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for fact in facts {
        if representations.contains_key(&fact.id) {
            continue;
        }
        let subject = fact.subject_entity_id.clone();
        let label = identities
            .get(&subject)
            .map(|identity| identity.canonical.clone())
            .unwrap_or_else(|| subject.clone());
        let recipe = fact_card_recipe(fact);
        let key = format!("factcard:{}", fact.id);
        representations.insert(fact.id.clone(), (key.clone(), "fact_card".to_string()));
        candidates.push(ProductionCandidate {
            id: String::new(),
            recipe: "fact_card".to_string(),
            rank: 0,
            priority: (0.7 + fact.importance * 0.15).min(0.88),
            scene_id: fact.scene_ids.first().cloned().unwrap_or_default(),
            scene_ids: fact.scene_ids.clone(),
            entity_ids: vec![subject.clone()],
            fact_ids: vec![fact.id.clone()],
            group_ids: Vec::new(),
            title: format!("{label} — {}", fact_card_title(fact)),
            evidence: fact.evidence.clone(),
            start_byte: fact.evidence_start_byte,
            end_byte: fact.evidence_end_byte,
            suggested_visual: "info_card".to_string(),
            chart_items: Vec::new(),
            temporal_surfaces: Vec::new(),
            asset_key: key.clone(),
            queries: Vec::new(),
            requires_media: false,
            dedup_key: key,
            covered_fact_ids: vec![fact.id.clone()],
            rationale: vec![
                "one contextual card preserves this claim's evidence and status".to_string(),
                format!("recipe: {recipe}"),
            ],
        });
    }
}

fn fact_card_recipe(fact: &ConsolidatedFact) -> String {
    match (fact.predicate.as_str(), fact.normalized_value.as_deref()) {
        ("investigation_status", Some("under_investigation")) => {
            "investigation_status_card".to_string()
        }
        ("investigation_status", Some("denies_accusations")) => "statement_card".to_string(),
        ("legal_outcome" | "legal_finality", _) => "legal_outcome_card".to_string(),
        _ => "fact_card".to_string(),
    }
}

fn fact_card_title(fact: &ConsolidatedFact) -> &'static str {
    match (fact.predicate.as_str(), fact.normalized_value.as_deref()) {
        ("investigation_status", Some("under_investigation")) => "stato del procedimento",
        ("investigation_status", Some("denies_accusations")) => "dichiarazioni",
        ("legal_outcome" | "legal_finality", _) => "esito giudiziario",
        _ => "scheda informativa",
    }
}

/// Pick the mention that best anchors one identity. Prefer the canonical full
/// surface when the script states it (so the query is both precise and an exact
/// slice of the anchor scene); otherwise use the longest surface, then source
/// order. This keeps every production query grounded in the scene it points at.
fn identity_anchor(identity: &IdentityState) -> Option<&MentionRef> {
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

fn is_named_kind(kind: &str) -> bool {
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

fn named_priority(kind: &str, mentions: usize, score: f32) -> f32 {
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

fn concept_priority(candidate: &Candidate) -> f32 {
    let lower = candidate.text.to_lowercase();
    let tokens = candidate.text.split_whitespace().count();
    let chars = candidate.text.chars().count();
    let length_bonus = ((tokens.saturating_sub(1)) as f32 * 0.06).min(0.15)
        + ((chars.min(24) as f32) / 24.0) * 0.05;
    let penalty = visual_phrase_penalty(&lower);
    (0.22 + candidate.score.clamp(0.0, 1.0) * 0.4 + length_bonus - penalty).clamp(0.05, 0.95)
}

fn concept_identity_priority(identity: &IdentityState) -> f32 {
    let lower = identity.canonical.to_lowercase();
    let tokens = identity.canonical.split_whitespace().count();
    let penalty = visual_phrase_penalty(&lower) + 0.1;
    (0.25
        + identity.max_score.clamp(0.0, 1.0) * 0.35
        + (identity.mentions.len().min(5) as f32) * 0.03
        + ((tokens.saturating_sub(1)) as f32 * 0.04).min(0.1)
        - penalty)
        .clamp(0.05, 0.85)
}

/// A phrase is a poor image query when it is entirely abstract or when it is
/// really a clause: an adverb (`automaticamente`) or an infinitive with an
/// attached pronoun (`restituirle`) is not a visual subject. The rule is
/// lexical and conservative; it only down-ranks, never deletes.
fn visual_phrase_penalty(lower: &str) -> f32 {
    let words: Vec<&str> = lower.split_whitespace().collect();
    let mut penalty = if GENERIC_VISUAL_NOUNS.contains(&lower) {
        0.35
    } else if words.iter().any(|word| GENERIC_VISUAL_NOUNS.contains(word)) {
        0.12
    } else {
        0.0
    };
    if words.iter().any(|word| word.ends_with("mente")) {
        penalty += 0.28;
    }
    if words.iter().any(|word| {
        word.len() > 5
            && (word.ends_with("are") || word.ends_with("ere") || word.ends_with("ire"))
            && !matches!(*word, "carcere" | "dovere" | "parere" | "piacere")
    }) {
        penalty += 0.18;
    }
    // Infinitive with an attached clitic ("restituirle", "raccontarlo") is a
    // clause fragment, not a visual noun phrase.
    if words.iter().any(|word| {
        word.len() > 6
            && (word.ends_with("rle")
                || word.ends_with("rlo")
                || word.ends_with("rla")
                || word.ends_with("rli")
                || word.ends_with("rci")
                || word.ends_with("rsi")
                || word.ends_with("rvi")
                || word.ends_with("rne"))
    }) {
        penalty += 0.2;
    }
    if words.iter().any(|word| {
        matches!(
            *word,
            "già"
                | "ancora"
                | "sempre"
                | "mai"
                | "più"
                | "meno"
                | "molto"
                | "poco"
                | "troppo"
                | "così"
                | "dove"
                | "quando"
                | "mentre"
                | "senza"
                | "dentro"
                | "fuori"
                | "dopo"
                | "prima"
                | "oggi"
                | "ieri"
                | "domani"
        )
    }) {
        penalty += 0.12;
    }
    penalty
}

fn scene_rank(index: &HashMap<&str, usize>, scene_id: &str) -> usize {
    index.get(scene_id).copied().unwrap_or(usize::MAX)
}

fn metrics(
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

fn scene_stats(
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
