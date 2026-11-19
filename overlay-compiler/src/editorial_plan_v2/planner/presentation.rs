//! Presentation: text splitting, styles, motion assignment and the
//! light-leak inserts. Purely the look of an already-admitted item.

use super::*;
use crate::editorial_plan_v2::EditorialBulletStyleV2;

const LIGHT_LEAK_MOTIONS: &[&str] = &[
    "lightleak_anamorphic_pass",
    "lightleak_bokeh_pass",
    "lightleak_corner_bloom",
    "lightleak_diagonal_sweep",
    "lightleak_dual_edge",
    "lightleak_edge_warm",
    "lightleak_film_burn",
    "lightleak_focus_glow",
    "lightleak_horizontal_sweep",
    "lightleak_prismatic",
    "lightleak_pulse",
    "lightleak_soft_flare",
];

fn light_leak_motion(plan_id: &str, target_id: &str) -> String {
    seeded_preset(
        plan_id,
        target_id,
        target_id,
        "light_leak",
        LIGHT_LEAK_MOTIONS,
    )
    .unwrap_or_default()
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Ports `ClipStartLightLeakItems`: one short leak at every real video clip
/// start. Freeze tails are not clips and receive nothing.
pub(super) fn append_light_leak_items(
    request: &EditorialPlanV2Request,
    mut items: Vec<PlanItemV2>,
) -> Vec<PlanItemV2> {
    let seal_duration = request
        .seal
        .as_ref()
        .map(|seal| seal.audio_duration_us)
        .unwrap_or(0);
    if request.canvas.light_leak_on_clip_start {
        for clip in &request.clips {
            if clip.freeze
                || clip.asset_id.trim().is_empty()
                || clip.duration_us <= 0
                || clip.start_us < 0
                || (seal_duration > 0 && clip.start_us >= seal_duration)
            {
                continue;
            }
            let scene = request
                .scenes
                .iter()
                .find(|scene| scene.id == clip.scene_id);
            let Some(scene) = scene else {
                continue;
            };
            let mut duration = clip.duration_us.min(1_200_000);
            if seal_duration > 0 {
                duration = duration.min(seal_duration.saturating_sub(clip.start_us));
            }
            if duration <= 0 {
                continue;
            }
            let id = format!(
                "light-leak-clip-{}",
                short_hash(&format!(
                    "{}\0{}\0{}",
                    clip.scene_id, clip.index, clip.asset_id
                ))
            );
            let mut item = scene_item(
                id.clone(),
                scene,
                "light_leak",
                "LIGHT_LEAK",
                clip.start_us,
                duration,
                String::new(),
            );
            item.group_id = scene.id.clone();
            item.subgroup_id = id.clone();
            item.motion_id = light_leak_motion(&request.plan.plan_id, &id);
            items.push(item);
        }
    }
    if request.canvas.light_leak_on_overlay_start {
        let mut leaks = Vec::new();
        for overlay in items.iter() {
            if overlay.kind == "light_leak" || overlay.duration_us <= 0 || overlay.id.is_empty() {
                continue;
            }
            let Some(scene) = request
                .scenes
                .iter()
                .find(|scene| scene.id == overlay.scene_id)
            else {
                continue;
            };
            let id = format!(
                "light-leak-overlay-{}",
                short_hash(&format!("{}\0{}", request.plan.plan_id, overlay.id))
            );
            let duration = overlay.duration_us.min(1_200_000);
            let mut item = scene_item(
                id.clone(),
                scene,
                "light_leak",
                "LIGHT_LEAK",
                overlay.start_us,
                duration,
                String::new(),
            );
            item.group_id = overlay.group_id.clone();
            item.subgroup_id = id.clone();
            item.motion_id = light_leak_motion(&request.plan.plan_id, &id);
            leaks.push(item);
        }
        items.extend(leaks);
    }
    items
}

/// Visible review identity: motion, subject and overlay family. Mirrors
/// `overlays.NameItemsForReview`, including the per-prefix two-digit counter.
/// The final documentary presentation pass. It owns typography, font alias
/// normalization, image-caption placement, chain-segment captions and the
/// long-copy split. It never changes selection or timing windows except for
/// the split chunks, which conserve the original interval.
pub(super) fn apply_presentation(
    items: Vec<PlanItemV2>,
    _plan_id: &str,
    bullet_styles: &[EditorialBulletStyleV2],
) -> Vec<PlanItemV2> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Some(parts) = split_long_overlay_text(&item) {
            out.extend(parts);
            continue;
        }
        out.push(item);
    }
    for item in &mut out {
        normalize_presentation_item(item);
    }
    // Pins run last: the presentation pass intentionally normalizes every
    // text lane to the run default, so an earlier pin would be erased.
    apply_editorial_bullet_pins(&mut out, bullet_styles);
    out
}

