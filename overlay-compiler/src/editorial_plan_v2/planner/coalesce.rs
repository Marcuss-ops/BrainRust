//! Chain coalescing: caption/entity-card fusion and the camera chain
//! segments. The fused item is one plan item, not a post-plan patch.

use std::collections::BTreeSet;

use super::*;

use crate::editorial_plan_v2::is_chain_kind;

/// Default handoff between two chain segments. It is a real gap in the plan:
/// the previous segment has ended, the next one has not started yet.
/// `animation_counts.chain_handoff_ms` overrides it (0 makes segments abut).
pub(super) const CHAIN_HANDOFF_US: i64 = 400_000;

/// A card needs this long to read its caption even when the scene is about to
/// end. A segment clipped below it is a flash, not a readable beat.
pub(super) const MIN_CHAIN_SEGMENT_US: i64 = 1_000_000;

/// Preferred hold for a non-text segment. The chain uses the scene's remaining
/// room first; this is a floor, never a reason to escape the scene window.
const PREFERRED_CHAIN_HOLD_US: i64 = 2_500_000;

/// Ports `CoalesceChainedOverlaysWithSceneEnds`: nearby, same-scene overlays
/// fuse into one sequential chain only after admission. Captioned entity cards
/// are image-backed and fuse as one image_chain; plain photos stay standalone
/// so a text card never interrupts a slideshow.
pub(super) fn coalesce_chained_overlays(
    items: Vec<PlanItemV2>,
    request: &EditorialPlanV2Request,
    scene_ends: &BTreeMap<String, i64>,
) -> Vec<PlanItemV2> {
    let Some((max_gap_us, max_segments, handoff_us)) =
        chain_config(&request.canvas.animation_counts)
    else {
        return items;
    };
    // `chain_camera` is the run-level choreography knob: 0 (default) keeps the
    // legacy per-segment fly-under handoff; 1..4 select the four certified
    // camera-pan directions. Only image chains consume it.
    let chain_camera = chain_camera_direction(&request.canvas.animation_counts);
    if items.len() < 2 {
        return items;
    }
    let has_images = items.iter().any(|item| chain_family(&item.kind) == "image");
    if has_images {
        let mut standalone_images = Vec::new();
        let mut others = Vec::new();
        for item in items {
            if chain_family(&item.kind) == "image" && !is_captioned_entity_image(&item) {
                // Plain photos are never fused into a chain container. The
                // renderer lowers a chain's image segments as a simultaneous
                // deck on the right when the segments carry no per-card
                // viewport, so generic images stay standalone and the planner
                // sequences their windows after coalescing.
                standalone_images.push(item);
            } else {
                // A captioned entity image and an entity card are the same
                // editorial object at two stages: the card still waits for its
                // verified portrait (Go promotes it after the Rust plan), the
                // image already carries one. They must share ONE coalescing
                // run, otherwise a scene with six nearby identities produced
                // two partial chains — verified images in one, pending cards in
                // the other — that Go then promoted independently.
                others.push(item);
            }
        }
        let mut out = standalone_images;
        out.extend(coalesce_chain_subset(
            others,
            max_gap_us,
            max_segments,
            handoff_us,
            scene_ends,
            0,
        ));
        out.sort_by(|left, right| {
            left.window_us()
                .0
                .cmp(&right.window_us().0)
                .then_with(|| left.id.cmp(&right.id))
        });
        return apply_chain_camera(out, chain_camera);
    }
    apply_chain_camera(
        coalesce_chain_subset(items, max_gap_us, max_segments, handoff_us, scene_ends, 0),
        chain_camera,
    )
}

