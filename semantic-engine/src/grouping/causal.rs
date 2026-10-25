//! Asserted causal relations, scene-local groups and cross-scene chains.

use super::super::*;

pub(crate) fn causal_groups(
    scene_id: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    identities: &IdentityLookup,
    relations: &RelationView,
) -> Vec<SemanticGroup> {
    let mut groups = Vec::new();
    for relation in relations.asserted_relations.iter().filter(|relation| {
        relation.predicate.as_deref() == Some("causal")
            && relation.status == RelationStatus::Asserted
    }) {
        let Some(sentence) = sentences.get(relation.sentence_index) else {
            continue;
        };
        let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let endpoints = [relation.source_id.as_str(), relation.target_id.as_str()];
        let mut members = Vec::new();
        for endpoint in endpoints {
            let Some((entity_index, entity)) =
                entities.iter().enumerate().find(|(index, entity)| {
                    identities.id_of(*index) == endpoint
                        && entity.start >= sentence.start_byte
                        && entity.end <= sentence.end_byte
                        && is_visual_subject(entity)
                })
            else {
                members.clear();
                break;
            };
            let role = if members.is_empty() {
                "cause"
            } else {
                "effect"
            };
            members.push(SemanticGroupMember {
                candidate_id: format!("{role}_{}", identities.id_of(entity_index)),
                scene_id: scene_id.to_string(),
                rank: None,
                sentence_index: relation.sentence_index,
                text: entity.text.clone(),
                evidence: entity.evidence.clone(),
                start_byte: entity.start,
                end_byte: entity.end,
                temporal_surface: None,
                normalized_date: None,
            });
        }
        if members.len() != 2 || evidence.is_empty() {
            continue;
        }
        groups.push(SemanticGroup {
            id: format!("cause_effect_{}_{}", scene_id, groups.len() + 1),
            group_type: "cause_effect".to_string(),
            title: format!("{} → {}", members[0].text, members[1].text),
            ranking_basis: Some("explicit_asserted_causal_relation".to_string()),
            target_count: Some(2),
            complete: Some(true),
            members,
        });
    }
    groups
}
/// Joins explicit causal edges into a cross-scene chain only when the first
/// edge's effect is the next edge's cause. Both edges must already have
/// grounded, asserted scene-local groups; co-mentions and uncertain language
/// cannot enter this projection.
pub(crate) fn add_cross_scene_causal_groups(
    input_scenes: &[SceneInput],
    manifests: &mut [SceneManifest],
) {
    let mut edges = Vec::new();
    for (scene_index, manifest) in manifests.iter().enumerate() {
        for group in manifest
            .semantic_groups
            .iter()
            .filter(|group| group.group_type == "cause_effect" && group.members.len() == 2)
        {
            let Some(source_id) = group.members[0].candidate_id.strip_prefix("cause_") else {
                continue;
            };
            let Some(target_id) = group.members[1].candidate_id.strip_prefix("effect_") else {
                continue;
            };
            edges.push((
                scene_index,
                source_id.to_string(),
                target_id.to_string(),
                group.members.clone(),
            ));
        }
    }

    let mut successors: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (edge_index, edge) in edges.iter().enumerate() {
        successors
            .entry(edge.1.clone())
            .or_default()
            .push(edge_index);
    }
    let mut chains = Vec::new();
    for first in &edges {
        if let Some(next_indices) = successors.get(&first.2) {
            for &next_index in next_indices {
                let second = &edges[next_index];
                if first.0 < second.0 {
                    chains.push((first.clone(), second.clone()));
                }
            }
        }
    }

    for (first, second) in chains {
        let first_scene = first.0;
        let mut members = first.3;
        members.extend(second.3);
        for member in &mut members {
            member.candidate_id = format!(
                "causal_chain_evidence_{}_{}_{}",
                member.scene_id, member.candidate_id, member.start_byte
            );
        }
        let title = format!(
            "{} → {} → {}",
            members[0].text, members[1].text, members[3].text
        );
        manifests[first_scene].semantic_groups.push(SemanticGroup {
            id: format!(
                "cross_cause_effect_{}_{}_{}_{}_{}",
                first.1,
                first.2,
                second.2,
                input_scenes[first.0].scene_id,
                input_scenes[second.0].scene_id
            ),
            group_type: "cause_effect".to_string(),
            title,
            ranking_basis: Some("explicit_asserted_causal_chain_across_scenes".to_string()),
            target_count: None,
            complete: None,
            members,
        });
    }
}