const PLAYFAIR_RUNTIME_FAMILY: &str = "playfair_display_italic";

const PRESENTATION_TEXT_FILL: &str = "#000000";

const PRESENTATION_GLOW_COLOR: &str = "#FFFFFF";

const PRESENTATION_GLOW_PX: f64 = 10.0;

/// One registry-valid default for split text. The previous seeded pool
/// contained retired ids (`text_phrase_reveal`/`text_phrase_focus`) that the
/// Go seal rejects, so do not reintroduce an unsynced motion list here.
const SPLIT_TEXT_MOTION: &str = "air_rise_type_on";

const PRESENTATION_TEXT_STYLE_KEYS: &[&str] = &[
    "font_family",
    "font_size_px",
    "fill",
    "glow_size",
    "glow_color",
    "stroke_size",
    "shadow_blur_px",
    "shadow_opacity",
    "shadow_offset_x_px",
    "shadow_offset_y_px",
];

fn is_chain_kind(kind: &str) -> bool {
    matches!(
        kind,
        "phrase_chain" | "image_chain" | "mixed_chain" | "map_chain"
    )
}

/// The presentation pass owns the fallback typography, not an explicit
/// choice. `playfair_display` is a legacy preset alias whose only runtime
/// family is `playfair_display_italic`; every other family is a real payload
/// choice and must survive this pass. Go projects the run-level
/// `overlay_style` after sealing, so rewriting a family here created a second
/// writer and made the payload look ignored.
fn normalize_font_alias(font: &str) -> Option<String> {
    let font = font.trim();
    if font.is_empty() {
        return None;
    }
    if font.eq_ignore_ascii_case("playfair_display") {
        return Some(PLAYFAIR_RUNTIME_FAMILY.to_string());
    }
    Some(font.to_string())
}

fn font_in_object(object: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    let object = object?;
    if let Some(font) = object
        .get("font_family")
        .and_then(Value::as_str)
        .and_then(normalize_font_alias)
    {
        return Some(font);
    }
    font_in_object(object.get("style").and_then(Value::as_object))
}

fn presentation_font_family(params: Option<&Value>, style: Option<&Value>) -> String {
    font_in_object(params.and_then(Value::as_object))
        .or_else(|| font_in_object(style.and_then(Value::as_object)))
        .unwrap_or_else(|| PLAYFAIR_RUNTIME_FAMILY.to_string())
}

fn presentation_text_params(base: Option<&Value>, font_size: f64, font_family: &str) -> Value {
    let mut params = base.and_then(Value::as_object).cloned().unwrap_or_default();
    params.remove("background");
    for key in [
        "shadow_color",
        "shadow_blur_px",
        "shadow_opacity",
        "shadow_offset_x_px",
        "shadow_offset_y_px",
    ] {
        params.remove(key);
    }
    params.insert("font_family".to_string(), json!(font_family));
    params.insert("fill".to_string(), json!(PRESENTATION_TEXT_FILL));
    params.insert("glow_size".to_string(), json!(PRESENTATION_GLOW_PX));
    params.insert("glow_color".to_string(), json!(PRESENTATION_GLOW_COLOR));
    params.insert("font_size_px".to_string(), json!(font_size));
    params.insert("width".to_string(), json!(1700.0));
    params.insert("height".to_string(), json!(600.0));
    params.insert("position".to_string(), json!("center"));
    params.remove("position_x");
    Value::Object(params)
}

fn presentation_text_style(base: Option<&Value>, font_size: f64, font_family: &str) -> Value {
    let mut style = base.and_then(Value::as_object).cloned().unwrap_or_default();
    style.remove("background");
    style.remove("shadow");
    style.insert("font_family".to_string(), json!(font_family));
    style.insert("fill".to_string(), json!(PRESENTATION_TEXT_FILL));
    style.insert(
        "glow".to_string(),
        json!({"color": PRESENTATION_GLOW_COLOR}),
    );
    style.insert("font_size".to_string(), json!(font_size));
    Value::Object(style)
}

