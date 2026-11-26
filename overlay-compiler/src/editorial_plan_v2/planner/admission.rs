//! Admission and pacing: the budget passes, the cascade and the overlap
//! resolution. This is where an item can be dropped, so every drop records a
//! reason instead of disappearing.

use super::*;

pub(super) fn new_intake(request: &ValidatedRequest<'_>, candidates: &[PlanItemV2]) -> IntakeV2 {
    let budgets = request.budgets();
    let mut intake = IntakeV2 {
        scenes: request.scenes.len() as i64,
        timed_scenes: request
            .scenes
            .iter()
            .filter(|scene| scene.duration_us > 0)
            .count() as i64,
        budget: IntakeBudgetV2 {
            phrase_overlays: budgets.phrase_overlays,
            image_overlays: budgets.image_overlays,
            map_overlays: budgets.map_overlays,
            max_overlays_per_scene: budgets.max_per_scene,
        },
        candidates: candidates.len() as i64,
        ..Default::default()
    };
    for item in candidates {
        *intake
            .candidates_by_kind
            .entry(item.kind.clone())
            .or_default() += 1;
    }
    intake
}

/// Explicit admission priority. Every ceiling is a finite resource, so the
/// kind that matters must be considered first; append order is not a policy.
/// Lower ranks win: a verified portrait or a map beats a generic still, a
/// text surface beats a date. Within one rank the request order is preserved,
/// so each run budget is still spent scene by scene and the old two-pass
/// append order no longer decides who lives (Augias R41: a full scene of
/// dates erased every verified portrait because cards were appended last).
fn admission_rank(item: &PlanItemV2) -> u8 {
    if is_map(item) {
        return 0;
    }
    match item.kind.as_str() {
        "entity_image" | "entity_card" | "organization" | "location" | "concept" => 0,
        "image" | "product" | "logo" => 1,
        "number" => 3,
        _ => 2,
    }
}

/// Admits every candidate exactly once. The selection walks the candidates in
/// [`admission_rank`] order and the survivors are returned in request order, so
/// the plan keeps its editorial ordering while the priority decides the cuts.
pub(super) fn admit(
    request: &ValidatedRequest<'_>,
    candidates: Vec<PlanItemV2>,
    mut intake: IntakeV2,
    decisions: &mut Vec<PlanDecisionV2>,
) -> (Vec<PlanItemV2>, IntakeV2) {
    let budgets = request.budgets();
    let max_per_scene = if budgets.max_per_scene > 0 {
        budgets.max_per_scene
    } else {
        request.canvas.max_overlays_per_scene
    };
    let mut group_counts: BTreeMap<&'static str, i64> = BTreeMap::new();
    let mut scene_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut phrases = 0;
    let mut images = 0;
    let mut maps = 0;
    let mut bullets = 0;
    let mut keep = vec![false; candidates.len()];
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|index| (admission_rank(&candidates[*index]), *index));
    for index in order {
        let item = &candidates[index];
        if request.canvas.maps_only && !is_map(item) {
            drop_candidate(&mut intake, decisions, "maps_only", item);
            continue;
        }
        if category_disabled(request, item) {
            drop_candidate(&mut intake, decisions, "category", item);
            continue;
        }
        let group = group_for(item);
        if request.canvas.group_limits.get(group).is_some_and(|limit| {
            *limit > 0 && group_counts.get(group).copied().unwrap_or(0) >= *limit
        }) {
            drop_candidate(&mut intake, decisions, "group", item);
            continue;
        }
        if item.kind == "text_phrase" {
            if budgets.phrase_overlays > 0 && phrases >= budgets.phrase_overlays {
                drop_candidate(&mut intake, decisions, "budget", item);
                continue;
            }
            if role(item) == "scene_bullet" && budgets.bullet > 0 && bullets >= budgets.bullet {
                drop_candidate(&mut intake, decisions, "budget", item);
                continue;
            }
        }
        if is_image(item) && budgets.image_overlays > 0 && images >= budgets.image_overlays {
            // Verified portraits were considered first, so a generic still
            // that reaches this branch lost the ceiling to them.
            drop_candidate(&mut intake, decisions, "budget", item);
            continue;
        }
        if is_map(item) && budgets.map_overlays > 0 && maps >= budgets.map_overlays {
            drop_candidate(&mut intake, decisions, "budget", item);
            continue;
        }
        if max_per_scene > 0
            && scene_counts
                .get(item.scene_id.as_str())
                .copied()
                .unwrap_or(0)
                >= max_per_scene
        {
            // A named card was considered first, so a date that reaches this
            // branch lost its slot to the identity, not to another date.
            drop_candidate(&mut intake, decisions, "per_scene", item);
            continue;
        }
        keep[index] = true;
        *group_counts.entry(group).or_default() += 1;
        *scene_counts.entry(item.scene_id.clone()).or_default() += 1;
        if item.kind == "text_phrase" {
            phrases += 1;
            if role(item) == "scene_bullet" {
                bullets += 1;
            }
        }
        if is_image(item) {
            images += 1;
        }
        if is_map(item) {
            maps += 1;
        }
    }
    let mut selected = Vec::with_capacity(keep.iter().filter(|kept| **kept).count());
    for (index, item) in candidates.into_iter().enumerate() {
        if !keep[index] {
            continue;
        }
        *intake.planned_by_kind.entry(item.kind.clone()).or_default() += 1;
        selected.push(item);
    }
    intake.planned = selected.len() as i64;
    (selected, intake)
}
fn drop_reason(intake: &mut IntakeV2, reason: &str) {
    *intake
        .dropped_by_reason
        .entry(reason.to_string())
        .or_default() += 1;
}

