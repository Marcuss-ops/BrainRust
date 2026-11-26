//! The v2 single-call planner. It consumes the complete request and returns a
//! sealed `FinalPlanV2`: scene candidates, editorial title/bullet selection,
//! verified entity cards, admission, entrance cascade and seal. The module
//! deliberately reuses the existing Rust ranking/cascade primitives.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::numeric::parse_chart_value;
use crate::{
    cascade_clustered_entrances, rank_entity_overlays, CascadeEntranceItem,
    CascadeEntrancesRequest, EntityRankingInput, EntityRankingScene, RankEntityOverlaysRequest,
    TimingItem,
};

use super::editorial::project_editorial_scenes;
use super::error::EditorialPlanError;
use super::request::{
    AssetRefV2, BudgetsV2, EditorialPlanV2Request, EntityOccurrenceV2, ImageCandidateV2,
    MapCandidateV2, MediaV2, SceneV2, SealV2, TimedAnnotationV2,
};
use super::response::{
    EditorialPlanV2Compiled, EntityRefV2, FinalPlanV2, IntakeBudgetV2, IntakeV2, PlanDecisionV2,
    PlanItemV2,
};
use super::EDITORIAL_PLAN_V2_SCHEMA_VERSION;

mod admission;
mod candidates;
mod coalesce;
mod decisions;
mod map_groups;
mod media;
mod presentation;
mod visual_recipes;

use admission::*;
use candidates::*;
use coalesce::*;
use decisions::*;
use media::*;
use presentation::*;

/// A request that passed [`EditorialPlanV2Request::validate_compilable`].
///
/// The planner phases take this type instead of the raw request, so a phase
/// never re-asserts that `budgets` and `seal` are present: the presence check
/// happens once, at the boundary, and [`Self::budgets`]/[`Self::seal`] hand
/// out references rather than `Option`s. [`Deref`] keeps every other field
/// reachable without threading a second argument through each phase.
#[derive(Debug)]
pub(super) struct ValidatedRequest<'a> {
    request: &'a EditorialPlanV2Request,
    budgets: &'a BudgetsV2,
    seal: &'a SealV2,
}

impl<'a> ValidatedRequest<'a> {
    pub(super) fn new(request: &'a EditorialPlanV2Request) -> Result<Self, EditorialPlanError> {
        request.validate_compilable()?;
        let budgets = request
            .budgets
            .as_ref()
            .ok_or(EditorialPlanError::MissingSection("budgets"))?;
        let seal = request
            .seal
            .as_ref()
            .ok_or(EditorialPlanError::MissingSection("seal"))?;
        Ok(Self {
            request,
            budgets,
            seal,
        })
    }

    /// The run budgets validation already required.
    pub(super) fn budgets(&self) -> &'a BudgetsV2 {
        self.budgets
    }

    /// The seal extent validation already required.
    pub(super) fn seal(&self) -> &'a SealV2 {
        self.seal
    }
}

impl std::ops::Deref for ValidatedRequest<'_> {
    type Target = EditorialPlanV2Request;

    fn deref(&self) -> &Self::Target {
        self.request
    }
}

