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
            let recipe = match proposal.proposal_type.as_str() {
                "chart" => "chart",
                "delta_comparison" => "delta_comparison",
                _ => continue,
            };
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
            let key = format!("{recipe}:{}:{}", scene.scene_id, spans.join(","));
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
                recipe: recipe.to_string(),
                rank: 0,
                priority: if recipe == "chart" { 0.88 } else { 0.84 },
                scene_id: scene.scene_id.clone(),
                scene_ids: vec![scene.scene_id.clone()],
                entity_ids,
                fact_ids: Vec::new(),
                group_ids: Vec::new(),
                title: if recipe == "chart" {
                    "Confronto dati".to_string()
                } else {
                    "Variazione".to_string()
                },
                evidence: proposal.evidence.clone(),
                start_byte: proposal.start_byte,
                end_byte: proposal.end_byte,
                suggested_visual: proposal.suggested_visual.clone(),
                chart_items: items,
                chart_meta: proposal.chart_meta.clone(),
                temporal_surfaces: Vec::new(),
                asset_key: key.clone(),
                queries: Vec::new(),
                requires_media: false,
                dedup_key: key,
                covered_fact_ids: Vec::new(),
                rationale: vec![
                    "one grounded comparison for the whole value set".to_string(),
                    if recipe == "chart" {
                        "reference numbers are excluded from quantitative charts".to_string()
                    } else {
                        "delta proposal requires two linked grounded quantities".to_string()
                    },
                ],
            });
        }
    }
}

/// Emits `ranking_bar` production candidates for grounded ranked groups whose
/// members carry real same-unit quantities. Rank indices are never chart
/// values: a quantity whose span starts at the member's rank marker is skipped,
/// and only 2..=10 members qualify. This never emits a `chart` proposal, so the
/// existing Top-10 rank-veto assertion stays intact.
pub(super) fn push_ranking_bars(
    sources: &[SceneInput],
    scenes: &[SceneManifest],
    candidates: &mut Vec<ProductionCandidate>,
) {
    let texts: HashMap<&str, &str> = sources
        .iter()
        .map(|scene| (scene.scene_id.as_str(), scene.text.as_str()))
        .collect();
    let mut seen = HashSet::new();
    for scene in scenes {
        let Some(text) = texts.get(scene.scene_id.as_str()) else {
            continue;
        };
        for group in &scene.semantic_groups {
            if group.group_type != "ranking" || !(2..=10).contains(&group.members.len()) {
                continue;
            }
            let mut matched: Vec<(&semantic_core::SemanticGroupMember, &semantic_core::Quantity)> =
                Vec::new();
            let mut signature: Option<(String, String)> = None;
            for member in &group.members {
                let quantity = scene
                    .quantities
                    .iter()
                    .filter(|quantity| {
                        quantity.start_byte >= member.start_byte
                            && quantity.end_byte <= member.end_byte
                            && quantity.value.is_some()
                            && quantity.dimension != "rank"
                            && quantity.dimension != "growth_rate"
                    })
                    // A quantity at the member's first non-space byte is the
                    // rank marker itself ("1. France ..."), never a metric.
                    .find(|quantity| {
                        let leading = &text[member.start_byte..quantity.start_byte];
                        leading.trim().chars().any(char::is_alphabetic)
                    });
                let Some(quantity) = quantity else {
                    signature = None;
                    break;
                };
                let key = (quantity.dimension.clone(), quantity.unit.clone());
                if signature.as_ref().is_some_and(|existing| existing != &key) {
                    signature = None;
                    break;
                }
                signature = Some(key);
                matched.push((member, quantity));
            }
            let Some((dimension, unit)) = signature else {
                continue;
            };
            if matched.len() != group.members.len() {
                continue;
            }
            let key = format!("ranking_bar:{}:{}", scene.scene_id, group.id);
            if !seen.insert(key.clone()) {
                continue;
            }
            // Do not repeat a chart that already represents the same values.
            let already_charted = candidates.iter().any(|candidate| {
                candidate.recipe == "chart"
                    && matched.iter().all(|(_, quantity)| {
                        candidate.chart_items.iter().any(|item| {
                            item.start_byte == quantity.start_byte
                                && item.end_byte == quantity.end_byte
                        })
                    })
            });
            if already_charted {
                continue;
            }
            let start = group
                .members
                .iter()
                .map(|member| member.start_byte)
                .min()
                .unwrap_or(scene_start_fallback());
            let end = group
                .members
                .iter()
                .map(|member| member.end_byte)
                .max()
                .unwrap_or(start);
            let Some(evidence) = text.get(start..end) else {
                continue;
            };
            let items: Vec<ChartItem> = matched
                .iter()
                .filter_map(|(member, quantity)| {
                    Some(ChartItem {
                        label: member.text.clone(),
                        text: quantity.evidence.clone(),
                        kind: match quantity.dimension.as_str() {
                            "percent" => "PERCENT",
                            "currency" => "MONEY",
                            _ => "NUMBER",
                        }
                        .to_string(),
                        value: quantity.value?,
                        start_byte: quantity.start_byte,
                        end_byte: quantity.end_byte,
                    })
                })
                .collect();
            if items.len() != matched.len() {
                continue;
            }
            let chart_meta = semantic_core::ChartMetadata {
                metric: group.ranking_basis.clone(),
                unit: Some(unit.clone()).filter(|unit| !unit.is_empty()),
                dimension: Some(dimension.clone()),
                period: None,
                series_type: Some("category".to_string()),
            };
            candidates.push(ProductionCandidate {
                id: String::new(),
                recipe: "ranking_bar".to_string(),
                rank: 0,
                priority: 0.82,
                scene_id: scene.scene_id.clone(),
                scene_ids: vec![scene.scene_id.clone()],
                entity_ids: Vec::new(),
                fact_ids: Vec::new(),
                group_ids: vec![group.id.clone()],
                title: group.title.clone(),
                evidence: evidence.to_string(),
                start_byte: start,
                end_byte: end,
                suggested_visual: "ranking_bar".to_string(),
                chart_items: items,
                chart_meta: Some(chart_meta),
                temporal_surfaces: Vec::new(),
                asset_key: key.clone(),
                queries: Vec::new(),
                requires_media: false,
                dedup_key: key,
                covered_fact_ids: Vec::new(),
                rationale: vec![
                    "ranked members carry grounded same-unit quantities".to_string(),
                    "rank indices are excluded from chart values".to_string(),
                ],
            });
        }
    }
}

