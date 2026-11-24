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
    if let Some(scene) = request.scenes.iter().find(|scene| scene.id == scene_id) {
        if let Some(toggle) = scene.overlay_categories.get(category) {
            if !toggle.is_empty() && toggle != "default" {
                return toggle == "disabled";
            }
        }
    }
    request
        .canvas
        .overlay_categories
        .get(category)
        .map(|toggle| toggle == "disabled")
        .unwrap_or(false)
}

fn category_disabled(request: &EditorialPlanV2Request, item: &PlanItemV2) -> bool {
    category_toggle_disabled(request, &item.scene_id, category_for(item))
}

fn group_for(item: &PlanItemV2) -> &'static str {
    match item.kind.as_str() {
        "text_phrase" if role(item) == "scene_bullet" => "bullets",
        "text_phrase" => "phrases",
        "image" | "entity_image" | "product" | "logo" => "images",
        "map" => "maps",
        "entity_card" | "organization" | "location" | "concept" => "entities",
        _ => "phrases",
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