/// Validates the complete request, runs every phase in order and seals the
/// final plan. There is no empty-plan fallback: a missing input is an error.
pub fn compile_plan(
    request: &EditorialPlanV2Request,
) -> Result<EditorialPlanV2Compiled, EditorialPlanError> {
    // WP4: raw surfaces are projected here, before any planner phase. The
    // producer cannot pre-select, drop or time a phrase/entity/number.
    let mut projected_request = request.clone();
    super::projection::project_request(&mut projected_request);
    let request = &projected_request;
    let validated = ValidatedRequest::new(request)?;
    let mut editorial = project_editorial_scenes(request)?;
    if request.chapters.is_some() {
        // Chapter bullets own the bullet slot on the scenes they cover. They
        // are projected before the editorial bullets are admitted, exactly as
        // the Go planner orders the two phases; a scene keeps at most one
        // bullet, so the editorial bullet is dropped on covered scenes.
        let chapters = crate::project_chapter_bullets(request);
        for (scene_id, phrases) in &chapters.scene_phrases {
            editorial
                .scene_phrases
                .entry(scene_id.clone())
                .or_default()
                .extend(phrases.iter().cloned());
        }
        let covered: BTreeSet<String> = chapters.covered_scenes.iter().cloned().collect();
        for (scene_id, phrases) in editorial.scene_phrases.iter_mut() {
            if covered.contains(scene_id) {
                phrases.retain(|phrase| phrase.presentation_role != "scene_bullet");
            }
        }
        editorial.covered_scenes = chapters.covered_scenes;
    }
    let mut decisions = Vec::new();
    let mut candidates = build_base_candidates(request, &editorial)?;
    trim_maps(request, &mut candidates);
    let recap = build_recap_candidates(request)?;
    candidates.extend(recap.items);
    // Every lane — editorial, annotations, recap and the entity cards — is a
    // candidate before admission. There is exactly one admission pass, so a
    // card cannot bypass the budget and no second pass can re-admit a scene
    // the first one already trimmed. `admit` ranks the candidates (entity
    // image/map > still > text > number), so append order no longer decides
    // which lane loses a full ceiling.
    candidates.extend(build_entity_card_candidates(request, &mut decisions)?);
    let intake = new_intake(&validated, &candidates);
    let (mut items, mut intake) = admit(&validated, candidates, intake, &mut decisions);
    // A numeric card and a plain phrase can describe the same grounded surface
    // at the same instant. The numeric card owns it; remove the duplicate only
    // after admission, so a card that lost its slot does not remove the phrase
    // that should have survived in its place.
    let before_numeric_dedup = items.len();
    items = drop_text_phrases_shadowed_by_numeric_cards(items);
    record_drops(
        &mut intake,
        "numeric_duplicate",
        before_numeric_dedup,
        items.len(),
    );
    let before_named_dedup = items.len();
    items = drop_phrases_duplicated_by_named_cards(items, &mut decisions);
    record_drops(
        &mut intake,
        "duplicate_named_phrase",
        before_named_dedup,
        items.len(),
    );
    // An entity card and a plain phrase share the center stage. Go re-anchors
    // phrases to the certified word timing after the seal, so the phrase cannot
    // be moved here; the entity card is shifted to the nearest free window and
    // the phrase is dropped with a typed reason only when no window exists.
    let before_occlusion = items.len();
    items = resolve_entity_text_overlaps(request, items, &mut decisions);
    record_drops(
        &mut intake,
        "occluded_by_entity_card",
        before_occlusion,
        items.len(),
    );
    assign_entity_media(request, &mut items)?;
    // The short-phrase entrance is decided before coalescing: a phrase that
    // already carries a lane motion is chainable on its own terms, and the
    // renderer must never receive an IMPORTANT_PHRASE with an empty motion_id.
    assign_short_phrase_motions(&mut items, request);
    // The long lane rotates one pool instead of per-length buckets, but the
    // rule is the same: the motion is decided before coalescing so the chain
    // segments that reach the renderer each carry their own entrance.
    assign_long_phrase_motions(&mut items, request);
    // Date and metric cards rotate the same three certified families and use
    // the same plan-wide no-repeat rule. Without this pass each card was an
    // independent hash: two dates could show the same gesture and a short run
    // could never reach the typewriter family.
    assign_numeric_motions(&mut items, request);
    cascade(&mut items, request);
    // Overlap resolution is the last layout phase: it must see the final
    // admitted and cascaded set, never the pre-budget candidates. A map can
    // shorten an earlier map or shift/drop a colliding card, so items and the
    // intake accounting are re-sealed together afterwards.
    // Grouping is an editorial decision that owns the map windows, so it runs
    // before overlap resolution: the surviving map already carries its pins and
    // their spoken schedule, and the resolver trims it against that schedule
    // instead of re-deriving one downstream.
    items = map_groups::group_map_items(items, request);
    let before_overlap = items.len();
    items = resolve_overlaps(request, items, &mut decisions);
    record_drops(&mut intake, "overlap", before_overlap, items.len());
    let scene_ends = request
        .scenes
        .iter()
        .map(|scene| {
            (
                scene.id.clone(),
                scene.timeline_start_us.saturating_add(scene.duration_us),
            )
        })
        .collect::<BTreeMap<String, i64>>();
    items = coalesce_chained_overlays(items, request, &scene_ends);
    // Plain photos remain independent render items: the renderer used to draw
    // a chain's generic image segments as a simultaneous deck of cards
    // (verified on the r13 cut, three city photos stacked at 31 s). Captioned
    // entity cards are the exception: they carry an authored centered viewport
    // and a below-portrait caption before coalescing, so they fuse into one
    // image_chain. `normalize_image_presentation` only touches standalone
    // images, so the final chain segments keep their pinned geometry.
    normalize_image_presentation(request, &mut items);
    let image_windows = request
        .scenes
        .iter()
        .map(|scene| {
            (
                scene.id.clone(),
                [
                    scene.timeline_start_us,
                    scene.timeline_start_us.saturating_add(scene.duration_us),
                ],
            )
        })
        .collect::<BTreeMap<String, [i64; 2]>>();
    items = sequence_scene_image_windows(&image_windows, items);
    let before_leaks = items.len();
    items = append_light_leak_items(request, items);
    // Leaks are derived render items, not budget candidates, but the sealed
    // intake contract still requires every planned item to be accounted for.
    let added_leaks = items.len().saturating_sub(before_leaks) as i64;
    if added_leaks > 0 {
        intake.candidates += added_leaks;
        *intake
            .candidates_by_kind
            .entry("light_leak".to_string())
            .or_default() += added_leaks;
    }
    let before_presentation = items.len();
    items = apply_presentation(
        items,
        &request.plan.plan_id,
        &request.canvas.editorial_bullet_styles,
    );
    apply_entity_badge_color(&mut items, &request.canvas.entity_badge_color);
    // The long-copy split turns one admitted phrase into multiple render
    // items. They are derived items, not new candidates, but the sealed
    // intake contract still requires every planned item to be accounted for.
    let added_by_presentation = items.len().saturating_sub(before_presentation) as i64;
    if added_by_presentation > 0 {
        intake.candidates += added_by_presentation;
        *intake
            .candidates_by_kind
            .entry("text_phrase".to_string())
            .or_default() += added_by_presentation;
    }
    // Coalescing re-lays a chain's segments after the map resolver has run, so
    // its window can cover an independently placed overlay. Reserve it here,
    // before the scene clamp, so the renderer can never hide the chain behind
    // a later image (or another chain) in the final composite.
    let before_chain_overlap = items.len();
    items = resolve_chain_overlaps(items);
    record_drops(
        &mut intake,
        "chain_overlap",
        before_chain_overlap,
        items.len(),
    );
    // A coalesced chain may absorb one copy of a phrase while a duplicate
    // candidate remains outside it. Resolve against the sealed segment windows.
    let before_chain_phrase_dedup = items.len();
    items = drop_phrases_duplicated_by_chain_segments(items, &mut decisions);
    record_drops(
        &mut intake,
        "duplicate_chain_phrase",
        before_chain_phrase_dedup,
        items.len(),
    );
    // Coalescing and the presentation pass can both extend a container past
    // its scene window (a fused chain lays segments out sequentially). The
    // seal is the last owner of the scene invariant, so clamp here instead of
    // letting `validate_for` reject the whole plan after the fact.
    items = clip_items_to_scene_bounds(items, request, &mut decisions);
    // QA gate: no entity card may overlap another centered text card. The
    // resolver above prevents this; fail closed if a later phase reintroduces
    // the condition (ITCLOSE-09).
    assert_no_entity_text_overlap(&items)?;
    name_items_for_review(&mut items);
    // There is no post-plan promotion pass: a card that has a verified portrait
    // was built as `entity_image` by `append_entity_cards` and passed the same
    // admission, coalescing and clipping as every other item.
    finalize_intake(&mut intake, &items);
    let plan = seal(&validated, items, intake, recap.placements, decisions);
    let compiled = EditorialPlanV2Compiled {
        ok: true,
        schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
        plan,
    };
    compiled.validate_for(request)?;
    Ok(compiled)
}