/// Fallback start for a malformed group without members; never used by the
/// public contract because an empty group is skipped earlier.
fn scene_start_fallback() -> usize {
    0
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
                chart_meta: None,
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
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    let mut relation_support: HashMap<&str, usize> = HashMap::new();
    let mut fact_support: HashMap<&str, usize> = HashMap::new();
    for scene in scenes {
        for relation in &scene.relations.asserted_relations {
            if relation.status == semantic_core::RelationStatus::Asserted {
                *relation_support.entry(&relation.source_id).or_default() += 1;
                *relation_support.entry(&relation.target_id).or_default() += 1;
            }
        }
        for fact in &scene.entity_facts {
            if fact.status == "asserted" {
                *fact_support.entry(&fact.subject_entity_id).or_default() += 1;
                if let Some(target) = fact.target_entity_id.as_deref() {
                    *fact_support.entry(target).or_default() += 1;
                }
            }
        }
    }
    for identity in identities.values() {
        if !is_named_kind(&identity.kind) {
            continue;
        }
        let Some(anchor) = identity_anchor(identity) else {
            continue;
        };
        let priority = named_priority_for_surface(
            &identity.kind,
            &identity.canonical,
            identity.mentions.len(),
            identity.max_score,
            relation_support
                .get(identity.id.as_str())
                .copied()
                .unwrap_or(0),
            fact_support.get(identity.id.as_str()).copied().unwrap_or(0),
        );
        let mut scene_ids = BTreeSet::new();
        for mention in &identity.mentions {
            scene_ids.insert(mention.scene_id.clone());
        }
        // Keep weak identities available for contextual enrichment: they only
        // become a media search when an asserted relation or a grounded visual
        // context supports the mention, which is exactly the non-empty query
        // ladder the factory turns into `requires_media`.
        let queries = if priority > 0.0 {
            vec![SearchQuery::literal(anchor.surface.clone())]
        } else {
            Vec::new()
        };
        candidates.push(entity_search_candidate(
            identity,
            anchor,
            priority,
            scene_ids.iter().cloned().collect(),
            queries,
            vec![
                format!(
                    "{} named identity collapsed from {} mentions",
                    identity.kind,
                    identity.mentions.len()
                ),
                "one asset search per canonical identity".to_string(),
            ],
        ));
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
            chart_meta: None,
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

use super::entity_search_candidate;
use super::identities::{identity_anchor, is_named_kind, named_priority_for_surface};
use super::IdentityState;
