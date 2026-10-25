//! Whole-script projections: the story graph and the visual coverage report.
//!
//! Both are derived from data the scene analysis already grounded; neither is a
//! second extractor. The story graph joins a canonical identity to its own facts
//! and orders one identity's dated facts across scenes. The coverage report asks,
//! for every grounded fact, whether a proposal charts its exact value or draws
//! its date as a timeline. Nothing is inferred from proximity: a shared sentence
//! is not a story link, and a searchable name is not a representation.

use std::collections::{BTreeMap, BTreeSet};

use semantic_core::{
    CoverageItem, EntityFact, SceneCoverage, SceneManifest, StoryEdge, StoryGraph, StoryNode,
    VisualCoverage,
};

/// Builds the cross-scene story graph. Nodes are the grounded facts plus the
/// named identities that matter to the story: the ones that own a fact, or that
/// are mentioned in at least two scenes. Edges are `about` (identity -> its
/// fact) and `chronological` (consecutive dated facts of one identity, when that
/// identity is mentioned in at least two scenes). A single-scene mention with
/// no fact is not a story link, and a generic concept (`VISUAL_CONCEPT`) is not
/// a story subject, so both stay out.
pub fn build_story_graph(scenes: &[SceneManifest]) -> StoryGraph {
    let scene_order: BTreeMap<&str, usize> = scenes
        .iter()
        .enumerate()
        .map(|(index, scene)| (scene.scene_id.as_str(), index))
        .collect();

    let mut identities: BTreeMap<String, IdentityNode> = BTreeMap::new();
    for scene in scenes {
        for identity in &scene.identities {
            // Measurements are not story subjects: a date or a percentage is
            // already carried by the fact that owns it. A `VISUAL_CONCEPT` is a
            // generic noun phrase ("giovane donna", "giorno", "modo"), not a
            // named subject: leaving it in fills the graph with the whole
            // vocabulary of the script.
            if matches!(
                identity.kind.as_str(),
                "NUMBER" | "PERCENT" | "MONEY" | "DATE" | "VISUAL_CONCEPT"
            ) {
                continue;
            }
            let node = identities.entry(identity.id.clone()).or_default();
            node.label = identity.canonical_surface.clone();
            for mention in &identity.mentions {
                let Some(&order) = scene_order.get(mention.scene_id.as_str()) else {
                    continue;
                };
                node.mentions.push((
                    order,
                    mention.scene_id.clone(),
                    mention.start_byte,
                    mention.end_byte,
                    mention.surface.clone(),
                ));
            }
        }
    }

    let mut edges = Vec::new();
    // Anchor metadata per identity, reused by the fact links.
    let mut anchors: BTreeMap<String, (usize, String, String, usize, usize)> = BTreeMap::new();

    for (identity_id, node) in &mut identities {
        node.mentions.sort();
        node.mentions.dedup();
        let Some((_, scene_id, start, end, _)) = node.mentions.first().cloned() else {
            continue;
        };
        anchors.insert(
            identity_id.clone(),
            (
                scene_order
                    .get(scene_id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX),
                scene_id,
                node.label.clone(),
                start,
                end,
            ),
        );
    }

    // Fact nodes keep the scene they come from: a fact id is only unique
    // inside its scene.
    let mut nodes = Vec::new();
    for scene in scenes {
        for fact in &scene.entity_facts {
            nodes.push(fact_node(&scene.scene_id, fact));
            let Some((anchor_order, _, _, _, _)) = anchors.get(&fact.subject_entity_id) else {
                continue;
            };
            let fact_order = scene_order
                .get(scene.scene_id.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            edges.push(StoryEdge {
                id: format!(
                    "about:{}:{}:{}",
                    fact.subject_entity_id, scene.scene_id, fact.id
                ),
                kind: "about".to_string(),
                from_id: format!("identity:{}", fact.subject_entity_id),
                to_id: fact_node_id(&scene.scene_id, &fact.id),
                cross_scene: *anchor_order != fact_order,
                evidence: fact.evidence.clone(),
                start_byte: fact.evidence_start_byte,
                end_byte: fact.evidence_end_byte,
            });
        }
    }

    edges.extend(chronology_edges(scenes, &scene_order, &anchors));
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    edges.dedup_by(|left, right| left.id == right.id);

    // An identity is a story node when a fact hangs off it or when the script
    // names it in more than one scene. Everything else would be an isolated
    // point that only repeats the scene-local entity list.
    let linked: BTreeSet<String> = edges
        .iter()
        .filter_map(|edge| edge.from_id.strip_prefix("identity:"))
        .map(str::to_string)
        .collect();
    for (identity_id, node) in &identities {
        let mut scene_ids: Vec<String> = Vec::new();
        for (_, mention_scene, _, _, _) in &node.mentions {
            if !scene_ids.iter().any(|known| known == mention_scene) {
                scene_ids.push(mention_scene.clone());
            }
        }
        if !linked.contains(identity_id) && scene_ids.len() < 2 {
            continue;
        }
        let Some((_, scene_id, start, end, surface)) = node.mentions.first().cloned() else {
            continue;
        };
        nodes.push(StoryNode {
            id: format!("identity:{identity_id}"),
            kind: "identity".to_string(),
            label: node.label.clone(),
            scene_id,
            scene_ids,
            entity_id: Some(identity_id.clone()),
            fact_id: None,
            normalized_date: None,
            sentence_index: None,
            // The anchor span belongs to one concrete mention; the canonical
            // label may be longer ("Alberto Stasi" vs a "Stasi" mention), so
            // the node carries the mention surface, never the label.
            evidence: surface,
            start_byte: start,
            end_byte: end,
        });
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));

    StoryGraph { nodes, edges }
}