fn clip_items_to_scene_bounds(
    items: Vec<PlanItemV2>,
    request: &EditorialPlanV2Request,
    decisions: &mut Vec<PlanDecisionV2>,
) -> Vec<PlanItemV2> {
    let windows: BTreeMap<&str, (i64, i64)> = request
        .scenes
        .iter()
        .map(|scene| {
            (
                scene.id.trim(),
                (
                    scene.timeline_start_us,
                    scene.timeline_start_us.saturating_add(scene.duration_us),
                ),
            )
        })
        .collect();
    let plan_end_us = windows.values().map(|(_, end)| *end).max().unwrap_or(0);
    items
        .into_iter()
        .filter_map(|mut item| {
            let Some(&(scene_start, scene_end)) = windows.get(item.scene_id.trim()) else {
                return Some(item);
            };
            if scene_end <= scene_start {
                return Some(item);
            }
            // A map carries the presentation window the resolver already
            // bounded to the plan; clipping it back to its scene would undo
            // the camera minimum (and make the tail clamp meaningless).
            let allowed_end = if item.kind.trim().eq_ignore_ascii_case("map") && plan_end_us > 0 {
                plan_end_us
            } else {
                scene_end
            };
            let (start_us, end_us) = item.window_us();
            let clipped_start = start_us.max(scene_start);
            let clipped_end = end_us.min(allowed_end);
            if clipped_end <= clipped_start {
                push_decision(decisions, STAGE_CLIP, REASON_CLIPPED_OUT, &item);
                return None;
            }
            item.start_us = clipped_start;
            item.duration_us = clipped_end - clipped_start;
            item.start_ms = clipped_start / 1_000;
            item.end_ms = (clipped_end + 999) / 1_000;
            if !clamp_chain_segments(&mut item, scene_start, scene_end) {
                push_decision(decisions, STAGE_CLIP, REASON_CLIPPED_OUT, &item);
                return None;
            }
            Some(item)
        })
        .collect()
}

