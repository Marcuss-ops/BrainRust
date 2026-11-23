//! Cross-scene event merging.

use semantic_core::EventFrame;

/// Reconciles repeated, asserted event mentions across scenes. A shared
/// participant is required, as is an exact shared date or place; conflicting
/// explicit dates prevent a merge.
pub(crate) fn merge_cross_scene(
    frames: Vec<EventFrame>,
    threads: &[semantic_core::EntityThread],
) -> Vec<EventFrame> {
    let mut merged: Vec<EventFrame> = Vec::new();
    for mut frame in frames {
        let match_index = merged.iter().position(|existing| {
            existing.event_type == frame.event_type
                && existing.status == "asserted"
                && frame.status == "asserted"
                && shares_threaded_participant(existing, &frame, threads)
                && compatible_dates(existing, &frame)
                && (same_date(existing, &frame) || same_place(existing, &frame))
        });
        if let Some(index) = match_index {
            let existing = &mut merged[index];
            for scene_id in frame.scene_ids.drain(..) {
                if !existing.scene_ids.contains(&scene_id) {
                    existing.scene_ids.push(scene_id);
                }
            }
            existing.evidence.append(&mut frame.evidence);
            existing.source_fact_ids.append(&mut frame.source_fact_ids);
            existing.source_fact_ids.sort();
            existing.source_fact_ids.dedup();
            for basis in frame.confidence_basis {
                if !existing.confidence_basis.contains(&basis) {
                    existing.confidence_basis.push(basis);
                }
            }
        } else {
            merged.push(frame);
        }
    }
    merged
}

fn shares_threaded_participant(
    left: &EventFrame,
    right: &EventFrame,
    threads: &[semantic_core::EntityThread],
) -> bool {
    left.participants.iter().any(|a| {
        right.participants.iter().any(|b| {
            a.entity_id == b.entity_id
                && threads.iter().any(|thread| {
                    thread.entity_id == a.entity_id
                        && left
                            .scene_ids
                            .iter()
                            .any(|scene| thread.scene_ids.contains(scene))
                        && right
                            .scene_ids
                            .iter()
                            .any(|scene| thread.scene_ids.contains(scene))
                })
        })
    })
}

fn date(frame: &EventFrame) -> Option<&str> {
    frame.time.as_ref()?.normalized_date.as_deref()
}

fn compatible_dates(left: &EventFrame, right: &EventFrame) -> bool {
    !matches!((date(left), date(right)), (Some(a), Some(b)) if a != b)
}

fn same_date(left: &EventFrame, right: &EventFrame) -> bool {
    matches!((date(left), date(right)), (Some(a), Some(b)) if a == b)
}

fn same_place(left: &EventFrame, right: &EventFrame) -> bool {
    matches!((&left.place_id, &right.place_id), (Some(a), Some(b)) if a == b)
}
