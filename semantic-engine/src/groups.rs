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
    pub temporal_surfaces: Vec<String>,
    pub suggested_layout: Option<&'static str>,
    /// Images for these entities must be verified before admission.
    pub requires_verified_images: bool,
    /// Verbatim source sentence or fragment that grounds this proposal.
    pub evidence: String,
    pub start: usize,
    pub end: usize,
}

/// Builds every proposal of one scene, in deterministic source order.
pub fn build_candidates(
    source: &str,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    chart_points: &[ChartPoint],
    lower_thirds: &[LowerThird],
    temporal_facts: &[TemporalFact],
    identities: &IdentityLookup,
) -> Vec<VisualCandidate> {
    let mut out = Vec::new();
    entity_searches(entities, identities, &mut out);
    multiple_entities(entities, bullets, identities, &mut out);
    charts(
        source,
        entities,
        bullets,
        chart_points,
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
        .filter_map(|(index, entity)| is_visual_entity(entity).then_some(index))
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
        if indices.len() < 2 {
            continue;
        }
        let layout = if indices.len() == 2 {
            "side_by_side"
        } else {
            "grid"
        };
        out.push(VisualCandidate {
            kind: "multiple_entities",
            sentence_index: bullet.sentence_index,
            entity_ids: ids_of(identities, &indices),
            entity_texts: texts_of(entities, &indices),
            entity_indices: indices,
            chart_items: Vec::new(),
            temporal_surfaces: Vec::new(),
            suggested_layout: Some(layout),
            requires_verified_images: true,
            evidence: bullet.text.clone(),
            start: bullet.start,
            end: bullet.end,
        });
    }
}

fn charts(
    source: &str,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    chart_points: &[ChartPoint],
    identities: &IdentityLookup,
    out: &mut Vec<VisualCandidate>,
) {
    let mut sentences: Vec<usize> = chart_points.iter().map(|p| p.sentence_index).collect();
    sentences.sort_unstable();
    sentences.dedup();
    for sentence in sentences {
        let Some(bullet) = bullets.iter().find(|b| b.sentence_index == sentence) else {
            continue;
        };
        if !crate::chart_rules::chart_claim_is_eligible(&bullet.text) {
            continue;
        }
        let points: Vec<&ChartPoint> = chart_points
            .iter()
            .filter(|p| p.sentence_index == sentence)
            .collect();
        let mut measures: Vec<crate::chart_rules::ChartUnit> = points
            .iter()
            .filter_map(|point| {
                let relative_start = point.start.checked_sub(bullet.start)?;
                let relative_end = point.end.checked_sub(bullet.start)?;
                Some(crate::chart_rules::chart_unit(
                    &bullet.text,
                    relative_start,
                    relative_end,
                    point.value,
                    &point.kind,
                ))
            })
            .collect();
        let explicit_units: std::collections::BTreeSet<(String, String)> = measures
            .iter()
            .filter(|measure| measure.signature != measure.kind)
            .map(|measure| (measure.signature.clone(), measure.kind.clone()))
            .collect();
        if explicit_units.len() == 1 {
            let (signature, kind) = explicit_units.iter().next().unwrap();
            for measure in &mut measures {
                if measure.signature == measure.kind {
                    measure.signature = signature.clone();
                    measure.kind = kind.clone();
                }
            }
        }
        if measures.len() != points.len()
            || measures
                .windows(2)
                .any(|pair| pair[0].signature != pair[1].signature || pair[0].kind != pair[1].kind)
        {
            continue;
        }
        let items: Vec<ChartItem> = points
            .iter()
            .zip(&measures)
            .filter_map(|(p, measure)| {
                let relative_start = p.start.checked_sub(bullet.start)?;
                let relative_end = p.end.checked_sub(bullet.start)?;
                let (qualified_start, qualified_end) = crate::chart_rules::qualified_numeric_span(
                    &bullet.text,
                    relative_start,
                    relative_end,
                );
                let text = bullet.text.get(qualified_start..qualified_end)?;
                Some(ChartItem {
                    label: p.label.clone(),
                    text: text.to_string(),
                    kind: measure.kind.clone(),
                    value: measure.value,
                    start_byte: bullet.start + qualified_start,
                    end_byte: bullet.start + qualified_end,
                })
            })
            .collect();
        // A chart needs at least two comparable values; one number is not a
        // comparison, and mixed kinds (a year beside a percentage) are not
        // comparable either.
        if items.len() < 2 || !all_same_kind(&items) {
            continue;
        }
        // Equal units and a shared sentence are not enough to prove that two
        // values belong in one chart. Require either an explicit comparison
        // cue or the conservative repeated-series context checked below.
        if !comparison_cue_binds_values(bullet, chart_points, sentence)
            && !crate::chart_rules::has_series_context(&bullet.text, items.len())
        {
            continue;
        }
        let mut indices: Vec<usize> = chart_points
            .iter()
            .filter(|p| p.sentence_index == sentence)
            .filter_map(|p| p.entity)
            .collect();
        indices.sort_unstable();
        indices.dedup();
        out.push(VisualCandidate {
            kind: "chart",
            sentence_index: sentence,
            entity_ids: ids_of(identities, &indices),
            entity_texts: texts_of(entities, &indices),
            entity_indices: indices,
            chart_items: items,
            temporal_surfaces: Vec::new(),
            suggested_layout: Some("bar_comparison"),
            requires_verified_images: false,
            evidence: bullet.text.clone(),
            start: bullet.start,
            end: bullet.end,
        });
    }
    cross_sentence_series(source, entities, bullets, chart_points, identities, out);
}

