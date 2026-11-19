//! Chain coalescing: caption/entity-card fusion and the camera chain
//! segments. The fused item is one plan item, not a post-plan patch.

use super::*;

/// Ports `CoalesceChainedOverlaysWithSceneEnds`: nearby, same-scene overlays
/// fuse into one sequential chain only after admission. Captioned entity cards
/// are image-backed and fuse as one image_chain; plain photos stay standalone
/// so a text card never interrupts a slideshow.
pub(super) fn coalesce_chained_overlays(
    items: Vec<PlanItemV2>,
    request: &EditorialPlanV2Request,
    scene_ends: &BTreeMap<String, i64>,
) -> Vec<PlanItemV2> {
    // Go attaches the resolved portrait before consulting the chain config, so
    // a chain-disabled plan still gives a text card its certified image.
    let items = attach_resolved_entity_images(items);
    let Some((max_gap_us, max_segments)) = chain_config(&request.canvas.animation_counts) else {
        return items;
    };
    if items.len() < 2 {
        return items;
    }
    let has_images = items
        .iter()
        .any(|item| chain_family(&item.kind) == "image");
    if has_images {
        let mut standalone_images = Vec::new();
        let mut chained_images = Vec::new();
        let mut others = Vec::new();
        for item in items {
            if chain_family(&item.kind) != "image" {
                others.push(item);
            } else if is_captioned_entity_image(&item) {
                // A captioned entity card is a portrait plus a name, and the
                // name must stay under the portrait for every card in the
                // sequence. These cards own a stable, centered viewport and an
                // entrance motion by the time coalescing runs, so they can be
                // fused into one image_chain instead of being forced to take
                // turns as independent items.
                chained_images.push(item);
            } else {
                // Plain photos are never fused into a chain container. The
                // renderer lowers a chain's image segments as a simultaneous
                // deck on the right when the segments carry no per-card
                // viewport, so generic images stay standalone and the planner
                // sequences their windows after coalescing.
                standalone_images.push(item);
            }
        }
        let mut out = standalone_images;
        out.extend(coalesce_chain_subset(
            chained_images,
            max_gap_us,
            max_segments,
            scene_ends,
            0,
        ));
        out.extend(coalesce_chain_subset(
            others,
            max_gap_us,
            max_segments,
            scene_ends,
            0,
        ));
        out.sort_by(|left, right| {
            left.window_us()
                .0
                .cmp(&right.window_us().0)
                .then_with(|| left.id.cmp(&right.id))
        });
        return out;
    }
    coalesce_chain_subset(items, max_gap_us, max_segments, scene_ends, 0)
}

fn chain_config(counts: &BTreeMap<String, i64>) -> Option<(i64, usize)> {
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
    let max_segments = counts
        .get("chain_max_segments")
        .copied()
        .filter(|value| *value > 0)
        .unwrap_or(0) as usize;
    Some((max_gap_us, max_segments))
}

fn coalesce_chain_subset(
    items: Vec<PlanItemV2>,
    max_gap_us: i64,
    max_segments: usize,
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
        if group.len() < 2 {
            out.push(sorted[index].clone());
            index += 1;
            continue;
        }
        if chain_exceeds_scene_end(&group, scene_ends.get(&scene_id).copied()) {
            out.extend(group);
            index = next;
            continue;
        }
        out.push(fuse_chain(&group, chain_index));
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
pub(super) fn projected_chain_end_us(group: &[PlanItemV2]) -> i64 {
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
        if chain_family(&item.kind) != "text" && duration < 2_500_000 {
            duration = 2_500_000;
        }
        if index > 0 {
            cursor = cursor.saturating_add(400_000);
        }
        cursor = cursor.saturating_add(duration);
    }
    cursor
}