/// Clamps a chain's segments to the scene and drops any segment that cannot
/// hold a readable window. A chain with fewer than two readable segments is no
/// longer a chain: it is dropped rather than rendered as a one-frame flash.
/// Returns false when the caller must drop the whole item.
fn clamp_chain_segments(item: &mut PlanItemV2, scene_start: i64, scene_end: i64) -> bool {
    let chain_item = crate::editorial_plan_v2::is_chain_kind(&item.kind);
    let (start_us, end_us, chain_count) = {
        let Some(params) = item.params.as_mut().and_then(Value::as_object_mut) else {
            return true;
        };
        let Some(original) = params
            .get("chain_segments")
            .and_then(Value::as_array)
            .cloned()
        else {
            return true;
        };
        let original_motions = params
            .get("chain_motions")
            .and_then(Value::as_array)
            .cloned();
        let mut kept = Vec::with_capacity(original.len());
        let mut kept_indices = Vec::with_capacity(original.len());
        for (index, segment) in original.iter().enumerate() {
            let Some(object) = segment.as_object() else {
                continue;
            };
            let Some(start_us) = object.get("start_us").and_then(Value::as_i64) else {
                continue;
            };
            let Some(duration_us) = object.get("duration_us").and_then(Value::as_i64) else {
                continue;
            };
            let clipped_start = start_us.max(scene_start);
            let clipped_end = start_us.saturating_add(duration_us).min(scene_end);
            let clipped_duration = clipped_end.saturating_sub(clipped_start);
            if clipped_duration <= 0 || (chain_item && clipped_duration < MIN_CHAIN_SEGMENT_US) {
                continue;
            }
            let mut object = object.clone();
            object.insert("start_us".to_string(), json!(clipped_start));
            object.insert("duration_us".to_string(), json!(clipped_duration));
            kept.push(Value::Object(object));
            kept_indices.push(index);
        }
        if chain_item && kept.len() < 2 {
            return false;
        }
        let kept_motions: Option<Vec<Value>> = original_motions.map(|motions| {
            kept_indices
                .iter()
                .filter_map(|index| motions.get(*index).cloned())
                .collect()
        });
        let start_us = kept
            .iter()
            .filter_map(|segment| segment.get("start_us").and_then(Value::as_i64))
            .min()
            .unwrap_or(item.start_us);
        let end_us = kept
            .iter()
            .filter_map(|segment| {
                let start = segment.get("start_us").and_then(Value::as_i64)?;
                let duration = segment.get("duration_us").and_then(Value::as_i64)?;
                start.checked_add(duration)
            })
            .max()
            .unwrap_or(start_us);
        let chain_count = kept.len();
        params.insert("chain_segments".to_string(), Value::Array(kept));
        if let Some(motions) = kept_motions {
            params.insert("chain_motions".to_string(), Value::Array(motions));
        }
        (start_us, end_us, chain_count)
    };
    if chain_item {
        if let Some(params) = item.params.as_mut().and_then(Value::as_object_mut) {
            params.insert("chain_count".to_string(), json!(chain_count));
        }
        item.start_us = start_us;
        item.duration_us = end_us.saturating_sub(start_us);
        item.start_ms = start_us / 1_000;
        item.end_ms = end_us.saturating_add(999) / 1_000;
    }
    true
}