/// Reserves every fused chain window against the rest of the plan.
///
/// Coalescing runs after the map resolver and re-lays its segments
/// sequentially (preferred hold + handoff), so the container can grow over an
/// overlay that was placed on its own beat. The renderer then composites the
/// later item on top and the chain, including its captions, disappears from the
/// final video. The chain is the editorial owner of its window: any non-chain,
/// non-background item that intersects it is dropped, exactly as the map
/// resolver drops an item it cannot shift past a map. Two chains never share a
/// window either; the later one is dropped instead of letting both render.
///
/// Background layers are the canvas, not competing overlays, so they survive.
pub(super) fn resolve_chain_overlaps(items: Vec<PlanItemV2>) -> Vec<PlanItemV2> {
    let mut chains: Vec<(usize, i64, i64)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let (start, end) = item.window_us();
            (is_chain_kind(&item.kind) && end > start).then_some((index, start, end))
        })
        .collect();
    if chains.is_empty() {
        return items;
    }
    chains.sort_by_key(|(_, start, end)| (*start, *end));
    let mut kept_windows: Vec<(i64, i64)> = Vec::with_capacity(chains.len());
    let mut dropped: BTreeSet<usize> = BTreeSet::new();
    for (index, start, end) in chains {
        if kept_windows
            .iter()
            .any(|(kept_start, kept_end)| windows_overlap(start, end, *kept_start, *kept_end))
        {
            dropped.insert(index);
            continue;
        }
        kept_windows.push((start, end));
    }
    items
        .into_iter()
        .enumerate()
        .filter(|(index, item)| {
            if dropped.contains(index) {
                return false;
            }
            if is_chain_kind(&item.kind) || is_background_template(&item.template_id) {
                return true;
            }
            let (start, end) = item.window_us();
            !kept_windows.iter().any(|(chain_start, chain_end)| {
                windows_overlap(start, end, *chain_start, *chain_end)
            })
        })
        .map(|(_, item)| item)
        .collect()
}

fn windows_overlap(left_start: i64, left_end: i64, right_start: i64, right_end: i64) -> bool {
    left_start < right_end && right_start < left_end
}

fn is_background_template(template_id: &str) -> bool {
    matches!(
        template_id.trim().to_ascii_lowercase().as_str(),
        "background" | "video_background"
    )
}

/// Maps the `chain_camera` run knob to the certified camera-pan motion ids.
/// The values are a small, closed enum so an out-of-range value falls back to
/// the legacy handoff instead of producing an uncertified motion id.
fn chain_camera_direction(counts: &BTreeMap<String, i64>) -> Option<&'static str> {
    match counts.get("chain_camera").copied().unwrap_or(0) {
        1 => Some("image_group_camera_pan_right"),
        2 => Some("image_group_camera_pan_left"),
        3 => Some("image_group_camera_pan_down"),
        4 => Some("image_group_camera_pan_up"),
        _ => None,
    }
}

/// Stamps the chosen camera on every image chain. Text and mixed chains keep
/// their `fly_under_fade` handoff; the camera-pan group is an image-only
/// choreography.
fn apply_chain_camera(mut items: Vec<PlanItemV2>, camera: Option<&str>) -> Vec<PlanItemV2> {
    let Some(camera) = camera else {
        return items;
    };
    for item in &mut items {
        if item.kind != "image_chain" {
            continue;
        }
        if let Some(params) = item.params.as_mut().and_then(Value::as_object_mut) {
            params.insert("chain_camera".to_string(), json!(camera));
        }
    }
    items
}

/// Hard ceiling for one automatic chain. Ten overlays in a row is the
/// editorial limit: beyond it the sequence reads as a slideshow and no
/// group can fit a scene without pushing later segments past its end.
/// `chain_max_segments` lowers the bound, never raises it, and `1` (a value
/// that used to disable chaining silently) is floored to the two segments a
/// chain needs. `chain_disabled` is the only way to opt out.
pub(super) const MAX_CHAIN_SEGMENTS: usize = 10;
const MIN_CHAIN_SEGMENTS: usize = 2;

