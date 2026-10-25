//! Explicit comparisons between named subjects, scene-local and cross-scene.

use super::super::*;

/// A comparison is only drawn between named subjects: two generic nouns
/// ("colpevolezza vs diritto") are not a visual contrast, because nothing in
/// the text says the two are comparable.
pub(crate) fn is_named_comparison_subject(entity: &VisualEntity) -> bool {
    groups::is_named_subject(entity)
}

/// Two named subjects separated by an explicit cue: the only shape the engine
/// proposes as a comparison. The old rule took *every* entity of a sentence
/// that contained "rispetto a", so a paragraph listing eleven nouns became an
/// eleven-way comparison. Here the cue is located first (byte-exact, from the
/// shared discourse vocabulary), then the nearest named subject on each side
/// is taken, so the group always has exactly two members.
pub(crate) fn comparison_groups(
    scene_id: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<SemanticGroup> {
    let mut groups = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if fact_status_lower(&text.to_lowercase()) != "asserted" {
            continue;
        }
        let Some((cue_start, cue_end)) = semantic_core::discourse::comparison_cue(text) else {
            continue;
        };
        let cue_start = sentence.start_byte + cue_start;
        let cue_end = sentence.start_byte + cue_end;
        let words = semantic_core::discourse::word_spans(text);
        let in_sentence = |entity: &VisualEntity| {
            entity.start >= sentence.start_byte
                && entity.end <= sentence.end_byte
                && is_named_comparison_subject(entity)
        };
        // Nearest named subject before the cue, and the first one after it.
        let left = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| in_sentence(entity) && entity.end <= cue_start)
            .max_by_key(|(_, entity)| entity.end);
        let right = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| in_sentence(entity) && entity.start >= cue_end)
            .min_by_key(|(_, entity)| entity.start);
        let (Some((left_index, left_entity)), Some((right_index, right_entity))) = (left, right)
        else {
            continue;
        };
        if identities.id_of(left_index) == identities.id_of(right_index) {
            continue;
        }
        let left_gap = semantic_core::discourse::words_between(
            &words,
            left_entity.end - sentence.start_byte,
            cue_start - sentence.start_byte,
        );
        let right_gap = semantic_core::discourse::words_between(
            &words,
            cue_end - sentence.start_byte,
            right_entity.start - sentence.start_byte,
        );
        if left_gap > COMPARISON_CUE_GAP || right_gap > COMPARISON_CUE_GAP {
            continue;
        }
        let members: Vec<SemanticGroupMember> =
            [(left_index, left_entity), (right_index, right_entity)]
                .into_iter()
                .map(|(entity_index, entity)| SemanticGroupMember {
                    candidate_id: format!("comparison_{}", identities.id_of(entity_index)),
                    scene_id: scene_id.to_string(),
                    rank: None,
                    sentence_index,
                    text: entity.text.clone(),
                    evidence: entity.evidence.clone(),
                    start_byte: entity.start,
                    end_byte: entity.end,
                    temporal_surface: None,
                    normalized_date: None,
                })
                .collect();
        let title = format!(
            "Confronto: {}",
            members
                .iter()
                .map(|member| member.text.as_str())
                .collect::<Vec<_>>()
                .join(" vs ")
        );
        groups.push(SemanticGroup {
            id: format!("comparison_{}_{}", scene_id, groups.len() + 1),
            group_type: "comparison".to_string(),
            title,
            ranking_basis: Some("explicit_source_comparison".to_string()),
            target_count: Some(2),
            complete: Some(true),
            members,
        });
    }
    groups
}
/// Consolidates explicit comparisons of the same canonical pair when both
/// identities are compared in at least two scenes. Each member keeps its own
/// scene-local mention span so Go can independently ground the full group.
pub(crate) fn add_cross_scene_comparison_groups(
    input_scenes: &[SceneInput],
    manifests: &mut [SceneManifest],
) {
    let mut comparisons: BTreeMap<Vec<String>, Vec<(usize, Vec<SemanticGroupMember>)>> =
        BTreeMap::new();
    for (scene_index, manifest) in manifests.iter().enumerate() {
        for group in manifest
            .semantic_groups
            .iter()
            .filter(|group| group.group_type == "comparison")
        {
            let mut identity_ids: Vec<String> = group
                .members
                .iter()
                .filter_map(|member| {
                    member
                        .candidate_id
                        .strip_prefix("comparison_")
                        .map(str::to_string)
                })
                .collect();
            identity_ids.sort();
            identity_ids.dedup();
            if identity_ids.len() < 2 || identity_ids.len() != group.members.len() {
                continue;
            }
            let mut members = group.members.clone();
            for member in &mut members {
                member.candidate_id = format!(
                    "comparison_evidence_{}_{}_{}",
                    input_scenes[scene_index].scene_id, member.candidate_id, member.start_byte
                );
            }
            comparisons
                .entry(identity_ids)
                .or_default()
                .push((scene_index, members));
        }
    }

    for (identity_ids, evidence) in comparisons {
        let scene_indices: BTreeSet<usize> = evidence.iter().map(|(index, _)| *index).collect();
        if scene_indices.len() < 2 {
            continue;
        }
        let first_scene = *scene_indices.first().expect("two scenes were checked");
        let mut members = Vec::new();
        for (_, pair) in evidence {
            members.extend(pair);
        }
        if members.len() < 4 {
            continue;
        }
        let title = format!(
            "Confronto: {}",
            members
                .iter()
                .take(identity_ids.len())
                .map(|member| member.text.as_str())
                .collect::<Vec<_>>()
                .join(" vs ")
        );
        manifests[first_scene].semantic_groups.push(SemanticGroup {
            id: format!("cross_comparison_{}", identity_ids.join("_")),
            group_type: "comparison".to_string(),
            title,
            ranking_basis: Some("explicit_pair_comparison_across_scenes".to_string()),
            target_count: None,
            complete: None,
            members,
        });
    }
}