fn seal(
    request: &ValidatedRequest<'_>,
    mut items: Vec<PlanItemV2>,
    intake: IntakeV2,
    recap_placements: i64,
    decisions: Vec<PlanDecisionV2>,
) -> FinalPlanV2 {
    // Render keys are derived state, never planner input. The seal clears
    // them; Go recomputes a SHA-256 key from the finished document.
    for item in &mut items {
        item.render_key.clear();
    }
    let seal = request.seal();
    let scene_end_ms = request
        .scenes
        .iter()
        .map(|scene| (scene.timeline_start_us + scene.duration_us) / 1000)
        .max()
        .unwrap_or(0);
    let duration_ms = (seal.audio_duration_us / 1000).max(scene_end_ms);
    let recap_cards = count_marked_items(&items, "recap");
    let chart_cards = count_marked_items(&items, "chart");
    let chart_placements = count_chart_placements(request);
    FinalPlanV2 {
        schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
        plan_id: request.plan.plan_id.clone(),
        video_id: request.plan.video_id.clone(),
        project_id: request.plan.project_id.clone(),
        language: request.language.clone(),
        width: request.plan.width,
        height: request.plan.height,
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        duration_ms,
        background: seal.background.clone(),
        items,
        recap_placements,
        recap_cards,
        chart_placements,
        chart_cards,
        timed_scenes: request
            .scenes
            .iter()
            .filter(|scene| scene.duration_us > 0)
            .count() as i64,
        intake,
        decisions,
        output_profile_id: seal.output_profile_id.clone(),
    }
}

/// Counts the grounded numeric comparisons the chart lane can render, with the
/// same rules as `ChartPlacementsForScenes`: 2..6 strict values in one scene
/// and a window of at least two seconds. The items themselves are generated
/// by the chart lane; this is the placement count the sealed plan reports.
fn count_chart_placements(request: &EditorialPlanV2Request) -> i64 {
    let mut placements = 0_i64;
    for scene in &request.scenes {
        let Some(annotations) = scene.annotations.as_ref() else {
            continue;
        };
        let numbers = if annotations.chart_numbers.is_empty() {
            &annotations.numbers
        } else {
            &annotations.chart_numbers
        };
        let mut windows: Vec<(i64, i64)> = Vec::new();
        for number in numbers {
            if number.text.trim().is_empty() || number.start_us < 0 || number.duration_us <= 0 {
                continue;
            }
            if parse_chart_value(&number.text).is_none() {
                continue;
            }
            windows.push((
                number.start_us,
                number.start_us.saturating_add(number.duration_us),
            ));
            if windows.len() >= 6 {
                break;
            }
        }
        if windows.len() < 2 {
            continue;
        }
        let start = windows.iter().map(|window| window.0).min().unwrap_or(0);
        let end = windows.iter().map(|window| window.1).max().unwrap_or(0);
        if end > start && end - start >= 2_000_000 {
            placements += 1;
        }
    }
    placements
}

#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "visual_recipes_tests.rs"]
mod visual_recipes_tests;
