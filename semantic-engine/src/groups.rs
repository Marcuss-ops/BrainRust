//! Visual proposals derived from one scene's grounded analysis.
//!
//! A proposal says *what could be shown together*, never *what must be shown*.
//! Two names in one sentence are a co-mention: the engine proposes a
//! side-by-side layout and never asserts a personal or investigative relation.
//! Admission (how many images, which assets, timing) stays in PipelineGen.
//!
//! Every proposal carries the canonical ids of the identities it is about, so
//! the planner can map a scene's proposal to the same person across scenes.

use serde::Serialize;
use std::collections::HashSet;
use visualner::analysis::{Bullet, ChartPoint, LowerThird};
use visualner::{TemporalFact, VisualEntity};

use crate::identity_lookup::IdentityLookup;
pub use semantic_core::ChartItem;
use semantic_core::{ChartMetadata, Quantity};

#[path = "groups/charts.rs"]
mod charts;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualCandidate {
    /// `multiple_entities`, `chart`, `timeline` or `entity_card`.
    pub kind: &'static str,
    pub sentence_index: usize,
    /// Indices into the scene's `entities`.
    pub entity_indices: Vec<usize>,
    /// Canonical ids of the referenced identities, resolved across scenes.
    pub entity_ids: Vec<String>,
    pub entity_texts: Vec<String>,
    pub chart_items: Vec<ChartItem>,
    pub chart_meta: Option<ChartMetadata>,
    pub temporal_surfaces: Vec<String>,
    pub suggested_layout: Option<&'static str>,
    /// Images for these entities must be verified before admission.
    pub requires_verified_images: bool,
    /// Verbatim source sentence or fragment that grounds this proposal.
    pub evidence: String,
    pub start: usize,
    pub end: usize,
}

/// One scene's source text and lexicon language.
pub struct SceneSource<'a> {
    pub text: &'a str,
    pub language: &'a str,
}

/// Numeric chart points paired with their semantic unit projection.
pub struct ChartInput<'a> {
    pub points: &'a [ChartPoint],
    pub quantities: &'a [Quantity],
    pub comparisons: &'a [semantic_core::QuantityComparison],
}

/// Builds every proposal of one scene, in deterministic source order.
pub fn build_candidates(
    scene: SceneSource<'_>,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    chart_input: ChartInput<'_>,
    lower_thirds: &[LowerThird],
    temporal_facts: &[TemporalFact],
    identities: &IdentityLookup,
) -> Vec<VisualCandidate> {
    let source = scene.text;
    let language = scene.language;
    let mut out = Vec::new();
    entity_searches(entities, identities, &mut out);
    multiple_entities(entities, bullets, identities, &mut out);
    charts::build(
        source,
        language,
        entities,
        bullets,
        chart_input,
        identities,
        &mut out,
    );
    timelines(bullets, temporal_facts, &mut out);
    entity_cards(entities, lower_thirds, identities, &mut out);
    out
}

/// Keeps an asset-search opportunity for every grounded visual entity mention,
/// even when it does not share a sentence with another entity or lower third.
/// Retrieval and admission remain the planner's responsibility.
fn entity_searches(
    entities: &[VisualEntity],
    identities: &IdentityLookup,
    out: &mut Vec<VisualCandidate>,
) {
    let indices: Vec<usize> = entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| {
            // Every grounded visual mention keeps a retrieval opportunity,
            // including an open-class concept: the planner still owns
            // admission, so suppressing the proposal would silently drop a
            // subject the caller can picture and search.
            (is_visual_entity(entity) || entity.r#type == "VISUAL_CONCEPT").then_some(index)
        })
        .collect();
    let mut seen = HashSet::new();
    for index in indices {
        let Some(entity) = entities.get(index) else {
            continue;
        };
        let id = identities.id_of(index);
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(VisualCandidate {
            kind: "entity_search",
            sentence_index: 0,
            entity_ids: vec![id],
            entity_indices: vec![index],
            entity_texts: vec![entity.text.clone()],
            chart_items: Vec::new(),
            chart_meta: None,
            temporal_surfaces: Vec::new(),
            suggested_layout: Some("search_asset"),
            requires_verified_images: true,
            evidence: entity.evidence.clone(),
            start: entity.start,
            end: entity.end,
        });
    }
}

