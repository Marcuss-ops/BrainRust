//! Presentation: text splitting, styles, motion assignment and the
//! light-leak inserts. Purely the look of an already-admitted item.

use super::*;

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
pub(super) fn apply_presentation(items: Vec<PlanItemV2>, plan_id: &str) -> Vec<PlanItemV2> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Some(parts) = split_long_overlay_text(&item, plan_id) {
            out.extend(parts);
            continue;
        }
        out.push(item);
    }
    for item in &mut out {
        normalize_presentation_item(item);
    }
    out
}

const PLAYFAIR_RUNTIME_FAMILY: &str = "playfair_display_italic";

const PRESENTATION_TEXT_FILL: &str = "#000000";

const PRESENTATION_GLOW_COLOR: &str = "#FFFFFF";

const PRESENTATION_GLOW_PX: f64 = 10.0;

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

fn presentation_text_params(base: Option<&Value>, font_size: f64) -> Value {
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
    params.insert("font_family".to_string(), json!(PLAYFAIR_RUNTIME_FAMILY));
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

fn presentation_text_style(base: Option<&Value>, font_size: f64) -> Value {
    let mut style = base.and_then(Value::as_object).cloned().unwrap_or_default();
    style.remove("background");
    style.remove("shadow");
    style.insert("font_family".to_string(), json!(PLAYFAIR_RUNTIME_FAMILY));
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
                    "font_family" | "caption_font_family"
                        if child.as_str().is_some_and(|v| !v.trim().is_empty()) =>
                    {
                        *child = json!(PLAYFAIR_RUNTIME_FAMILY);
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

fn normalize_presentation_item(item: &mut PlanItemV2) {
    if item.caption_font_family.trim().is_empty() {
        if !item.entity_caption.trim().is_empty() {
            item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
        }
    } else {
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
            item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
            item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
        }
        "number" => {
            let font_size = 140.0;
            item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
            item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
        }
        "quote" => {
            let font_size = 72.0;
            item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
            item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
        }
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
                let font_size = 96.0;
                item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
                item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
            }
        }
        _ => {}
    }
}

fn split_long_overlay_text(item: &PlanItemV2, plan_id: &str) -> Option<Vec<PlanItemV2>> {
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
        part.params = None;
        part.style = None;
        part.render_key.clear();
        part.text = chunk.clone();
        part.start_us = cursor;
        part.duration_us = chunk_duration;
        part.start_ms = cursor / 1000;
        part.end_ms = (cursor + chunk_duration + 999) / 1000;
        if part.motion_id.trim().is_empty() {
            part.motion_id = seeded_preset(
                plan_id,
                &part.scene_id,
                &part.id,
                "text_motion",
                &[
                    "text_phrase_rise",
                    "text_phrase_reveal",
                    "text_phrase_focus",
                ],
            )
            .unwrap_or_default();
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