/// Joins adjacent source-grounded measures when the script repeats a metric
/// across sentences (for example, annual revenue or output). The emitted
/// evidence is one exact source slice spanning all included sentences.
fn cross_sentence_series(
    source: &str,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    chart_points: &[ChartPoint],
    identities: &IdentityLookup,
    out: &mut Vec<VisualCandidate>,
) {
    let mut ordered: Vec<&ChartPoint> = chart_points.iter().collect();
    ordered.sort_by_key(|point| (point.start, point.end));
    for (left_index, left) in ordered.iter().enumerate() {
        for right in ordered.iter().skip(left_index + 1) {
            if left.sentence_index == right.sentence_index {
                continue;
            }
            let Some(left_bullet) = bullets
                .iter()
                .find(|b| b.sentence_index == left.sentence_index)
            else {
                continue;
            };
            let Some(right_bullet) = bullets
                .iter()
                .find(|b| b.sentence_index == right.sentence_index)
            else {
                continue;
            };
            if right.sentence_index.saturating_sub(left.sentence_index) > 3
                || !crate::chart_rules::chart_claim_is_eligible(&left_bullet.text)
                || !crate::chart_rules::chart_claim_is_eligible(&right_bullet.text)
                || !same_series_metric(&left_bullet.text, &right_bullet.text)
            {
                continue;
            }
            let has_time_anchor = has_four_digit_year(&left_bullet.text)
                && (has_four_digit_year(&right_bullet.text)
                    || right_bullet.text.to_lowercase().contains("seguente")
                    || right_bullet.text.to_lowercase().contains("following year"));
            if !has_time_anchor {
                continue;
            }
            let evidence_start = left_bullet.start.min(right_bullet.start);
            let evidence_end = left_bullet.end.max(right_bullet.end);
            let Some(evidence) = source.get(evidence_start..evidence_end) else {
                continue;
            };
            if !crate::chart_rules::chart_claim_is_eligible(evidence) {
                continue;
            }
            let Some(left_start) = left.start.checked_sub(left_bullet.start) else {
                continue;
            };
            let Some(left_end) = left.end.checked_sub(left_bullet.start) else {
                continue;
            };
            let Some(right_start) = right.start.checked_sub(right_bullet.start) else {
                continue;
            };
            let Some(right_end) = right.end.checked_sub(right_bullet.start) else {
                continue;
            };
            let mut measures = [
                crate::chart_rules::chart_unit(
                    &left_bullet.text,
                    left_start,
                    left_end,
                    left.value,
                    &left.kind,
                ),
                crate::chart_rules::chart_unit(
                    &right_bullet.text,
                    right_start,
                    right_end,
                    right.value,
                    &right.kind,
                ),
            ];
            if measures[0].signature == measures[0].kind
                && measures[1].signature != measures[1].kind
            {
                measures[0].signature = measures[1].signature.clone();
                measures[0].kind = measures[1].kind.clone();
            } else if measures[1].signature == measures[1].kind
                && measures[0].signature != measures[0].kind
            {
                measures[1].signature = measures[0].signature.clone();
                measures[1].kind = measures[0].kind.clone();
            }
            if measures[0].signature != measures[1].signature
                || measures[0].kind != measures[1].kind
            {
                continue;
            }
            let (left_display_start, left_display_end) =
                crate::chart_rules::qualified_numeric_span(&left_bullet.text, left_start, left_end);
            let (right_display_start, right_display_end) =
                crate::chart_rules::qualified_numeric_span(
                    &right_bullet.text,
                    right_start,
                    right_end,
                );
            let (Some(left_text), Some(right_text)) = (
                left_bullet.text.get(left_display_start..left_display_end),
                right_bullet
                    .text
                    .get(right_display_start..right_display_end),
            ) else {
                continue;
            };
            let items = vec![
                ChartItem {
                    label: left.label.clone(),
                    text: left_text.to_string(),
                    kind: measures[0].kind.clone(),
                    value: measures[0].value,
                    start_byte: left_bullet.start + left_display_start,
                    end_byte: left_bullet.start + left_display_end,
                },
                ChartItem {
                    label: right.label.clone(),
                    text: right_text.to_string(),
                    kind: measures[1].kind.clone(),
                    value: measures[1].value,
                    start_byte: right_bullet.start + right_display_start,
                    end_byte: right_bullet.start + right_display_end,
                },
            ];
            if out.iter().any(|candidate| {
                candidate.kind == "chart"
                    && candidate
                        .chart_items
                        .iter()
                        .any(|item| item.start_byte == left.start)
                    && candidate
                        .chart_items
                        .iter()
                        .any(|item| item.start_byte == right.start)
            }) {
                continue;
            }
            let mut indices: Vec<usize> =
                [left.entity, right.entity].into_iter().flatten().collect();
            indices.sort_unstable();
            indices.dedup();
            out.push(VisualCandidate {
                kind: "chart",
                sentence_index: left.sentence_index.min(right.sentence_index),
                entity_ids: ids_of(identities, &indices),
                entity_texts: texts_of(entities, &indices),
                entity_indices: indices,
                chart_items: items,
                temporal_surfaces: Vec::new(),
                suggested_layout: Some("bar_comparison"),
                requires_verified_images: false,
                evidence: evidence.to_string(),
                start: evidence_start,
                end: evidence_end,
            });
        }
    }
}

