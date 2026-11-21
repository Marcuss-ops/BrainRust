//! Entity-card → entity-image promotion, the single owner of the decision.
//!
//! Before this pass existed in Rust, PipelineGen rewrote the sealed plan: it
//! flipped `entity_card` segments to `entity_image`, re-selected their preset
//! and deleted their text/motion after Rust had ranked the plan. The wire now
//! carries `media.promotable_entity_assets` — the portraits the producer can
//! render but did **not** already ship as `verified_entity_assets` — and the
//! planner decides here, before it seals.
//!
//! The pass reproduces the Go media plane byte for byte, which is what keeps
//! the same request on the same entrance rotation:
//!
//! * a card that already carries assets is never promoted (`verified_entity_assets`
//!   cards are already `entity_image`; a promoted card is left untouched), so
//!   "has an asset" and "is promotable" stay disjoint sets;
//! * a chain segment is promoted **after** fusion and seeds its preset with the
//!   fused chain item id, exactly like the Go caller did;
//! * a segment rewrite touches only the declared surface (kind, template,
//!   preset, image preset, caption, asset id/assets) and drops the card-only
//!   keys (text, entity style, motion, params);
//! * the run-level entity-image ceiling is applied only when the pass promoted
//!   something, the same condition the Go caller used.

use super::*;

use crate::editorial_plan_v2::is_chain_kind;

/// The image treatment a promoted card always carries: RenderingGen requires
/// the fit preset explicitly when an entity image also has a composition
/// selector. Independent from the entrance preset chosen by the sampler.
const ENTITY_IMAGE_FIT_PRESET: &str = "image_scale_in";

/// Certified default run ceiling for image overlays, used when the request did
/// not resolve one. Mirrors `overlays.MaxImageOverlaysPerRun`.
const DEFAULT_ENTITY_IMAGE_CEILING: i64 = 20;

const CARD_KINDS: [&str; 4] = ["entity_card", "organization", "location", "concept"];

fn is_card_kind(kind: &str) -> bool {
    CARD_KINDS.contains(&kind.trim().to_ascii_lowercase().as_str())
}

fn is_image_kind(kind: &str) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "entity_image" | "image_popup"
    )
}

/// Mirrors the Go promotion guard: an explicit `images=disabled` wins for the
/// run, and a scene-level toggle can disable the lane for its own cards.
fn images_disabled(request: &EditorialPlanV2Request, scene_id: &str) -> bool {
    if let Some(scene) = request.scenes.iter().find(|scene| scene.id == scene_id) {
        if let Some(toggle) = scene.overlay_categories.get("images") {
            if !toggle.is_empty() && toggle != "default" {
                return toggle == "disabled";
            }
        }
    }
    request
        .canvas
        .overlay_categories
        .get("images")
        .map(|toggle| toggle == "disabled")
        .unwrap_or(false)
}

fn renderable_asset(entry: &PromotableEntityAssetV2) -> Option<&AssetRefV2> {
    entry
        .assets
        .iter()
        .find(|asset| !asset.url.trim().is_empty() && asset.sha256.trim().len() == 64)
}

/// Finds the promotable portrait of one identity. `entity_id` is the plan
/// occurrence identity the producer stamped; the canonical id is the fallback
/// for a card whose item identity was rewritten by a translation.
fn promotable_assets<'a>(
    media: &'a MediaV2,
    entity_id: &str,
    canonical_id: &str,
) -> Option<&'a PromotableEntityAssetV2> {
    let entity_id = entity_id.trim();
    let canonical_id = canonical_id.trim();
    media.promotable_entity_assets.iter().find(|entry| {
        (!entity_id.is_empty() && entry.entity_id.trim() == entity_id)
            || (!canonical_id.is_empty()
                && !entry.canonical_entity_id.trim().is_empty()
                && entry.canonical_entity_id.trim() == canonical_id)
    })
}

fn entity_caption_of(item: &PlanItemV2) -> String {
    let caption = item.entity_caption.trim();
    if !caption.is_empty() {
        caption.to_string()
    } else {
        item.text.trim().to_string()
    }
}

