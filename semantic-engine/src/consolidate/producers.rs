//! Production candidate producers: reference numbers, charts, multiple-
//! entity compositions and named-entity candidates.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use semantic_core::{
    ChartItem, ProductionCandidate, ReferenceNumber, SceneInput, SceneManifest, SearchQuery,
};

pub(super) fn reference_numbers(
    sources: &[SceneInput],
    scenes: &[SceneManifest],
) -> Vec<ReferenceNumber> {
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

pub(super) fn push_charts(
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

pub(super) fn push_multiple_entities(
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

pub(super) fn push_named_entities(
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

use super::identities::{identity_anchor, is_named_kind, named_priority};
use super::IdentityState;