fn chain_config(counts: &BTreeMap<String, i64>) -> Option<(i64, usize, i64)> {
    if counts.get("chain_disabled").copied().unwrap_or(0) > 0 {
        return None;
    }
    let max_gap_us = if let Some(value) = counts.get("chain_gap_ms").copied().filter(|v| *v > 0) {
        // Go bounds the value before multiplying so a malformed oversized
        // configuration cannot overflow int64; saturate here for the same
        // fail-closed result instead of panicking or wrapping.
        value.saturating_mul(1000).min(5_000_000)
    } else if counts.get("chain_enabled").copied().unwrap_or(0) > 0 {
        5_000_000
    } else {
        return None;
    };
    let configured = counts
        .get("chain_max_segments")
        .copied()
        .filter(|value| *value > 0)
        .map_or(MAX_CHAIN_SEGMENTS, |value| value as usize);
    let max_segments = configured.clamp(MIN_CHAIN_SEGMENTS, MAX_CHAIN_SEGMENTS);
    // The handoff is the visible time between two segments. Zero is a valid
    // value (segments abut); negative input falls back to the legacy 400ms.
    let handoff_us = counts
        .get("chain_handoff_ms")
        .copied()
        .filter(|value| *value >= 0)
        .map_or(CHAIN_HANDOFF_US, |value| {
            value.saturating_mul(1000).min(2_000_000)
        });
    Some((max_gap_us, max_segments, handoff_us))
}