fn has_four_digit_year(text: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_digit()).any(|token| {
        token.len() == 4
            && token
                .parse::<u16>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
    })
}

fn same_series_metric(left: &str, right: &str) -> bool {
    const METRICS: &[&str] = &[
        "ricav",
        "fatturat",
        "vendu",
        "vendit",
        "prodott",
        "produzion",
        "incident",
        "sales",
        "revenue",
        "produced",
        "production",
        "accident",
        "export",
    ];
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    METRICS
        .iter()
        .any(|metric| left.contains(metric) && right.contains(metric))
}

/// A comparison word elsewhere in a long enumerated bullet must not turn the
/// list's rank numbers into chart values. Bind the cue to at least two
/// comparable values on the same source line. Ordinary prose comparisons
/// remain valid because their values and cue share a line.
fn comparison_cue_binds_values(
    bullet: &Bullet,
    chart_points: &[ChartPoint],
    sentence_index: usize,
) -> bool {
    let Some((cue_start, _)) = semantic_core::discourse::comparison_cue(&bullet.text) else {
        return false;
    };
    let cue_line = bullet.text[..cue_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    let comparable_on_line = chart_points
        .iter()
        .filter(|point| point.sentence_index == sentence_index)
        .filter(|point| point.start >= bullet.start && point.end <= bullet.end)
        .filter(|point| {
            let relative = point.start - bullet.start;
            bullet.text[..relative]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                == cue_line
        })
        .count();
    comparable_on_line >= 2
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
