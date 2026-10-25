//! Timeline proposals: date grouping, sequence selection and the timeline
//! card projection.

use std::collections::{BTreeMap, BTreeSet};

use semantic_core::{
    ConsolidatedDate, ConsolidatedDateSpan, ConsolidatedEventLink, ConsolidatedFact,
    ProductionCandidate, SceneManifest,
};

use super::TimelineGroup;

pub(super) fn canonical_dates(scenes: &[SceneManifest]) -> Vec<ConsolidatedDate> {
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

pub(super) fn timeline_groups<'a>(
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

pub(super) fn selected_timeline_groups(
    mut groups: Vec<TimelineGroup<'_>>,
) -> Vec<TimelineGroup<'_>> {
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

pub(super) fn push_timeline(
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