/// Drops one candidate and records both the run-level count and the typed
/// per-candidate decision, so the two projections cannot disagree.
fn drop_candidate(
    intake: &mut IntakeV2,
    decisions: &mut Vec<PlanDecisionV2>,
    reason: &str,
    item: &PlanItemV2,
) {
    drop_reason(intake, reason);
    push_decision(
        decisions,
        STAGE_ADMISSION,
        admission_reason_code(reason),
        item,
    );
}

pub(super) fn role(item: &PlanItemV2) -> &str {
    item.params
        .as_ref()
        .and_then(|params| params.get("editorial_role"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn category_for(item: &PlanItemV2) -> &'static str {
    match item.kind.as_str() {
        "text_phrase" => match role(item) {
            "scene_title" => "titles",
            "scene_bullet" | "chapter_bullet" => "bullets",
            _ => "phrases",
        },
        "image" | "entity_image" | "product" | "logo" => "images",
        "map" => "maps",
        "number" => "numbers",
        "quote" => "quotes",
        "brand_text" => "brand_text",
        "entity_card" | "organization" | "location" | "concept" => "entities",
        _ => "phrases",
    }
}

/// One category-toggle lookup for both admission and the card builder. The
/// card builder needs it before attaching a portrait: an `images: disabled`
/// scene must keep a text card, not become an `entity_image` that admission
/// then drops.
pub(super) fn category_toggle_disabled(
    request: &EditorialPlanV2Request,
    scene_id: &str,
    category: &str,
) -> bool {
    // A run-level disable is an operator-wide safety switch. A scene's
    // generated `enabled` value must not turn that category back on.
    let run_toggle = request.canvas.overlay_categories.get(category);
    if run_toggle.is_some_and(|toggle| toggle == "disabled") {
        return true;
    }

    let scene = request.scenes.iter().find(|scene| scene.id == scene_id);
    if category == "entities" {
        // Older payloads used phrases=disabled as a broad editorial switch,
        // which also suppressed text-only entity cards because they shared
        // that category. Preserve that behavior only when no explicit
        // entities toggle is supplied; new payloads can control the two lanes
        // independently.
        let scene_entity_toggle = scene.and_then(|scene| scene.overlay_categories.get(category));
        let run_entity_toggle_is_explicit =
            run_toggle.is_some_and(|toggle| !toggle.is_empty() && toggle != "default");
        if !run_entity_toggle_is_explicit
            && request
                .canvas
                .overlay_categories
                .get("phrases")
                .is_some_and(|toggle| toggle == "disabled")
        {
            return true;
        }
        let explicit_entity_toggle = run_toggle
            .is_some_and(|toggle| !toggle.is_empty() && toggle != "default")
            || scene_entity_toggle.is_some_and(|toggle| !toggle.is_empty() && toggle != "default");
        if scene_entity_toggle.is_some_and(|toggle| toggle == "disabled") {
            return true;
        }
        if !explicit_entity_toggle {
            if request
                .canvas
                .overlay_categories
                .get("phrases")
                .is_some_and(|toggle| toggle == "disabled")
            {
                return true;
            }
            if scene.is_some_and(|scene| {
                scene
                    .overlay_categories
                    .get("phrases")
                    .is_some_and(|toggle| toggle == "disabled")
            }) {
                return true;
            }
        }
        return false;
    }

    if let Some(toggle) = scene.and_then(|scene| scene.overlay_categories.get(category)) {
        if !toggle.is_empty() && toggle != "default" {
            return toggle == "disabled";
        }
    }
    false
}

fn category_disabled(request: &EditorialPlanV2Request, item: &PlanItemV2) -> bool {
    category_toggle_disabled(request, &item.scene_id, category_for(item))
}

fn group_for(item: &PlanItemV2) -> &'static str {
    match item.kind.as_str() {
        "text_phrase" if role(item) == "scene_bullet" => "bullets",
        "text_phrase" => "phrases",
        // Numeric cards have their own admission lane. Sharing `phrases`
        // makes grounded dates and metrics consume the same caller cap as
        // editorial copy, so a dense scene can silently lose its chronology.
        "number" => "numbers",
        "image" | "entity_image" | "product" | "logo" => "images",
        "map" => "maps",
        "entity_card" | "organization" | "location" | "concept" => "entities",
        _ => "phrases",
    }
}

#[cfg(test)]
mod tests {
    use super::group_for;
    use crate::editorial_plan_v2::response::PlanItemV2;

    #[test]
    fn numeric_cards_do_not_consume_the_phrase_group_limit() {
        let phrase = PlanItemV2 {
            kind: "text_phrase".to_string(),
            ..Default::default()
        };
        let number = PlanItemV2 {
            kind: "number".to_string(),
            template_id: "TIMELINE_DATE_CARD".to_string(),
            ..Default::default()
        };

        assert_eq!("phrases", group_for(&phrase));
        assert_eq!("numbers", group_for(&number));
    }
}

pub(super) fn is_image(item: &PlanItemV2) -> bool {
    matches!(
        item.kind.as_str(),
        "image" | "entity_image" | "product" | "logo"
    )
}

fn is_map(item: &PlanItemV2) -> bool {
    item.kind == "map" && item.map.is_some()
}

pub(super) fn cascade(items: &mut [PlanItemV2], request: &EditorialPlanV2Request) {
    for item in items.iter_mut() {
        if item.kind == "entity_image" {
            // Entity cards keep the certified fast 8-frame snap even when
            // their one-second minimum window overlaps an earlier image.
            item.motion_params = Some(json!({"enter_frames": 8}));
            continue;
        }
        if matches!(
            item.kind.as_str(),
            "text_phrase" | "image" | "entity_image" | "number"
        ) {
            item.motion_params = Some(json!({"enter_frames": 8}));
        }
    }
    let entrance_items: Vec<CascadeEntranceItem> = items
        .iter()
        .map(|item| CascadeEntranceItem {
            id: item.id.clone(),
            kind: item.kind.clone(),
            template_id: item.template_id.clone(),
            start_ms: item.start_ms,
            end_ms: item.end_ms,
            motion_params: item.motion_params.clone(),
            image_layer_count: 0,
            asset_ref_count: item.asset_refs.len(),
            entity_caption: item.entity_caption.clone(),
            text: item.text.clone(),
            editorial_role: json!(role(item)),
        })
        .collect();
    let edits = cascade_clustered_entrances(CascadeEntrancesRequest {
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        items: entrance_items,
    });
    for edit in edits {
        if let Some(item) = items.get_mut(edit.index) {
            if item.kind == "entity_image" {
                continue;
            }
            let mut params = item.motion_params.take().unwrap_or_else(|| json!({}));
            if let Some(object) = params.as_object_mut() {
                object.insert("enter_frames".to_string(), json!(edit.enter_frames));
            }
            item.motion_params = Some(params);
        }
    }
}

pub(super) fn resolve_overlaps(
    request: &EditorialPlanV2Request,
    items: Vec<PlanItemV2>,
    decisions: &mut Vec<PlanDecisionV2>,
) -> Vec<PlanItemV2> {
    if items.is_empty() {
        return items;
    }
    let input: Vec<TimingItem> = items.iter().map(timing_item).collect();
    // Map presentation is the planner's decision from here on: the resolver
    // owns the camera minimum, the readable floor and the tail bound, so the
    // sealed plan already carries the window the renderer uses.
    let plan_end_us = request
        .scenes
        .iter()
        .map(|scene| scene.timeline_start_us.saturating_add(scene.duration_us))
        .max()
        .unwrap_or(0);
    let map_presentation =
        input
            .iter()
            .any(|item| item.has_map)
            .then_some(crate::MapPresentationWindow {
                camera_duration_us: super::map_groups::MAP_CAMERA_DURATION_US,
                camera_lead_us: super::map_groups::MAP_CAMERA_LEAD_US,
                pin_read_duration_us: super::map_groups::MAP_PIN_READ_DURATION_US,
                readable_floor_us: super::map_groups::MAP_READABLE_FLOOR_US,
                presentation_ceiling_us: super::map_groups::MAP_PRESENTATION_CEILING_US,
                plan_end_us,
            });
    let edits = crate::resolve_map_overlaps(crate::ResolveMapOverlapsRequest {
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        scene_windows: request.scene_windows.clone(),
        items: input,
        map_presentation,
    });
    if edits.len() != items.len() {
        return items;
    }
    let mut resolved = Vec::with_capacity(items.len());
    for (index, edit) in edits.into_iter().enumerate() {
        if !edit.keep {
            push_decision(decisions, STAGE_OVERLAP, REASON_OVERLAP_LOST, &items[index]);
            continue;
        }
        let mut item = items[index].clone();
        if edit.start_us != item.start_us || edit.duration_us != item.duration_us {
            item.start_us = edit.start_us;
            item.duration_us = edit.duration_us;
            item.start_ms = edit.start_ms;
            item.end_ms = edit.end_ms;
        }
        resolved.push(item);
    }
    resolved
}

/// Centered text cards cannot share the same visible window. This includes
/// phrases, entity cards and organization cards (r48: a `Prime Video Sport`
/// organization card covered the following `Francesco Totti` card). Go
/// re-anchors phrases to certified word timing after this plan is sealed, so
/// the movable entity card is shifted to the nearest free window. If no such
/// window exists, the entity card is dropped with a typed decision.
pub(super) fn resolve_entity_text_overlaps(
    request: &EditorialPlanV2Request,
    mut items: Vec<PlanItemV2>,
    decisions: &mut Vec<PlanDecisionV2>,
) -> Vec<PlanItemV2> {
    let scene_bounds: BTreeMap<&str, (i64, i64)> = request
        .scenes
        .iter()
        .map(|scene| {
            (
                scene.id.as_str(),
                (
                    scene.timeline_start_us,
                    scene.timeline_start_us.saturating_add(scene.duration_us),
                ),
            )
        })
        .collect();
    let mut dropped: BTreeSet<String> = BTreeSet::new();
    for entity_index in 0..items.len() {
        if !is_entity_card(&items[entity_index]) || dropped.contains(&items[entity_index].id) {
            continue;
        }
        let scene_id = items[entity_index].scene_id.clone();
        let (entity_start, entity_end) = items[entity_index].window_us();
        if entity_end <= entity_start {
            continue;
        }
        let blockers: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(index, item)| {
                *index != entity_index
                    && !dropped.contains(&item.id)
                    && item.scene_id == scene_id
                    && entity_text_conflict(entity_start, entity_end, item)
            })
            .map(|(index, _)| index)
            .collect();
        if blockers.is_empty() {
            continue;
        }
        let Some(&(scene_start, scene_end)) = scene_bounds.get(scene_id.as_str()) else {
            continue;
        };
        let duration = entity_end - entity_start;
        let mut candidates = Vec::with_capacity(blockers.len() * 2);
        for &blocker_index in &blockers {
            let (blocker_start, blocker_end) = items[blocker_index].window_us();
            // End before the blocker starts, or start after it ends.
            candidates.push(blocker_start - duration);
            candidates.push(blocker_end);
        }
        candidates.sort_by_key(|start| (start - entity_start).abs());
        candidates.dedup();
        let target = candidates.into_iter().find(|start| {
            *start >= scene_start
                && start.saturating_add(duration) <= scene_end
                && !overlaps_text_window(
                    &items,
                    &dropped,
                    entity_index,
                    &scene_id,
                    *start,
                    start + duration,
                )
        });
        if let Some(new_start) = target {
            let item = &mut items[entity_index];
            if let Some(params) = item.params.get_or_insert_with(|| json!({})).as_object_mut() {
                params.insert(
                    "occlusion_shift_us".to_string(),
                    json!(new_start - entity_start),
                );
            }
            item.start_us = new_start;
            item.start_ms = new_start / 1_000;
            item.end_ms = (new_start + duration + 999) / 1_000;
            item.duration_us = duration;
            continue;
        }
        // No free window: preserve the spoken/text surface and drop the entity
        // card rather than hide the phrase or another named card.
        let card = &items[entity_index];
        push_decision(
            decisions,
            STAGE_OVERLAP,
            REASON_OCCLUDED_BY_ENTITY_CARD,
            card,
        );
        dropped.insert(card.id.clone());
    }
    items
        .into_iter()
        .filter(|item| !dropped.contains(&item.id))
        .collect()
}