fn multiple_entities(
    entities: &[VisualEntity],
    bullets: &[Bullet],
    identities: &IdentityLookup,
    out: &mut Vec<VisualCandidate>,
) {
    for bullet in bullets {
        // Only named subjects share a frame. A sentence that lists ten generic
        // nouns ("persone", "nome", "tempo") is narration, not a composition,
        // and each of those nouns would become a search query of its own.
        let mut indices: Vec<usize> = bullet
            .entities
            .iter()
            .copied()
            .filter(|&i| {
                entities
                    .get(i)
                    .is_some_and(|entity| is_visual_entity(entity) && is_named_subject(entity))
            })
            .collect();
        indices.sort_unstable();
        indices.dedup();
        let mut seen_identities = HashSet::new();
        indices.retain(|&index| seen_identities.insert(identities.id_of(index)));
        // A long co-mention is a semantic topic, not a frame. Individual
        // entity searches already preserve coverage; only an explicit pair
        // can become a side-by-side composition without inventing a collage.
        if indices.len() != 2 {
            continue;
        }
        if !has_renderable_pair_cue(&bullet.text) {
            continue;
        }
        out.push(VisualCandidate {
            kind: "multiple_entities",
            sentence_index: bullet.sentence_index,
            entity_ids: ids_of(identities, &indices),
            entity_texts: texts_of(entities, &indices),
            entity_indices: indices,
            chart_items: Vec::new(),
            chart_meta: None,
            temporal_surfaces: Vec::new(),
            suggested_layout: Some("side_by_side"),
            requires_verified_images: true,
            evidence: bullet.text.clone(),
            start: bullet.start,
            end: bullet.end,
        });
    }
}

fn has_renderable_pair_cue(sentence: &str) -> bool {
    let normalized = sentence.to_lowercase();
    crate::consolidate::image_query_policy()
        .renderable_pair_cues
        .iter()
        .any(|cue| normalized.contains(&cue.to_lowercase()))
}

fn timelines(bullets: &[Bullet], temporal_facts: &[TemporalFact], out: &mut Vec<VisualCandidate>) {
    for bullet in bullets {
        let inside: Vec<&TemporalFact> = temporal_facts
            .iter()
            .filter(|f| f.start_byte >= bullet.start && f.end_byte <= bullet.end)
            .collect();
        if inside.len() < 2 {
            continue;
        }
        out.push(VisualCandidate {
            kind: "timeline",
            sentence_index: bullet.sentence_index,
            entity_ids: Vec::new(),
            entity_indices: Vec::new(),
            entity_texts: Vec::new(),
            chart_items: Vec::new(),
            chart_meta: None,
            temporal_surfaces: inside.iter().map(|f| f.surface.clone()).collect(),
            suggested_layout: Some("timeline"),
            requires_verified_images: false,
            evidence: bullet.text.clone(),
            start: bullet.start,
            end: bullet.end,
        });
    }
}

fn entity_cards(
    entities: &[VisualEntity],
    lower_thirds: &[LowerThird],
    identities: &IdentityLookup,
    out: &mut Vec<VisualCandidate>,
) {
    let mut seen = HashSet::new();
    for third in lower_thirds {
        let Some(entity) = entities.get(third.entity) else {
            continue;
        };
        if !is_visual_entity(entity) {
            continue;
        }
        let id = identities.id_of(third.entity);
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(VisualCandidate {
            kind: "entity_card",
            sentence_index: third.sentence_index,
            entity_ids: vec![id],
            entity_indices: vec![third.entity],
            entity_texts: vec![entity.text.clone()],
            chart_items: Vec::new(),
            chart_meta: None,
            temporal_surfaces: Vec::new(),
            suggested_layout: None,
            requires_verified_images: true,
            evidence: third.evidence.clone(),
            start: third.start,
            end: third.end,
        });
    }
}

/// Numbers and dates are measurements, not things to picture. A spelled-out
/// number such as "due" must never become a person card or a co-mention group;
/// measurements feed charts and timelines instead.
pub(crate) fn is_visual_entity(entity: &VisualEntity) -> bool {
    matches!(
        entity.r#type.as_str(),
        "PERSON"
            | "ORGANIZATION"
            | "ORG"
            | "LOCATION"
            | "FACILITY"
            | "BRAND"
            | "PRODUCT"
            | "WORK"
            | "OBJECT"
            | "STRUCTURE"
            | "EVENT"
    )
}

/// A named subject a planner can picture and search: a person, place, venue,
/// organization, brand, product or work. Generic concepts and measurements
/// are not subjects of a co-mention grid.
pub(crate) fn is_named_subject(entity: &VisualEntity) -> bool {
    matches!(
        entity.r#type.as_str(),
        "PERSON" | "ORGANIZATION" | "ORG" | "LOCATION" | "FACILITY" | "BRAND" | "PRODUCT" | "WORK"
    )
}

/// A chart compares like with like: all values share one kind.
fn all_same_kind(items: &[ChartItem]) -> bool {
    items
        .first()
        .is_some_and(|first| items.iter().all(|item| item.kind == first.kind))
}

fn texts_of(entities: &[VisualEntity], indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .filter_map(|&i| entities.get(i).map(|e| e.text.clone()))
        .collect()
}

fn ids_of(identities: &IdentityLookup, indices: &[usize]) -> Vec<String> {
    indices.iter().map(|&i| identities.id_of(i)).collect()
}