fn canonical_of(item: &PlanItemV2) -> String {
    item.params
        .as_ref()
        .and_then(|params| params.get("canonical_entity_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn promote_chain(
    request: &EditorialPlanV2Request,
    mut item: PlanItemV2,
    promoted: &mut i64,
) -> PlanItemV2 {
    let Some(media) = request.media.as_ref() else {
        return item;
    };
    let Some(mut params) = item.params.take() else {
        return item;
    };
    let Some(segments) = params
        .get_mut("chain_segments")
        .and_then(Value::as_array_mut)
    else {
        item.params = Some(params);
        return item;
    };
    // Read the seed fields before the loop: the body mutates the item.
    let scene_id = item.scene_id.clone();
    let item_id = item.id.clone();
    // Go promoted a segment with the fused chain item as the seed, so every
    // segment of one chain shares the chain's entrance preset.
    let preset = select_preset(
        &request.plan.plan_id,
        &scene_id,
        &item_id,
        "entity_image",
        &request.canvas.image_presets,
    );
    let mut count = 0_i64;
    let mut all_images = true;
    let mut seen_assets: BTreeSet<String> = item
        .asset_refs
        .iter()
        .map(|asset| asset.asset_id.clone())
        .filter(|id| !id.trim().is_empty())
        .collect();
    for (index, segment) in segments.iter_mut().enumerate() {
        let Some(object) = segment.as_object_mut() else {
            all_images = false;
            continue;
        };
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let has_assets = object
            .get("asset_id")
            .and_then(Value::as_str)
            .map(|id| !id.trim().is_empty())
            .unwrap_or(false)
            || object
                .get("assets")
                .and_then(Value::as_array)
                .map(|assets| !assets.is_empty())
                .unwrap_or(false)
            || object
                .get("image_layers")
                .and_then(Value::as_array)
                .map(|layers| !layers.is_empty())
                .unwrap_or(false);
        if !is_card_kind(&kind) || has_assets {
            if !is_image_kind(&kind) {
                all_images = false;
            }
            continue;
        }
        let entity_id = object
            .get("entity_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let canonical_id = object
            .get("params")
            .and_then(|params| params.get("canonical_entity_id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let Some(entry) = promotable_assets(media, &entity_id, &canonical_id) else {
            all_images = false;
            continue;
        };
        let Some(asset) = renderable_asset(entry) else {
            all_images = false;
            continue;
        };
        let caption = object
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        object.insert("kind".to_string(), json!("entity_image"));
        object.insert("template_id".to_string(), json!("image_popup"));
        object.insert("preset_id".to_string(), json!(preset.clone()));
        object.insert(
            "image_preset_id".to_string(),
            json!(ENTITY_IMAGE_FIT_PRESET),
        );
        object.insert("entity_caption".to_string(), json!(caption));
        object.insert("asset_id".to_string(), json!(asset.asset_id));
        object.insert(
            "assets".to_string(),
            Value::Array(vec![
                serde_json::to_value(asset).expect("asset ref serializes")
            ]),
        );
        for key in [
            "text",
            "entity_style_id",
            "motion_id",
            "motion_params",
            "params",
        ] {
            object.remove(key);
        }
        if seen_assets.insert(asset.asset_id.clone()) {
            item.asset_refs.push(asset.clone());
        }
        if index == 0 {
            item.preset_id = preset.clone();
            item.text.clear();
        }
        count += 1;
    }
    item.params = Some(params);
    if count > 0 {
        if all_images {
            item.kind = "image_chain".to_string();
            item.template_id = "IMAGE_CHAIN".to_string();
        }
        item.render_key.clear();
        *promoted += count;
    }
    item
}

fn promote_standalone(
    request: &EditorialPlanV2Request,
    mut item: PlanItemV2,
    promoted: &mut i64,
) -> PlanItemV2 {
    let Some(media) = request.media.as_ref() else {
        return item;
    };
    if !is_card_kind(&item.kind)
        || !item.asset_refs.is_empty()
        || item
            .image_layers
            .as_ref()
            .is_some_and(|layers| layers.as_array().is_some_and(|layers| !layers.is_empty()))
    {
        return item;
    }
    let canonical_id = canonical_of(&item);
    let Some(entry) = promotable_assets(media, &item.entity_id, &canonical_id) else {
        return item;
    };
    let Some(asset) = renderable_asset(entry) else {
        return item;
    };
    let caption = entity_caption_of(&item);
    item.asset_refs = vec![asset.clone()];
    item.kind = "entity_image".to_string();
    item.template_id = "image_popup".to_string();
    item.preset_id = select_preset(
        &request.plan.plan_id,
        &item.scene_id,
        &item.id,
        "entity_image",
        &request.canvas.image_presets,
    );
    item.image_preset_id = ENTITY_IMAGE_FIT_PRESET.to_string();
    item.entity_caption = caption;
    item.entity_style_id.clear();
    item.motion_id.clear();
    item.motion_params = None;
    item.text.clear();
    item.params = None;
    *promoted += 1;
    item
}

/// Drops entity images past the run ceiling. Identity is scene-scoped so a
/// portrait reused in a later scene keeps its own slot; nothing but an
/// `entity_image` is ever removed and the planner order is preserved.
fn cap_entity_images(items: Vec<PlanItemV2>, max: i64) -> Vec<PlanItemV2> {
    if max <= 0 {
        return items;
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        if item.kind != "entity_image" {
            kept.push(item);
            continue;
        }
        let identity = {
            let canonical = canonical_of(&item);
            if !canonical.is_empty() {
                canonical
            } else if !item.entity_id.trim().is_empty() {
                item.entity_id.trim().to_string()
            } else if let Some(asset) = item.asset_refs.first() {
                asset.sha256.trim().to_ascii_lowercase()
            } else {
                item.id.clone()
            }
        };
        let key = format!("{}::{identity}", item.scene_id.trim());
        if !seen.insert(key) || (seen.len() as i64) > max {
            continue;
        }
        kept.push(item);
    }
    kept
}

/// Promotes every card that has a promotable portrait. Called after the plan
/// has been clipped, named and ordered, so the seed id and the chain windows
/// are the ones the renderer receives.
pub(super) fn promote_entity_cards(
    request: &EditorialPlanV2Request,
    mut items: Vec<PlanItemV2>,
) -> Vec<PlanItemV2> {
    if request
        .media
        .as_ref()
        .map(|media| media.promotable_entity_assets.is_empty())
        .unwrap_or(true)
    {
        return items;
    }
    let mut promoted = 0_i64;
    for item in items.iter_mut() {
        if images_disabled(request, &item.scene_id) {
            continue;
        }
        let current = std::mem::take(item);
        *item = if is_chain_kind(&current.kind) {
            promote_chain(request, current, &mut promoted)
        } else {
            promote_standalone(request, current, &mut promoted)
        };
    }
    if promoted == 0 {
        return items;
    }
    let ceiling = if request.canvas.max_image_overlays > 0 {
        request.canvas.max_image_overlays
    } else {
        DEFAULT_ENTITY_IMAGE_CEILING
    };
    cap_entity_images(items, ceiling)
}

#[cfg(test)]
#[path = "promotion_tests.rs"]
mod tests;