fn strip_presentation_text_keys(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for key in PRESENTATION_TEXT_STYLE_KEYS {
            object.remove(*key);
        }
    }
}

fn normalize_fonts(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                match key.as_str() {
                    "font_family" | "caption_font_family" => {
                        if let Some(font) = child.as_str().and_then(normalize_font_alias) {
                            *child = json!(font);
                        }
                    }
                    "caption_layout" => *child = json!("below"),
                    "caption_motion_id" => {
                        if child
                            .as_str()
                            .is_some_and(|v| v.to_ascii_lowercase().contains("split_side"))
                        {
                            *child = json!("");
                        }
                    }
                    _ => normalize_fonts(child),
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize_fonts(child);
            }
        }
        _ => {}
    }
}

/// Rewrites the text lanes of one item to the presentation default. The
/// caller owns the size decision (long copy, number, quote, card); family
/// and fill/glow are the same contract for every lane.
fn normalize_text_item(item: &mut PlanItemV2, font_size: f64) {
    let font_family = presentation_font_family(item.params.as_ref(), item.style.as_ref());
    item.params = Some(presentation_text_params(
        item.params.as_ref(),
        font_size,
        &font_family,
    ));
    item.style = Some(presentation_text_style(
        item.style.as_ref(),
        font_size,
        &font_family,
    ));
}

fn normalize_presentation_item(item: &mut PlanItemV2) {
    // One writer for the caption default: only a card that actually carries a
    // caption gets the runtime family. Earlier this branch assigned the same
    // constant on both paths, which looked like a second author overriding the
    // value `media.rs` had already set.
    if !item.caption_font_family.trim().is_empty() || !item.entity_caption.trim().is_empty() {
        item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
    }
    if !item.entity_caption.trim().is_empty() {
        item.caption_layout = "below".to_string();
        item.caption_color = PRESENTATION_TEXT_FILL.to_string();
    }
    if item
        .caption_motion_id
        .to_ascii_lowercase()
        .contains("split_side")
    {
        item.caption_motion_id.clear();
    }
    if let Some(params) = item.params.as_mut() {
        normalize_fonts(params);
    }
    if let Some(style) = item.style.as_mut() {
        normalize_fonts(style);
    }
    if is_chain_kind(&item.kind) {
        item.caption_layout = "below".to_string();
        item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
        if let Some(params) = item.params.as_mut() {
            strip_presentation_text_keys(params);
            if let Some(segments) = params
                .get_mut("chain_segments")
                .and_then(Value::as_array_mut)
            {
                for segment in segments {
                    normalize_fonts(segment);
                    if let Some(object) = segment.as_object_mut() {
                        let has_caption = object
                            .get("entity_caption")
                            .and_then(Value::as_str)
                            .is_some_and(|caption| !caption.trim().is_empty());
                        if has_caption {
                            object.insert("caption_layout".to_string(), json!("below"));
                            object.insert(
                                "caption_font_family".to_string(),
                                json!(PLAYFAIR_RUNTIME_FAMILY),
                            );
                            object
                                .insert("caption_color".to_string(), json!(PRESENTATION_TEXT_FILL));
                        }
                    }
                }
            }
        }
        if let Some(style) = item.style.as_mut() {
            strip_presentation_text_keys(style);
        }
        return;
    }
    match item.kind.as_str() {
        "text_phrase" | "important_phrase" => {
            let runes = item.text.chars().count();
            let existing = item
                .params
                .as_ref()
                .and_then(|params| params.get("font_size_px"))
                .and_then(Value::as_f64)
                .unwrap_or(72.0);
            let font_size = if runes > 160 {
                64.0
            } else if runes > 120 {
                existing.min(72.0)
            } else {
                existing
            };
            normalize_text_item(item, font_size);
        }
        "number" => normalize_text_item(item, 140.0),
        "quote" => normalize_text_item(item, 72.0),
        "entity_image" | "entity_card" | "location" | "organization" | "concept" => {
            if !item.asset_refs.is_empty() || !item.entity_caption.trim().is_empty() {
                item.caption_layout = "below".to_string();
                item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
                item.caption_color = PRESENTATION_TEXT_FILL.to_string();
                item.entity_style_id = "premium_below_random_v1".to_string();
                if let Some(params) = item.params.as_mut() {
                    strip_presentation_text_keys(params);
                    if let Some(object) = params.as_object_mut() {
                        object.insert("caption_layout".to_string(), json!("below"));
                        object.insert("position_x".to_string(), json!(0.0));
                        object.entry("position_y".to_string()).or_insert(json!(0.0));
                    }
                }
                item.style = None;
            } else {
                normalize_text_item(item, 96.0);
            }
        }
        _ => {}
    }
}