fn coalesce_chain_subset(
    items: Vec<PlanItemV2>,
    max_gap_us: i64,
    max_segments: usize,
    handoff_us: i64,
    scene_ends: &BTreeMap<String, i64>,
    chain_index_start: u64,
) -> Vec<PlanItemV2> {
    let mut sorted = items;
    sorted.sort_by(|left, right| {
        left.window_us()
            .0
            .cmp(&right.window_us().0)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut out = Vec::with_capacity(sorted.len());
    let mut index = 0;
    let mut chain_index = chain_index_start;
    while index < sorted.len() {
        if !chainable(&sorted[index]) {
            out.push(sorted[index].clone());
            index += 1;
            continue;
        }
        let scene_id = sorted[index].scene_id.clone();
        let mut group = vec![sorted[index].clone()];
        let mut group_end = sorted[index].window_us().1;
        let mut next = index + 1;
        while next < sorted.len() && (max_segments == 0 || group.len() < max_segments) {
            let candidate = &sorted[next];
            if !chainable(candidate) || candidate.scene_id != scene_id {
                break;
            }
            let gap = candidate.window_us().0.saturating_sub(group_end);
            let mut gap_limit = max_gap_us;
            if max_gap_us >= 5_000_000
                && is_image_backed_entity_card(group.last().expect("group"))
                && is_image_backed_entity_card(candidate)
            {
                gap_limit = gap_limit.max(7_500_000);
            }
            if gap > gap_limit {
                break;
            }
            group.push(candidate.clone());
            group_end = group_end.max(candidate.window_us().1);
            next += 1;
        }
        // A run that would spill past the scene end is not dropped: the
        // segments that fit are fused and the remainder stays standalone. The
        // old all-or-nothing guard discarded the whole chain when the last
        // segment overflowed by a few milliseconds, which is exactly how a
        // scene with three nearby entity images ended up with no chain.
        let fitted = fit_group_to_scene_end(&group, scene_ends.get(&scene_id).copied(), handoff_us);
        if fitted < 2 {
            out.extend(group);
            index = next;
            continue;
        }
        out.push(fuse_chain(&group[..fitted], chain_index, handoff_us));
        out.extend(group[fitted..].iter().cloned());
        chain_index += 1;
        index = next;
    }
    out
}

/// Only text, image and card items enter automatic coalescing. Maps are
/// deliberately absent: a map owns the full georeferenced canvas, pin timing
/// and LOD assets, so it must keep its standalone window. The wire contract
/// still accepts an explicitly authored `map_chain`; this compiler never
/// produces one.
fn chainable(item: &PlanItemV2) -> bool {
    if item.params.as_ref().is_some_and(|params| {
        params.get("chart_card") == Some(&Value::Bool(true))
            || params.get("chart_bar") == Some(&Value::Bool(true))
    }) {
        return false;
    }
    if item.template_id.trim() == "TIMELINE_DATE_CARD" {
        return false;
    }
    if item
        .image_layers
        .as_ref()
        .is_some_and(|layers| layers.as_array().is_some_and(|layers| !layers.is_empty()))
        || !item.group_motion_id.trim().is_empty()
    {
        return false;
    }
    let family = chain_family(&item.kind);
    if family.is_empty() {
        return false;
    }
    if family == "image" && item.asset_refs.is_empty() {
        return false;
    }
    if item.sound_effect.is_some() || item.window_us().1 <= item.window_us().0 {
        return false;
    }
    if family == "text" && item.motion_id.starts_with("short_phrase_") {
        return false;
    }
    true
}

/// Automatic chain families. `map` is intentionally not a family: maps stay
/// standalone and `map_chain` is accepted only as an explicitly authored wire
/// item, never emitted by the Rust compiler.
/// A captioned entity card is the one image family that chains automatically:
/// its caption is anchored below its own portrait and its viewport is pinned
/// by `assign_entity_media`, so a sequence cannot collapse into a deck.
fn is_captioned_entity_image(item: &PlanItemV2) -> bool {
    item.kind.trim() == "entity_image"
        && !item.entity_caption.trim().is_empty()
        && !item.asset_refs.is_empty()
}

pub(super) fn chain_family(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "text_phrase" | "important_word" | "keyword" | "quote" | "brand_text" | "brand-text"
        | "number" => "text",
        "image" | "entity_image" | "product" | "logo" => "image",
        "entity_card" | "organization" | "location" | "concept" | "lower_third" => "card",
        _ => "",
    }
}

/// Mirrors `projectedChainEndUS`: the first segment keeps the group start,
/// later segments follow after the handoff gap, and card segments get the same
/// 2.5s minimum hold as the fused output.
#[cfg(test)]
pub(super) fn projected_chain_end_us(group: &[PlanItemV2], handoff_us: i64) -> i64 {
    let Some(first) = group.first() else {
        return 0;
    };
    let start = group
        .iter()
        .map(|item| item.window_us().0)
        .min()
        .unwrap_or_else(|| first.window_us().0);
    let mut cursor = start;
    for (index, item) in group.iter().enumerate() {
        let (item_start, item_end) = item.window_us();
        let mut duration = item_end.saturating_sub(item_start);
        if chain_family(&item.kind) != "text" && duration < PREFERRED_CHAIN_HOLD_US {
            duration = PREFERRED_CHAIN_HOLD_US;
        }
        if index > 0 {
            cursor = cursor.saturating_add(handoff_us);
        }
        cursor = cursor.saturating_add(duration);
    }
    cursor
}

/// Number of leading segments of `group` whose projected chain window starts
/// before the scene end. The chain keeps the first segment's start and the
/// configured handoff, so a later item's own start is not a bound. The last included
/// segment may overrun the end: `clip_items_to_scene_bounds` shortens it (item
/// and segment) to the scene window afterwards.
fn fit_group_to_scene_end(group: &[PlanItemV2], scene_end: Option<i64>, handoff_us: i64) -> usize {
    let Some(end) = scene_end.filter(|end| *end > 0) else {
        return group.len();
    };
    let mut cursor = group
        .first()
        .map(|item| {
            group
                .iter()
                .map(|item| item.window_us().0)
                .min()
                .unwrap_or_else(|| item.window_us().0)
        })
        .unwrap_or(0);
    let mut fitted = 0;
    for (index, item) in group.iter().enumerate() {
        let (item_start, item_end) = item.window_us();
        let mut duration = item_end.saturating_sub(item_start);
        if chain_family(&item.kind) != "text" && duration < PREFERRED_CHAIN_HOLD_US {
            duration = PREFERRED_CHAIN_HOLD_US;
        }
        let segment_start = if index == 0 {
            item_start
        } else {
            cursor.saturating_add(handoff_us)
        };
        // Include a segment only when its own readable window fits before the
        // scene end. A later clamp cannot invent the missing time: it used to
        // leave a 300-900ms caption flash at the tail of every tight scene.
        if segment_start.saturating_add(MIN_CHAIN_SEGMENT_US) > end {
            break;
        }
        fitted = index + 1;
        cursor = segment_start.saturating_add(duration);
    }
    fitted
}

/// One segment of a chain container: the typed producer side of the
/// `chain_segments` wire shared with PipelineGen's `capmodel.ChainSegment` and
/// RenderingGen's `chainSegment` mirror. The serialized key set IS the
/// contract — every key below appears in the canonical list pinned by
/// `chain_segment_wire_keys_are_the_documented_contract`.
///
/// `is_blank` reproduces the former `insert_non_empty` helper exactly (a
/// whitespace-only value was never emitted), and every optional payload keeps
/// `skip_serializing_if`, so a plan whose chain carries the same content still
/// serializes to the same JSON as before this struct existed.
#[derive(Debug, Clone, serde::Serialize)]
struct ChainSegment {
    #[serde(skip_serializing_if = "is_blank")]
    text: String,
    #[serde(skip_serializing_if = "is_blank")]
    group_id: String,
    #[serde(skip_serializing_if = "is_blank")]
    subgroup_id: String,
    #[serde(skip_serializing_if = "is_blank")]
    motion_id: String,
    #[serde(skip_serializing_if = "is_blank")]
    preset_id: String,
    #[serde(skip_serializing_if = "is_blank")]
    image_preset_id: String,
    #[serde(skip_serializing_if = "is_blank")]
    template_id: String,
    #[serde(skip_serializing_if = "is_blank")]
    kind: String,
    #[serde(skip_serializing_if = "is_blank")]
    entity_id: String,
    #[serde(skip_serializing_if = "is_blank")]
    entity_caption: String,
    #[serde(skip_serializing_if = "is_blank")]
    caption_motion_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption_motion_params: Option<Value>,
    #[serde(skip_serializing_if = "is_blank")]
    caption_font_family: String,
    #[serde(skip_serializing_if = "is_blank")]
    caption_layout: String,
    #[serde(skip_serializing_if = "is_blank")]
    caption_color: String,
    #[serde(skip_serializing_if = "is_blank")]
    entity_style_id: String,
    start_us: i64,
    duration_us: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    motion_params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    assets: Vec<AssetRefV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_layers: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    map: Option<Value>,
}

fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}