#[derive(Default)]
struct IdentityNode {
    label: String,
    /// `(scene order, scene id, start, end, surface)` for every grounded
    /// mention. The surface is the text actually in that scene, so an alias
    /// mention ("Stasi") still grounds its own node span.
    mentions: Vec<(usize, String, usize, usize, String)>,
}

/// Identity -> fact links and their chronological projection. A chronology is
/// only built for an identity whose dated facts span at least two scenes; one
/// scene is already covered by the scene-local `step_sequence`/`chronology`
/// groups and would otherwise duplicate them.
fn chronology_edges(
    scenes: &[SceneManifest],
    scene_order: &BTreeMap<&str, usize>,
    anchors: &BTreeMap<String, (usize, String, String, usize, usize)>,
) -> Vec<StoryEdge> {
    let mut dated: BTreeMap<String, Vec<(&SceneManifest, &EntityFact)>> = BTreeMap::new();
    for scene in scenes {
        for fact in &scene.entity_facts {
            if fact.normalized_date.is_some() {
                dated
                    .entry(fact.subject_entity_id.clone())
                    .or_default()
                    .push((scene, fact));
            }
        }
    }

    let mut edges = Vec::new();
    for (identity_id, mut facts) in dated {
        let distinct_scenes: BTreeSet<&str> = facts
            .iter()
            .map(|(scene, _)| scene.scene_id.as_str())
            .collect();
        if distinct_scenes.len() < 2 {
            continue;
        }
        if !anchors.contains_key(&identity_id) {
            continue;
        }
        facts.sort_by(|left, right| {
            let left_date = left.1.normalized_date.as_deref().unwrap_or_default();
            let right_date = right.1.normalized_date.as_deref().unwrap_or_default();
            left_date
                .cmp(right_date)
                .then(
                    scene_order
                        .get(left.0.scene_id.as_str())
                        .cmp(&scene_order.get(right.0.scene_id.as_str())),
                )
                .then(left.1.start_byte.cmp(&right.1.start_byte))
        });
        for pair in facts.windows(2) {
            let (from_scene, from) = pair[0];
            let (to_scene, to) = pair[1];
            let from_order = scene_order
                .get(from_scene.scene_id.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            let to_order = scene_order
                .get(to_scene.scene_id.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            edges.push(StoryEdge {
                id: format!("chronological:{}:{}", from.id, to.id),
                kind: "chronological".to_string(),
                from_id: fact_node_id(&from_scene.scene_id, &from.id),
                to_id: fact_node_id(&to_scene.scene_id, &to.id),
                cross_scene: from_order != to_order,
                evidence: to.evidence.clone(),
                start_byte: to.evidence_start_byte,
                end_byte: to.evidence_end_byte,
            });
        }
    }
    edges
}

fn fact_node_id(scene_id: &str, fact_id: &str) -> String {
    format!("fact:{scene_id}:{fact_id}")
}

fn fact_node(scene_id: &str, fact: &EntityFact) -> StoryNode {
    StoryNode {
        id: fact_node_id(scene_id, &fact.id),
        kind: "fact".to_string(),
        label: fact.value.clone(),
        scene_id: scene_id.to_string(),
        scene_ids: vec![scene_id.to_string()],
        entity_id: Some(fact.subject_entity_id.clone()),
        fact_id: Some(fact.id.clone()),
        normalized_date: fact.normalized_date.clone(),
        sentence_index: Some(fact.sentence_index),
        evidence: fact.evidence.clone(),
        start_byte: fact.evidence_start_byte,
        end_byte: fact.evidence_end_byte,
    }
}

/// Reports, per scene, which grounded facts a proposal already represents.
///
/// A fact is covered when a proposal *presents* its content: a chart of the
/// fact's exact value span, or a timeline (including the one-event date card)
/// over a fact that carries a date. An `entity_search` proposal is an asset
/// query and an `entity_card` presents the subject rather than the claim, so
/// neither masks an uncovered fact; sharing a sentence is not a representation
/// either, and a timeline never covers a fact that has no date.
pub fn build_visual_coverage(scenes: &[SceneManifest]) -> VisualCoverage {
    // Only a chart or a timeline presents a fact's own content. A card or a
    // co-mention presents the subject, which is why it does not count: else
    // almost every fact would look covered and the report would say nothing.
    const PRESENTATION_KINDS: [&str; 2] = ["chart", "timeline"];
    let mut coverage = VisualCoverage::default();
    for scene in scenes {
        let mut facts: Vec<CoverageItem> = Vec::new();
        for fact in &scene.entity_facts {
            let mut proposal_types: Vec<String> = Vec::new();
            for proposal in &scene.visual_proposals {
                if !PRESENTATION_KINDS.contains(&proposal.proposal_type.as_str()) {
                    continue;
                }
                let charts_value = proposal.chart_items.iter().any(|item| {
                    item.start_byte == fact.start_byte && item.end_byte == fact.end_byte
                });
                let covers_date = proposal.proposal_type == "timeline"
                    && fact.normalized_date.is_some()
                    && proposal.start_byte <= fact.start_byte
                    && fact.end_byte <= proposal.end_byte;
                if (charts_value || covers_date)
                    && !proposal_types
                        .iter()
                        .any(|kind| kind == &proposal.proposal_type)
                {
                    proposal_types.push(proposal.proposal_type.clone());
                }
            }
            coverage.total_facts += 1;
            if !proposal_types.is_empty() {
                coverage.covered_facts += 1;
            }
            facts.push(CoverageItem {
                fact_id: fact.id.clone(),
                predicate: fact.predicate.clone(),
                subject_entity_id: fact.subject_entity_id.clone(),
                importance: fact.importance,
                status: if proposal_types.is_empty() {
                    "uncovered".to_string()
                } else {
                    "covered".to_string()
                },
                proposal_types,
                sentence_index: fact.sentence_index,
                evidence: fact.evidence.clone(),
                start_byte: fact.start_byte,
                end_byte: fact.end_byte,
            });
        }
        facts.sort_by(|left, right| {
            right
                .importance
                .total_cmp(&left.importance)
                .then(left.start_byte.cmp(&right.start_byte))
        });
        coverage.scenes.push(SceneCoverage {
            scene_id: scene.scene_id.clone(),
            facts,
        });
    }
    coverage
}