/// True for the two editorial roles that ride the bullet lane. Chapter cards
/// share the lane and therefore the same pin contract.
fn presentation_bullet_role(params: Option<&Value>) -> bool {
    params
        .and_then(Value::as_object)
        .and_then(|object| object.get("editorial_role"))
        .and_then(Value::as_str)
        .is_some_and(|role| matches!(role, "scene_bullet" | "chapter_bullet"))
}

fn write_pin_text(
    object: &mut serde_json::Map<String, Value>,
    pin: &EditorialBulletStyleV2,
    size_key: &str,
) {
    let family = pin.font_family.trim();
    if !family.is_empty() {
        object.insert("font_family".to_string(), json!(family));
    }
    if pin.font_size_px > 0.0 {
        object.insert(size_key.to_string(), json!(pin.font_size_px));
    }
}

fn apply_pin_text(
    params: &mut Option<Value>,
    style: &mut Option<Value>,
    pin: &EditorialBulletStyleV2,
) {
    if pin.font_family.trim().is_empty() && pin.font_size_px <= 0.0 {
        return;
    }
    let params_object = params.get_or_insert_with(|| json!({}));
    if let Some(object) = params_object.as_object_mut() {
        write_pin_text(object, pin, "font_size_px");
    }
    let style_object = style.get_or_insert_with(|| json!({}));
    if let Some(object) = style_object.as_object_mut() {
        write_pin_text(object, pin, "font_size");
    }
}

fn apply_pin_to_item(item: &mut PlanItemV2, pin: &EditorialBulletStyleV2) {
    apply_pin_text(&mut item.params, &mut item.style, pin);
    if !pin.motion_id.trim().is_empty() {
        item.motion_id = pin.motion_id.trim().to_string();
    }
}

fn apply_pin_to_chain_segment(segment: &mut Value, pin: &EditorialBulletStyleV2) {
    let Some(object) = segment.as_object_mut() else {
        return;
    };
    if !pin.font_family.trim().is_empty() || pin.font_size_px > 0.0 {
        let params = object
            .entry("params".to_string())
            .or_insert_with(|| json!({}));
        if let Some(params_object) = params.as_object_mut() {
            write_pin_text(params_object, pin, "font_size_px");
        }
        let style = object
            .entry("style".to_string())
            .or_insert_with(|| json!({}));
        if let Some(style_object) = style.as_object_mut() {
            write_pin_text(style_object, pin, "font_size");
        }
    }
    if !pin.motion_id.trim().is_empty() {
        object.insert("motion_id".to_string(), json!(pin.motion_id.trim()));
    }
}

/// Applies the per-scene bullet pins after the documentary presentation pass.
/// A fused chain keeps the pin on its bullet segment only; sibling segments
/// keep the run-level typography and their own motions.
fn apply_editorial_bullet_pins(items: &mut [PlanItemV2], pins: &[EditorialBulletStyleV2]) {
    if pins.is_empty() {
        return;
    }
    let pins: BTreeMap<&str, &EditorialBulletStyleV2> = pins
        .iter()
        .filter(|pin| !pin.scene_id.trim().is_empty())
        .map(|pin| (pin.scene_id.trim(), pin))
        .collect();
    for item in items.iter_mut() {
        let Some(pin) = pins.get(item.scene_id.trim()).copied() else {
            continue;
        };
        if is_chain_kind(&item.kind) {
            let Some(segments) = item
                .params
                .as_mut()
                .and_then(Value::as_object_mut)
                .and_then(|params| params.get_mut("chain_segments"))
                .and_then(Value::as_array_mut)
            else {
                continue;
            };
            for segment in segments {
                let is_bullet = segment
                    .as_object()
                    .and_then(|segment| segment.get("params"))
                    .is_some_and(|params| presentation_bullet_role(Some(params)));
                if is_bullet {
                    apply_pin_to_chain_segment(segment, pin);
                }
            }
            continue;
        }
        if presentation_bullet_role(item.params.as_ref()) {
            apply_pin_to_item(item, pin);
        }
    }
}