fn is_entity_card(item: &PlanItemV2) -> bool {
    item.kind == "entity_card"
}

fn is_plain_phrase(item: &PlanItemV2) -> bool {
    item.kind == "text_phrase" && item.template_id == "IMPORTANT_PHRASE" && role(item).is_empty()
}

fn entity_text_conflict(entity_start: i64, entity_end: i64, item: &PlanItemV2) -> bool {
    let (start, end) = item.window_us();
    if end <= start || entity_start >= end || start >= entity_end {
        return false;
    }
    match item.kind.as_str() {
        // Named labels and another person card occupy the same centered text
        // surface, so any overlap can hide part of one card.
        "entity_card" | "organization" | "location" | "concept" => true,
        // Go re-anchors phrase timing after the seal. Preserve that anchor and
        // move the entity only when it would completely cover the phrase.
        "text_phrase" if is_plain_phrase(item) => start >= entity_start && end <= entity_end,
        "number" | "quote" | "lower_third" => start >= entity_start && end <= entity_end,
        _ => false,
    }
}

/// A named entity card and an identical phrase at the same time say the same
/// thing twice. Keep the named card and remove the redundant phrase.
pub(super) fn drop_phrases_duplicated_by_named_cards(
    items: Vec<PlanItemV2>,
    decisions: &mut Vec<PlanDecisionV2>,
) -> Vec<PlanItemV2> {
    let named_cards: Vec<(String, String, String, i64, i64)> = items
        .iter()
        .filter(|item| matches!(item.kind.as_str(), "entity_card" | "organization"))
        .map(|card| {
            let normalized = card
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let (start, end) = card.window_us();
            (
                card.id.clone(),
                card.scene_id.clone(),
                normalized,
                start,
                end,
            )
        })
        .collect();
    items
        .into_iter()
        .filter(|item| {
            if item.kind != "text_phrase" || item.template_id != "IMPORTANT_PHRASE" {
                return true;
            }
            let normalized = item
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let (start, end) = item.window_us();
            let duplicate =
                named_cards
                    .iter()
                    .any(|(id, scene_id, card_text, card_start, card_end)| {
                        if id == &item.id || scene_id != &item.scene_id {
                            return false;
                        }
                        !normalized.is_empty()
                            && normalized == *card_text
                            && start < *card_end
                            && *card_start < end
                    });
            if duplicate {
                push_decision(
                    decisions,
                    STAGE_OVERLAP,
                    REASON_DUPLICATE_NAMED_PHRASE,
                    item,
                );
            }
            !duplicate
        })
        .collect()
}