/// DEFAULT_CHAIN_IMAGE_MOTION is the entrance a chained image segment gets when
/// the producer item carries none. Without it the segment is a still frame that
/// replaces the previous one: the chain hand-off looks like a cut and there is
/// nothing for the renderer to animate. The id is certified in the Chronon
/// motion catalog for image templates (same family the entity pool rotates).
const DEFAULT_CHAIN_IMAGE_MOTION: &str = "image_focus_reveal";

/// Builds one chain container exactly like the former Go `model.fuseChain`:
/// segments are laid
/// out sequentially with the configured handoff, card segments keep a 2.5s minimum
/// hold, and every segment keeps its own text, motion, caption, assets and
/// opaque metadata. The container carries no preset/motion of its own.
fn fuse_chain(group: &[PlanItemV2], chain_index: u64, handoff_us: i64) -> PlanItemV2 {
    let first = &group[0];
    let start_us = group
        .iter()
        .map(|item| item.window_us().0)
        .min()
        .unwrap_or(0);
    let mut families = BTreeSet::new();
    for item in group {
        families.insert(chain_family(&item.kind));
    }
    let family = if families.len() == 1 {
        families.iter().next().copied().unwrap_or("mixed")
    } else {
        "mixed"
    };
    let (kind, template) = match family {
        "text" => ("phrase_chain", "PHRASE_CHAIN"),
        "image" => ("image_chain", "IMAGE_CHAIN"),
        _ => ("mixed_chain", "MIXED_CHAIN"),
    };
    let mut cursor_us = start_us;
    let mut segments: Vec<Value> = Vec::with_capacity(group.len());
    let mut motions: Vec<Value> = Vec::with_capacity(group.len());
    let mut assets: Vec<AssetRefV2> = Vec::new();
    let mut seen_assets: BTreeSet<String> = BTreeSet::new();
    let mut recap_count = 0_i64;
    for (index, item) in group.iter().enumerate() {
        let (item_start, item_end) = item.window_us();
        let mut duration_us = item_end.saturating_sub(item_start);
        if chain_family(&item.kind) != "text" && duration_us < PREFERRED_CHAIN_HOLD_US {
            duration_us = PREFERRED_CHAIN_HOLD_US;
        }
        let segment_start_us = if index == 0 {
            item_start
        } else {
            cursor_us.saturating_add(handoff_us)
        };
        // Every image segment must animate on entry: a segment with no motion
        // is a still image swapped in place, which is exactly what makes a
        // chain read as "one image behind the other" instead of a sequence.
        let motion_id = if item.motion_id.trim().is_empty() && chain_family(&item.kind) == "image" {
            DEFAULT_CHAIN_IMAGE_MOTION.to_string()
        } else {
            item.motion_id.clone()
        };
        let segment = ChainSegment {
            text: item.text.clone(),
            group_id: item.group_id.clone(),
            subgroup_id: item.subgroup_id.clone(),
            motion_id: motion_id.clone(),
            preset_id: item.preset_id.clone(),
            image_preset_id: item.image_preset_id.clone(),
            template_id: item.template_id.clone(),
            kind: item.kind.clone(),
            entity_id: item.entity_id.clone(),
            entity_caption: item.entity_caption.clone(),
            caption_motion_id: item.caption_motion_id.clone(),
            caption_motion_params: item.caption_motion_params.clone(),
            caption_font_family: item.caption_font_family.clone(),
            caption_layout: item.caption_layout.clone(),
            caption_color: item.caption_color.clone(),
            entity_style_id: item.entity_style_id.clone(),
            start_us: segment_start_us,
            duration_us,
            motion_params: item.motion_params.clone(),
            asset_id: item.asset_refs.first().map(|asset| asset.asset_id.clone()),
            assets: item.asset_refs.clone(),
            image_layers: item.image_layers.clone(),
            frame: item.frame.clone(),
            params: item.params.clone(),
            style: item.style.clone(),
            map: item.map.clone(),
        };
        if item
            .params
            .as_ref()
            .and_then(|params| params.get("recap"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            recap_count += 1;
        }
        segments.push(serde_json::to_value(&segment).expect("chain segment serializes"));
        motions.push(json!(motion_id));
        cursor_us = segment_start_us.saturating_add(duration_us);
        for asset in &item.asset_refs {
            if asset.asset_id.trim().is_empty() || !seen_assets.insert(asset.asset_id.clone()) {
                continue;
            }
            assets.push(asset.clone());
        }
    }
    let mut params = serde_json::Map::new();
    if recap_count > 0 {
        params.insert("recap_count".to_string(), json!(recap_count));
    }
    params.insert("chain_segments".to_string(), Value::Array(segments));
    params.insert("chain_motions".to_string(), Value::Array(motions));
    params.insert("chain_count".to_string(), json!(group.len()));
    params.insert("chain_camera".to_string(), json!("fly_under_fade"));
    params.insert("chain_gap_us".to_string(), json!(handoff_us));
    let legacy_text = if first.text.trim().is_empty() {
        group
            .iter()
            .map(|item| item.text.as_str())
            .find(|text| !text.trim().is_empty())
            .unwrap_or("")
            .to_string()
    } else {
        first.text.clone()
    };
    PlanItemV2 {
        id: if first.scene_id.trim().is_empty() {
            format!("chain-{family}:{chain_index}")
        } else {
            format!("{}:chain-{family}:{chain_index}", first.scene_id.trim())
        },
        scene_id: first.scene_id.clone(),
        kind: kind.to_string(),
        template_id: template.to_string(),
        text: legacy_text,
        start_ms: start_us / 1000,
        end_ms: (cursor_us + 999) / 1000,
        start_us,
        duration_us: cursor_us.saturating_sub(start_us),
        asset_refs: assets,
        params: Some(Value::Object(params)),
        ..PlanItemV2::default()
    }
}

#[cfg(test)]
mod segment_contract_tests;
