//! Chain coalescing: caption/entity-card fusion and the camera chain
//! segments. The fused item is one plan item, not a post-plan patch.

use super::*;

/// Ports `CoalesceChainedOverlaysWithSceneEnds`: nearby, same-scene overlays
/// fuse into one sequential chain only after admission. Image-backed item runs
/// are coalesced separately so a text card never interrupts a slideshow.
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
    let image_count = items
        .iter()
        .filter(|item| chain_family(&item.kind) == "image")
        .count();
    if image_count > 0 && image_count < items.len() {
        let mut images = Vec::new();
        let mut others = Vec::new();
        for item in items {
            if chain_family(&item.kind) == "image" {
                images.push(item);
            } else {
                others.push(item);
            }
        }
        let mut out = coalesce_chain_subset(images, max_gap_us, max_segments, scene_ends, 0);
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
pub(super) fn chain_family(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "text_phrase" | "important_word" | "keyword" | "quote" | "brand_text" | "brand-text"
        | "number" => "text",
        "image" | "entity_image" | "product" | "logo" => "image",
        "entity_card" | "organization" | "location" | "concept" | "lower_third" => "card",
        _ => "",
    }
}

fn captioned_entity_image_card(item: &PlanItemV2) -> bool {
    chain_family(&item.kind) == "card"
        && !item.asset_refs.is_empty()
        && !item.entity_caption.trim().is_empty()
        && item.sound_effect.is_none()
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

fn insert_non_empty(object: &mut serde_json::Map<String, Value>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_optional(object: &mut serde_json::Map<String, Value>, key: &str, value: &Option<Value>) {
    if let Some(value) = value.as_ref() {
        object.insert(key.to_string(), value.clone());
    }
}

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
        let mut segment = serde_json::Map::new();
        insert_non_empty(&mut segment, "text", &item.text);
        insert_non_empty(&mut segment, "group_id", &item.group_id);
        insert_non_empty(&mut segment, "subgroup_id", &item.subgroup_id);
        insert_non_empty(&mut segment, "motion_id", &item.motion_id);
        insert_non_empty(&mut segment, "preset_id", &item.preset_id);
        insert_non_empty(&mut segment, "image_preset_id", &item.image_preset_id);
        insert_non_empty(&mut segment, "template_id", &item.template_id);
        insert_non_empty(&mut segment, "kind", &item.kind);
        insert_non_empty(&mut segment, "entity_id", &item.entity_id);
        insert_non_empty(&mut segment, "entity_caption", &item.entity_caption);
        insert_non_empty(&mut segment, "caption_motion_id", &item.caption_motion_id);
        insert_optional(
            &mut segment,
            "caption_motion_params",
            &item.caption_motion_params,
        );
        insert_non_empty(
            &mut segment,
            "caption_font_family",
            &item.caption_font_family,
        );
        insert_non_empty(&mut segment, "caption_layout", &item.caption_layout);
        insert_non_empty(&mut segment, "caption_color", &item.caption_color);
        insert_non_empty(&mut segment, "entity_style_id", &item.entity_style_id);
        insert_non_empty(&mut segment, "camera_motion_id", &item.camera_motion_id);
        segment.insert("start_us".to_string(), json!(segment_start_us));
        segment.insert("duration_us".to_string(), json!(duration_us));
        insert_optional(&mut segment, "motion_params", &item.motion_params);
        if let Some(asset) = item.asset_refs.first() {
            segment.insert("asset_id".to_string(), json!(asset.asset_id));
        }
        if !item.asset_refs.is_empty() {
            segment.insert("assets".to_string(), json!(item.asset_refs));
        }
        insert_optional(&mut segment, "image_layers", &item.image_layers);
        insert_optional(&mut segment, "frame", &item.frame);
        insert_optional(&mut segment, "params", &item.params);
        insert_optional(&mut segment, "style", &item.style);
        insert_optional(&mut segment, "map", &item.map);
        if item
            .params
            .as_ref()
            .and_then(|params| params.get("recap"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            recap_count += 1;
        }
        segments.push(Value::Object(segment));
        motions.push(json!(item.motion_id));
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

/// Ports `CoalesceCaptionedEntityImageCards`: build runs from the card lane per
/// scene (not adjacency in the full list), fuse only same-scene runs within the
/// 7.5s entity-card gap, and keep the group standalone when the sequential
/// layout would cross the scene end.
pub(super) fn fuse_captioned_entity_image_chains(
    items: Vec<PlanItemV2>,
    scene_ends: &BTreeMap<String, i64>,
) -> Vec<PlanItemV2> {
    if items.len() < 2 {
        return items;
    }
    // No-op plans must stay byte-identical: only a run with at least two
    // captioned entity cards can produce a chain, so do not re-sort the rest.
    if items
        .iter()
        .filter(|item| captioned_entity_image_card(item))
        .count()
        < 2
    {
        return items;
    }
    let mut sorted = items;
    sorted.sort_by(|left, right| {
        left.window_us()
            .0
            .cmp(&right.window_us().0)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut by_scene: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, item) in sorted.iter().enumerate() {
        if captioned_entity_image_card(item) {
            by_scene
                .entry(item.scene_id.clone())
                .or_default()
                .push(index);
        }
    }
    let mut fused_at: BTreeMap<usize, PlanItemV2> = BTreeMap::new();
    let mut suppressed: BTreeSet<usize> = BTreeSet::new();
    for indices in by_scene.values() {
        let mut index = 0;
        while index < indices.len() {
            let first = indices[index];
            let mut group = vec![sorted[first].clone()];
            let mut members = vec![first];
            let mut group_end = sorted[first].window_us().1;
            let mut next = index + 1;
            while next < indices.len() {
                let candidate_index = indices[next];
                let candidate = &sorted[candidate_index];
                let gap = candidate.window_us().0.saturating_sub(group_end);
                if gap > 7_500_000 {
                    break;
                }
                group.push(candidate.clone());
                members.push(candidate_index);
                group_end = group_end.max(candidate.window_us().1);
                next += 1;
            }
            if group.len() > 1 {
                let scene_end = scene_ends.get(&group[0].scene_id).copied();
                if !chain_exceeds_scene_end(&group, scene_end) {
                    fused_at.insert(first, fuse_chain(&group, 1000 + first as u64));
                    for member in members.iter().skip(1) {
                        suppressed.insert(*member);
                    }
                }
            }
            index = next;
        }
    }
    let mut out = Vec::with_capacity(sorted.len().saturating_sub(suppressed.len()));
    for (index, item) in sorted.into_iter().enumerate() {
        if let Some(chain) = fused_at.remove(&index) {
            out.push(chain);
        } else if !suppressed.contains(&index) {
            out.push(item);
        }
    }
    out.sort_by(|left, right| {
        left.window_us()
            .0
            .cmp(&right.window_us().0)
            .then_with(|| left.id.cmp(&right.id))
    });
    out
}