fn split_long_overlay_text(item: &PlanItemV2) -> Option<Vec<PlanItemV2>> {
    let text = item.text.trim();
    if text.chars().count() <= 180 || item.kind != "text_phrase" || !item.asset_refs.is_empty() {
        return None;
    }
    let chunks = split_phrase_text(text, 180);
    if chunks.len() <= 1 {
        return None;
    }
    let (start_us, duration_us) = item.window_us();
    let total: usize = chunks.iter().map(|chunk| chunk.chars().count()).sum();
    if total == 0 {
        return None;
    }
    let mut kept_params = serde_json::Map::new();
    if let Some(params) = item.params.as_ref().and_then(Value::as_object) {
        for key in ["editorial_role", "editorial_candidate", "priority"] {
            if let Some(value) = params.get(key) {
                kept_params.insert(key.to_string(), value.clone());
            }
        }
    }
    let mut out = Vec::with_capacity(chunks.len());
    let mut cursor = start_us;
    for (index, chunk) in chunks.iter().enumerate() {
        let chunk_duration = if index + 1 == chunks.len() {
            start_us + duration_us - cursor
        } else {
            (duration_us * chunk.chars().count() as i64 / total as i64).max(1_200_000)
        }
        .min(start_us + duration_us - cursor);
        if chunk_duration <= 0 {
            continue;
        }
        let mut part = item.clone();
        part.id = format!("{}-part-{}", item.id, (b'a' + index as u8) as char);
        part.kind = "text_phrase".to_string();
        part.template_id = "IMPORTANT_PHRASE".to_string();
        part.preset_id = "phrase_default".to_string();
        part.image_preset_id.clear();
        part.entity_caption.clear();
        part.caption_motion_id.clear();
        part.caption_font_family.clear();
        part.caption_layout.clear();
        part.caption_color.clear();
        part.asset_refs.clear();
        part.image_layers = None;
        part.group_motion_id.clear();
        part.frame = None;
        part.map = None;
        part.entity_id.clear();
        // Keep the editorial role on every part: the bullet pin is applied
        // after this split and must still recognize the split as the bullet.
        part.params = if kept_params.is_empty() {
            None
        } else {
            Some(Value::Object(kept_params.clone()))
        };
        part.style = None;
        part.render_key.clear();
        part.text = chunk.clone();
        part.start_us = cursor;
        part.duration_us = chunk_duration;
        part.start_ms = cursor / 1000;
        part.end_ms = (cursor + chunk_duration + 999) / 1000;
        if part.motion_id.trim().is_empty() {
            part.motion_id = SPLIT_TEXT_MOTION.to_string();
        }
        out.push(part);
        cursor += chunk_duration;
    }
    (!out.is_empty()).then_some(out)
}

fn split_phrase_text(text: &str, max_runes: usize) -> Vec<String> {
    let mut runes: Vec<char> = text.trim().chars().collect();
    let mut out = Vec::new();
    while !runes.is_empty() {
        if runes.len() <= max_runes {
            let chunk = runes.iter().collect::<String>().trim().to_string();
            if !chunk.is_empty() {
                out.push(chunk);
            }
            break;
        }
        let mut cut = max_runes;
        for index in (80..max_runes).rev() {
            if matches!(runes[index - 1], '.' | '!' | '?' | ';' | ':') {
                cut = index;
                break;
            }
        }
        if cut == max_runes {
            for index in (80..max_runes).rev() {
                if runes[index - 1] == ' ' {
                    cut = index;
                    break;
                }
            }
        }
        let chunk = runes[..cut].iter().collect::<String>().trim().to_string();
        if !chunk.is_empty() {
            out.push(chunk);
        }
        runes.drain(..cut);
        while runes.first() == Some(&' ') {
            runes.remove(0);
        }
    }
    if out.is_empty() {
        vec![text.to_string()]
    } else {
        out
    }
}

