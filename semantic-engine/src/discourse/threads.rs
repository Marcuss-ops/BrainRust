//! Entity threads and scene-level continuity.
//!
//! A thread is a chain of resolved references to the same identity. The module
//! does not infer a topic label: scene continuity is the explicit Jaccard
//! overlap of resolved entity ids, which a planner can use as a deterministic
//! signal without pretending it is a semantic probability.

use std::collections::{BTreeMap, BTreeSet};

use semantic_core::narrative::{DiscourseReference, EntityThread, SceneContinuity, ThreadBeat};
use semantic_core::SceneManifest;

pub(super) fn build(
    references: &[DiscourseReference],
    scenes: &[SceneManifest],
) -> (Vec<EntityThread>, Vec<SceneContinuity>) {
    let scene_order: BTreeMap<&str, usize> = scenes
        .iter()
        .enumerate()
        .map(|(index, scene)| (scene.scene_id.as_str(), index))
        .collect();
    let mut labels: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    for scene in scenes {
        for identity in &scene.identities {
            labels
                .entry(identity.id.as_str())
                .or_insert((identity.canonical_surface.as_str(), identity.kind.as_str()));
        }
    }

    let mut per_entity: BTreeMap<String, Vec<ThreadBeat>> = BTreeMap::new();
    for reference in references {
        if reference.status != "resolved" {
            continue;
        }
        for entity_id in &reference.target_entity_ids {
            let beats = per_entity.entry(entity_id.clone()).or_default();
            let order = scene_order
                .get(reference.scene_id.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            if beats.iter().any(|beat| {
                scene_order
                    .get(beat.scene_id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
                    == order
                    && beat.sentence_index == reference.sentence_index
            }) {
                continue;
            }
            let previous = beats.last().map(|beat| {
                (
                    scene_order
                        .get(beat.scene_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX),
                    beat.sentence_index,
                )
            });
            let state = match previous {
                None => "starts",
                Some((previous_order, previous_sentence)) => {
                    let skipped_scene = order.saturating_sub(previous_order) >= 2;
                    let skipped_sentence =
                        order == previous_order && reference.sentence_index > previous_sentence + 1;
                    if skipped_scene || skipped_sentence {
                        "returns"
                    } else {
                        "continues"
                    }
                }
            };
            beats.push(ThreadBeat {
                reference_id: reference.id.clone(),
                scene_id: reference.scene_id.clone(),
                sentence_index: reference.sentence_index,
                state: state.to_string(),
            });
        }
    }

    let mut threads = Vec::new();
    for (entity_id, beats) in per_entity {
        if beats.len() < 2 {
            continue;
        }
        let Some((canonical_surface, kind)) = labels.get(entity_id.as_str()).copied() else {
            continue;
        };
        let scene_ids: Vec<String> = {
            let mut seen = BTreeSet::new();
            let mut ids = Vec::new();
            for beat in &beats {
                if seen.insert(beat.scene_id.clone()) {
                    ids.push(beat.scene_id.clone());
                }
            }
            ids
        };
        let Some(first_scene_id) = scene_ids.first().cloned() else {
            continue;
        };
        let Some(last_scene_id) = scene_ids.last().cloned() else {
            continue;
        };
        threads.push(EntityThread {
            entity_id,
            canonical_surface: canonical_surface.to_string(),
            entity_type: kind.to_string(),
            scene_ids,
            first_scene_id,
            last_scene_id,
            mention_count: beats.len(),
            return_count: beats.iter().filter(|beat| beat.state == "returns").count(),
            beats,
        });
    }
    threads.sort_by(|left, right| {
        left.first_scene_id
            .cmp(&right.first_scene_id)
            .then(left.entity_id.cmp(&right.entity_id))
    });

    let mut by_scene: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for reference in references {
        if reference.status != "resolved" {
            continue;
        }
        let active = by_scene.entry(reference.scene_id.as_str()).or_default();
        for entity_id in &reference.target_entity_ids {
            active.insert(entity_id.clone());
        }
    }
    let mut scene_continuity = Vec::new();
    let mut previous: BTreeSet<String> = BTreeSet::new();
    for scene in scenes {
        let current = by_scene
            .get(scene.scene_id.as_str())
            .cloned()
            .unwrap_or_default();
        let shared: BTreeSet<String> = current.intersection(&previous).cloned().collect();
        let union: BTreeSet<String> = current.union(&previous).cloned().collect();
        let entered: BTreeSet<String> = current.difference(&previous).cloned().collect();
        let exited: BTreeSet<String> = previous.difference(&current).cloned().collect();
        let status = if previous.is_empty() {
            "start"
        } else if current.is_empty() {
            "empty"
        } else if shared.is_empty() {
            "shift"
        } else {
            "continues"
        };
        scene_continuity.push(SceneContinuity {
            scene_id: scene.scene_id.clone(),
            status: status.to_string(),
            score: if union.is_empty() {
                0.0
            } else {
                shared.len() as f32 / union.len() as f32
            },
            active_entity_ids: current.into_iter().collect(),
            shared_entity_ids: shared.into_iter().collect(),
            entered_entity_ids: entered.into_iter().collect(),
            exited_entity_ids: exited.into_iter().collect(),
        });
        previous = by_scene
            .get(scene.scene_id.as_str())
            .cloned()
            .unwrap_or_default();
    }
    (threads, scene_continuity)
}