/// Coalescing can place an admitted phrase inside a chain while an identical
/// phrase candidate remains a top-level item. Remove that duplicate after
/// coalescing, when the final chain segments and their absolute windows are
/// available.
pub(super) fn drop_phrases_duplicated_by_chain_segments(
    items: Vec<PlanItemV2>,
    decisions: &mut Vec<PlanDecisionV2>,
) -> Vec<PlanItemV2> {
    let chained_phrases: Vec<(String, String, i64, i64)> = items
        .iter()
        .filter(|item| matches!(item.kind.as_str(), "phrase_chain" | "mixed_chain"))
        .flat_map(|chain| {
            chain
                .params
                .as_ref()
                .and_then(|params| params.get("chain_segments"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|segment| {
                    segment.get("kind").and_then(Value::as_str) == Some("text_phrase")
                })
                .filter_map(|segment| {
                    let text = segment.get("text").and_then(Value::as_str)?;
                    let start = segment.get("start_us").and_then(Value::as_i64)?;
                    let duration = segment.get("duration_us").and_then(Value::as_i64)?;
                    let normalized = text
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase();
                    Some((
                        chain.scene_id.clone(),
                        normalized,
                        start,
                        start.saturating_add(duration),
                    ))
                })
        })
        .collect();

    let mut kept_phrases: Vec<(String, String, i64, i64)> = Vec::new();
    items
        .into_iter()
        .filter(|item| {
            if item.kind != "text_phrase" || item.template_id != "IMPORTANT_PHRASE" {
                return true;
            }
            let normalized = item
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let (start, end) = item.window_us();
            let duplicate = !normalized.is_empty()
                && (chained_phrases
                    .iter()
                    .any(|(scene_id, text, chain_start, chain_end)| {
                        scene_id == &item.scene_id
                            && text == &normalized
                            && start < *chain_end
                            && *chain_start < end
                    })
                    || kept_phrases
                        .iter()
                        .any(|(scene_id, text, phrase_start, phrase_end)| {
                            scene_id == &item.scene_id
                                && text == &normalized
                                && start < *phrase_end
                                && *phrase_start < end
                        }));
            if duplicate {
                push_decision(
                    decisions,
                    STAGE_OVERLAP,
                    REASON_DUPLICATE_NAMED_PHRASE,
                    item,
                );
            } else {
                kept_phrases.push((item.scene_id.clone(), normalized, start, end));
            }
            !duplicate
        })
        .collect()
}

/// True when the proposed window intersects any other entity card or text
/// card in the same scene. Images are deliberately excluded: a photo is a
/// background layer and may legally sit under a phrase.
fn overlaps_text_window(
    items: &[PlanItemV2],
    dropped: &BTreeSet<String>,
    skip_index: usize,
    scene_id: &str,
    start_us: i64,
    end_us: i64,
) -> bool {
    items.iter().enumerate().any(|(index, item)| {
        if index == skip_index || item.scene_id != scene_id || dropped.contains(&item.id) {
            return false;
        }
        entity_text_conflict(start_us, end_us, item)
    })
}

/// QA gate: no centered text card may overlap an entity card in the sealed
/// plan. The resolver above prevents this; the check catches later drift.
pub(super) fn assert_no_entity_text_overlap(
    items: &[PlanItemV2],
) -> Result<(), EditorialPlanError> {
    let cards: Vec<&PlanItemV2> = items.iter().filter(|item| is_entity_card(item)).collect();
    for text in items.iter().filter(|item| !is_entity_card(item)) {
        if let Some(card) = cards.iter().find(|card| {
            card.scene_id == text.scene_id && {
                let (card_start, card_end) = card.window_us();
                entity_text_conflict(card_start, card_end, text)
            }
        }) {
            return Err(EditorialPlanError::invalid(
                "plan.items",
                format!(
                    "text item {:?} overlaps entity card {:?} in scene {:?}",
                    text.id, card.id, text.scene_id
                ),
            ));
        }
    }
    for phrase in items
        .iter()
        .filter(|item| item.kind == "text_phrase" && item.template_id == "IMPORTANT_PHRASE")
    {
        let normalized = phrase
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let (phrase_start, phrase_end) = phrase.window_us();
        let duplicate = items
            .iter()
            .filter(|item| matches!(item.kind.as_str(), "phrase_chain" | "mixed_chain"))
            .flat_map(|chain| {
                chain
                    .params
                    .as_ref()
                    .and_then(|params| params.get("chain_segments"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .any(|segment| {
                if segment.get("kind").and_then(Value::as_str) != Some("text_phrase") {
                    return false;
                }
                let text = segment
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
                let start = segment.get("start_us").and_then(Value::as_i64).unwrap_or(0);
                let end = start.saturating_add(
                    segment
                        .get("duration_us")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                );
                !normalized.is_empty()
                    && text == normalized
                    && phrase_start < end
                    && start < phrase_end
            });
        if duplicate {
            return Err(EditorialPlanError::invalid(
                "plan.items",
                format!(
                    "phrase {:?} duplicates an overlapping phrase_chain segment",
                    phrase.id
                ),
            ));
        }
    }
    let phrases: Vec<&PlanItemV2> = items
        .iter()
        .filter(|item| item.kind == "text_phrase" && item.template_id == "IMPORTANT_PHRASE")
        .collect();
    for (index, phrase) in phrases.iter().enumerate() {
        let normalized = phrase
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let (start, end) = phrase.window_us();
        if phrases[..index].iter().any(|previous| {
            if previous.scene_id != phrase.scene_id {
                return false;
            }
            let previous_text = previous
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let (previous_start, previous_end) = previous.window_us();
            !normalized.is_empty()
                && normalized == previous_text
                && start < previous_end
                && previous_start < end
        }) {
            return Err(EditorialPlanError::invalid(
                "plan.items",
                format!(
                    "phrase {:?} duplicates an overlapping phrase card",
                    phrase.id
                ),
            ));
        }
    }
    Ok(())
}

pub(super) fn count_marked_items(items: &[PlanItemV2], key: &str) -> i64 {
    let mut count = 0;
    for item in items {
        if item
            .params
            .as_ref()
            .and_then(|params| params.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            count += 1;
        }
        if let Some(segments) = item
            .params
            .as_ref()
            .and_then(|params| params.get("chain_segments"))
            .and_then(Value::as_array)
        {
            count += segments
                .iter()
                .filter(|segment| {
                    segment
                        .get("params")
                        .and_then(|params| params.get(key))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .count() as i64;
        }
    }
    count
}

pub(super) fn record_drops(intake: &mut IntakeV2, reason: &str, before: usize, after: usize) {
    if before > after {
        *intake
            .dropped_by_reason
            .entry(reason.to_string())
            .or_default() += (before - after) as i64;
    }
}

pub(super) fn finalize_intake(intake: &mut IntakeV2, items: &[PlanItemV2]) {
    intake.planned = items.len() as i64;
    intake.planned_by_kind.clear();
    for item in items {
        *intake.planned_by_kind.entry(item.kind.clone()).or_default() += 1;
    }
}
