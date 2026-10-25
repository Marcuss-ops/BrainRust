//! Scene-local and cross-scene explicit process phases.

use super::super::*;

/// Links explicit phases only when the same grounded entity is named in each
/// source sentence, phase order is increasing, and every assertion is certain
/// and positive. This is a scene-local process group; cross-scene continuation
/// requires scene-addressed members and is intentionally a separate contract
/// change.
pub(crate) fn process_groups(
    scene_id: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<SemanticGroup> {
    let mut phases: BTreeMap<String, Vec<(usize, SemanticGroupMember, String)>> = BTreeMap::new();
    let mut invalid = BTreeSet::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let lower = text.to_lowercase();
        if fact_status_lower(&lower) != "asserted" {
            continue;
        }
        let Some((phase, phase_id)) = process_phase(&lower) else {
            continue;
        };
        for (entity_index, entity) in entities.iter().enumerate() {
            if entity.start < sentence.start_byte || entity.end > sentence.end_byte {
                continue;
            }
            if matches!(
                entity.r#type.as_str(),
                "NUMBER" | "PERCENT" | "MONEY" | "DATE"
            ) {
                continue;
            }
            let identity_id = identities.id_of(entity_index);
            let members = phases.entry(identity_id.clone()).or_default();
            if let Some((previous, _, _)) = members.last() {
                if *previous == phase {
                    continue;
                }
                if *previous > phase {
                    invalid.insert(identity_id);
                    continue;
                }
            }
            members.push((
                phase,
                SemanticGroupMember {
                    candidate_id: format!("process_{}_{}", identity_id, phase_id),
                    scene_id: scene_id.to_string(),
                    rank: None,
                    sentence_index,
                    text: text.to_string(),
                    evidence: text.to_string(),
                    start_byte: sentence.start_byte,
                    end_byte: sentence.end_byte,
                    temporal_surface: None,
                    normalized_date: None,
                },
                entity.text.clone(),
            ));
        }
    }

    phases
        .into_iter()
        .filter_map(|(identity_id, members)| {
            if invalid.contains(&identity_id) || members.len() < 3 {
                return None;
            }
            let title_entity = &members[0].2;
            Some(SemanticGroup {
                id: format!("process_{}", identity_id),
                group_type: "process".to_string(),
                title: format!("Processo: {title_entity}"),
                ranking_basis: Some("explicit_source_phase_order".to_string()),
                target_count: None,
                complete: None,
                members: members.into_iter().map(|(_, member, _)| member).collect(),
            })
        })
        .collect()
}

pub(crate) fn process_phase(sentence_lower: &str) -> Option<(usize, String)> {
    lexicon::lexicon()
        .process_phases
        .phases
        .iter()
        .find(|phase| {
            phase
                .markers
                .iter()
                .any(|marker| sentence_lower.contains(marker.as_str()))
        })
        .map(|phase| (phase.order, phase.name.clone()))
}
pub(crate) fn add_cross_scene_process_groups(
    input_scenes: &[SceneInput],
    manifests: &mut [SceneManifest],
    language: &str,
) {
    let mut phases: BTreeMap<String, Vec<(usize, usize, SemanticGroupMember, String)>> =
        BTreeMap::new();
    let mut invalid = BTreeSet::new();
    for (scene_index, (input, manifest)) in input_scenes.iter().zip(manifests.iter()).enumerate() {
        if manifest.status != "valid" {
            continue;
        }
        let sentences = semantic_core::split_sentences(&input.text, language, None);
        for (sentence_index, sentence) in sentences.iter().enumerate() {
            let Some(text) = input.text.get(sentence.start_byte..sentence.end_byte) else {
                continue;
            };
            let lower = text.to_lowercase();
            if fact_status_lower(&lower) != "asserted" {
                continue;
            }
            let Some((phase, phase_id)) = process_phase(&lower) else {
                continue;
            };
            for entity in manifest.entities.iter().filter(|entity| {
                entity.start_byte >= sentence.start_byte
                    && entity.end_byte <= sentence.end_byte
                    && !matches!(
                        entity.kind.as_str(),
                        "NUMBER" | "PERCENT" | "MONEY" | "DATE"
                    )
            }) {
                let members = phases.entry(entity.id.clone()).or_default();
                if members
                    .last()
                    .is_some_and(|(previous, _, _, _)| *previous == phase)
                {
                    continue;
                }
                if members
                    .last()
                    .is_some_and(|(previous, _, _, _)| *previous > phase)
                {
                    invalid.insert(entity.id.clone());
                    continue;
                }
                members.push((
                    phase,
                    scene_index,
                    SemanticGroupMember {
                        candidate_id: format!("cross_process_{}_{}", entity.id, phase_id),
                        scene_id: input.scene_id.clone(),
                        rank: None,
                        sentence_index,
                        text: text.to_string(),
                        evidence: text.to_string(),
                        start_byte: sentence.start_byte,
                        end_byte: sentence.end_byte,
                        temporal_surface: None,
                        normalized_date: None,
                    },
                    entity.text.clone(),
                ));
            }
        }
    }

    for (identity_id, members) in phases {
        if invalid.contains(&identity_id) || members.len() < 3 {
            continue;
        }
        let distinct_scenes: BTreeSet<usize> =
            members.iter().map(|(_, scene, _, _)| *scene).collect();
        if distinct_scenes.len() < 2 {
            continue;
        }
        // A complete local group is already a better representation of the
        // same process. Avoid publishing overlapping local and global groups.
        let local_group_id = format!("process_{identity_id}");
        if manifests.iter().any(|manifest| {
            manifest
                .semantic_groups
                .iter()
                .any(|group| group.id == local_group_id)
        }) {
            continue;
        }
        let first_scene = members[0].1;
        let entity_name = &members[0].3;
        manifests[first_scene].semantic_groups.push(SemanticGroup {
            id: format!("cross_process_{identity_id}"),
            group_type: "process".to_string(),
            title: format!("Processo: {entity_name}"),
            ranking_basis: Some("explicit_source_phase_order".to_string()),
            target_count: None,
            complete: None,
            members: members
                .into_iter()
                .map(|(_, _, member, _)| member)
                .collect(),
        });
    }
}