pub(super) fn chain_exceeds_scene_end(group: &[PlanItemV2], scene_end: Option<i64>) -> bool {
    matches!(scene_end, Some(end) if end > 0 && group.len() >= 2 && projected_chain_end_us(group) > end)
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
    #[serde(skip_serializing_if = "is_blank")]
    camera_motion_id: String,
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
/// out sequentially with a 400ms handoff, card segments keep a 2.5s minimum
/// hold, and every segment keeps its own text, motion, caption, assets and
/// opaque metadata. The container carries no preset/motion of its own.
fn fuse_chain(group: &[PlanItemV2], chain_index: u64) -> PlanItemV2 {
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
        if chain_family(&item.kind) != "text" && duration_us < 2_500_000 {
            duration_us = 2_500_000;
        }
        let segment_start_us = if index == 0 {
            item_start
        } else {
            cursor_us.saturating_add(400_000)
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
            camera_motion_id: item.camera_motion_id.clone(),
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
    params.insert("chain_gap_us".to_string(), json!(400_000_i64));
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
mod segment_contract_tests {
    use super::*;

    /// The canonical key set of one `chain_segments` entry. PipelineGen
    /// (`capmodel.ChainSegment`) and RenderingGen (`chainSegment`) decode these
    /// exact names, so adding or renaming a key here is a port-wide contract
    /// change, not a local refactor.
    const CANONICAL_KEYS: [&str; 27] = [
        "asset_id",
        "assets",
        "camera_motion_id",
        "caption_color",
        "caption_font_family",
        "caption_layout",
        "caption_motion_id",
        "caption_motion_params",
        "duration_us",
        "entity_caption",
        "entity_id",
        "entity_style_id",
        "frame",
        "group_id",
        "image_layers",
        "image_preset_id",
        "kind",
        "map",
        "motion_id",
        "motion_params",
        "params",
        "preset_id",
        "start_us",
        "style",
        "subgroup_id",
        "template_id",
        "text",
    ];

    fn fully_populated(id: &str, start_us: i64) -> PlanItemV2 {
        PlanItemV2 {
            id: id.to_string(),
            scene_id: "scene-1".to_string(),
            group_id: "group-a".to_string(),
            subgroup_id: "subgroup-a".to_string(),
            entity_id: "ent_ada".to_string(),
            kind: "entity_card".to_string(),
            start_ms: start_us / 1000,
            end_ms: (start_us + 2_000_000) / 1000,
            start_us,
            duration_us: 2_000_000,
            template_id: "person_default".to_string(),
            preset_id: "card_default".to_string(),
            image_preset_id: "image_scale_in".to_string(),
            motion_id: "card_reveal".to_string(),
            camera_motion_id: "camera_push".to_string(),
            entity_style_id: "premium_below_random_v1".to_string(),
            motion_params: Some(json!({"enter_frames": 17})),
            text: "Ada Lovelace".to_string(),
            entity_caption: "Matematica".to_string(),
            caption_motion_id: "caption_rise".to_string(),
            caption_motion_params: Some(json!({"rise_px": 12})),
            caption_font_family: "playfair_display_italic".to_string(),
            caption_layout: "below".to_string(),
            caption_color: "#000000".to_string(),
            style: Some(json!({"font_family": "Inter"})),
            asset_refs: vec![AssetRefV2 {
                asset_id: "portrait-ada".to_string(),
                url: "https://example.test/ada.jpg".to_string(),
                sha256: "1".repeat(64),
                media_type: "image/jpeg".to_string(),
            }],
            image_layers: Some(json!([{"asset_id": "layer-a"}])),
            frame: Some(json!({"x": 0})),
            map: Some(json!({"provider": "local"})),
            params: Some(json!({"recap": true})),
            ..Default::default()
        }
    }

    fn fused_segments(group: Vec<PlanItemV2>) -> Vec<Value> {
        let chain = fuse_chain(&group, 0);
        chain.params.as_ref().expect("chain params")["chain_segments"]
            .as_array()
            .expect("chain segments")
            .clone()
    }

    #[test]
    fn chain_segment_wire_keys_are_the_documented_contract() {
        let segments = fused_segments(vec![
            fully_populated("a", 100_000),
            fully_populated("b", 3_000_000),
        ]);
        assert_eq!(2, segments.len());
        let keys: std::collections::BTreeSet<&str> = segments[0]
            .as_object()
            .expect("segment object")
            .keys()
            .map(String::as_str)
            .collect();
        let want: std::collections::BTreeSet<&str> = CANONICAL_KEYS.iter().copied().collect();
        assert_eq!(want, keys, "chain segment wire keys drifted");
    }

    #[test]
    fn blank_string_fields_stay_off_the_wire() {
        let mut first = fully_populated("a", 0);
        first.caption_color = "   ".to_string();
        let mut second = fully_populated("b", 3_000_000);
        second.caption_color = "#ffffff".to_string();
        let segments = fused_segments(vec![first, second]);
        assert!(
            segments[0].get("caption_color").is_none(),
            "a whitespace-only segment field must not be emitted"
        );
        assert_eq!(
            Some("#ffffff"),
            segments[1]["caption_color"].as_str(),
            "a populated segment field must reach the wire"
        );
    }
}