pub(super) fn name_items_for_review(items: &mut [PlanItemV2]) {
    let mut seen: BTreeMap<String, i64> = BTreeMap::new();
    for item in items.iter_mut() {
        let motion = if !item.motion_id.trim().is_empty() {
            item.motion_id.trim().to_string()
        } else {
            "static".to_string()
        };
        let mut entity = String::new();
        if let Some(map) = item.map.as_ref() {
            entity = map
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if entity.is_empty() {
            entity = item.text.trim().to_string();
        }
        if entity.is_empty() {
            entity = item.entity_caption.trim().to_string();
        }
        if entity.is_empty() {
            entity = item.id.clone();
        }
        let category = name_category(&item.kind, &motion);
        let base = format!(
            "{}__{}__{}",
            name_slug(&motion),
            name_slug(&entity),
            category
        );
        let count = seen.entry(base.clone()).or_default();
        *count += 1;
        item.id = format!("{base}__{count:02}");
    }
}

fn name_category(kind: &str, motion: &str) -> String {
    match kind {
        "map" => "map".to_string(),
        "text_phrase" if motion.starts_with("short_phrase_") => "short_phrase".to_string(),
        "text_phrase" => "phrase".to_string(),
        "number" => "date_number".to_string(),
        "entity_image" | "entity_card" => "entity_image".to_string(),
        "image" => "image".to_string(),
        "quote" => "quote".to_string(),
        "brand_text" => "brand_text".to_string(),
        other => name_slug(other),
    }
}

pub(super) fn name_slug(value: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::new();
    let mut separator = false;
    for ch in value.trim().to_lowercase().nfd() {
        if unicode_normalization::char::is_combining_mark(ch) {
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            if separator && !out.is_empty() {
                out.push('-');
            }
            out.push(ch);
            separator = false;
        } else {
            separator = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let mut result = trimmed.chars().take(48).collect::<String>();
    result = result.trim_end_matches('-').to_string();
    if result.is_empty() {
        "unnamed".to_string()
    } else {
        result
    }
}

#[cfg(test)]
mod bullet_pin_tests {
    use super::*;

    fn pin() -> EditorialBulletStyleV2 {
        EditorialBulletStyleV2 {
            scene_id: "scene-1".to_string(),
            font_family: "oswald".to_string(),
            font_size_px: 96.0,
            motion_id: "air_rise_type_on".to_string(),
        }
    }

    fn bullet_item(role: &str) -> PlanItemV2 {
        PlanItemV2 {
            id: "bullet".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "text_phrase".to_string(),
            template_id: "IMPORTANT_PHRASE".to_string(),
            text: "Il debito pubblico sale al nord".to_string(),
            params: Some(json!({
                "editorial_role": role,
                "font_family": "playfair_display_italic"
            })),
            style: Some(json!({
                "font_family": "playfair_display_italic",
                "font_size": 72.0
            })),
            ..Default::default()
        }
    }

    #[test]
    fn pin_wins_over_the_presentation_font_and_sets_the_entrance() {
        let items = apply_presentation(vec![bullet_item("scene_bullet")], "plan", &[pin()]);
        let item = &items[0];
        let params = item.params.as_ref().expect("params");
        let style = item.style.as_ref().expect("style");
        assert_eq!(Some("oswald"), params["font_family"].as_str());
        assert_eq!(Some(96.0), params["font_size_px"].as_f64());
        assert_eq!(Some("oswald"), style["font_family"].as_str());
        assert_eq!(Some(96.0), style["font_size"].as_f64());
        assert_eq!("air_rise_type_on", item.motion_id);
    }

    #[test]
    fn chapter_bullet_uses_the_same_pin_lane() {
        let items = apply_presentation(vec![bullet_item("chapter_bullet")], "plan", &[pin()]);
        assert_eq!("air_rise_type_on", items[0].motion_id);
        assert_eq!(
            Some("oswald"),
            items[0].params.as_ref().expect("params")["font_family"].as_str()
        );
    }

    #[test]
    fn non_bullet_text_keeps_the_presentation_default() {
        let items = apply_presentation(vec![bullet_item("")], "plan", &[pin()]);
        let item = &items[0];
        assert_ne!("air_rise_type_on", item.motion_id);
        assert_eq!(
            Some("playfair_display_italic"),
            item.params.as_ref().expect("params")["font_family"].as_str()
        );
    }

    #[test]
    fn split_bullet_keeps_the_pin_on_every_part() {
        let mut bullet = bullet_item("scene_bullet");
        bullet.text = "Il debito pubblico sale al nord. ".repeat(10);
        bullet.duration_us = 6_000_000;
        bullet.end_ms = 6_000;
        let items = apply_presentation(vec![bullet], "plan", &[pin()]);
        assert!(items.len() > 1, "long bullet should split");
        for item in &items {
            let params = item.params.as_ref().expect("params");
            assert_eq!(Some("scene_bullet"), params["editorial_role"].as_str());
            assert_eq!(Some("oswald"), params["font_family"].as_str());
            assert_eq!(Some(96.0), params["font_size_px"].as_f64());
            assert_eq!("air_rise_type_on", item.motion_id);
        }
    }

    #[test]
    fn fused_chain_pins_only_its_bullet_segment() {
        let chain = PlanItemV2 {
            id: "chain".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "phrase_chain".to_string(),
            template_id: "PHRASE_CHAIN".to_string(),
            text: "Il debito pubblico sale al nord".to_string(),
            params: Some(json!({"chain_segments": [
                {
                    "kind": "text_phrase", "template_id": "IMPORTANT_PHRASE",
                    "motion_id": "motion_a", "start_us": 0, "duration_us": 2_000_000,
                    "params": {"editorial_role": "scene_bullet", "font_family": "playfair_display_italic"},
                    "style": {"font_family": "playfair_display_italic", "font_size": 72.0}
                },
                {
                    "kind": "text_phrase", "template_id": "IMPORTANT_PHRASE",
                    "motion_id": "motion_b", "start_us": 2_400_000, "duration_us": 2_000_000,
                    "params": {"font_family": "playfair_display_italic"},
                    "style": {"font_family": "playfair_display_italic", "font_size": 72.0}
                }
            ]})),
            ..Default::default()
        };
        let items = apply_presentation(vec![chain], "plan", &[pin()]);
        let segments = items[0].params.as_ref().expect("params")["chain_segments"]
            .as_array()
            .expect("segments");
        assert_eq!(
            "air_rise_type_on",
            segments[0]["motion_id"].as_str().unwrap()
        );
        assert_eq!(
            "oswald",
            segments[0]["params"]["font_family"].as_str().unwrap()
        );
        assert_eq!(
            96.0,
            segments[0]["params"]["font_size_px"].as_f64().unwrap()
        );
        assert_eq!(
            "oswald",
            segments[0]["style"]["font_family"].as_str().unwrap()
        );
        assert_eq!("motion_b", segments[1]["motion_id"].as_str().unwrap());
        assert_eq!(
            "playfair_display_italic",
            segments[1]["params"]["font_family"].as_str().unwrap()
        );
    }

    #[test]
    fn number_card_preserves_an_explicit_payload_font() {
        let item = PlanItemV2 {
            id: "metric".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "number".to_string(),
            template_id: "METRIC_STAT_CARD".to_string(),
            text: "42 per cento".to_string(),
            params: Some(json!({"font_family": "oswald", "font_size_px": 88.0})),
            style: Some(json!({"font_family": "oswald", "font_size": 88.0})),
            ..Default::default()
        };
        let items = apply_presentation(vec![item], "plan", &[]);
        let params = items[0].params.as_ref().expect("params");
        let style = items[0].style.as_ref().expect("style");
        assert_eq!(Some("oswald"), params["font_family"].as_str());
        assert_eq!(Some("oswald"), style["font_family"].as_str());
        assert_eq!(Some(140.0), params["font_size_px"].as_f64());
    }

    #[test]
    fn legacy_playfair_alias_normalizes_but_other_families_survive() {
        let item = PlanItemV2 {
            id: "date".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "number".to_string(),
            template_id: "TIMELINE_DATE_CARD".to_string(),
            text: "3 giugno 2007".to_string(),
            params: Some(json!({"font_family": "playfair_display"})),
            style: Some(json!({"font_family": "bricolage_grotesque"})),
            ..Default::default()
        };
        let items = apply_presentation(vec![item], "plan", &[]);
        let params = items[0].params.as_ref().expect("params");
        assert_eq!(
            Some("playfair_display_italic"),
            params["font_family"].as_str()
        );

        let mut other = PlanItemV2 {
            id: "number".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "number".to_string(),
            template_id: "METRIC_STAT_CARD".to_string(),
            text: "42".to_string(),
            params: Some(json!({"font_family": "abril_fatface"})),
            ..Default::default()
        };
        other.style = Some(json!({"font_family": "abril_fatface"}));
        let items = apply_presentation(vec![other], "plan", &[]);
        assert_eq!(
            Some("abril_fatface"),
            items[0].params.as_ref().expect("params")["font_family"].as_str()
        );
    }
}
