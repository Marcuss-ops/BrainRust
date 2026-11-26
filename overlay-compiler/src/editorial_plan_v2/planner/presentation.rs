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
    for mut item in items {
        // Candidate budgeting already split every text_phrase before
        // admission. Keep this call as the last-resort invariant for authored
        // or externally constructed chain items.
        split_long_chain_text(&mut item);
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

/// Splits every over-budget text candidate before admission. Doing this here
/// means the fragments are counted by the phrase budget and can never enter a
/// fused chain as one oversized segment.
pub(super) fn split_long_text_candidates(items: Vec<PlanItemV2>) -> Vec<PlanItemV2> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Some(parts) = split_long_overlay_text(&item) {
            out.extend(parts);
        } else {
            out.push(item);
        }
    }
    out
}

const PLAYFAIR_RUNTIME_FAMILY: &str = "playfair_display_italic";

/// Phrase-card family presets. Short spoken phrases are display copy: a
/// condensed sans at a larger size separates them from the long editorial
/// phrases, which keep the Playfair italic voice. The planner tags the item so
/// the later run-level `overlay_style` projection cannot flatten the two
/// families back into one look (ITCLOSE-06).
const SHORT_PHRASE_FONT_FAMILY: &str = "oswald";
const SHORT_PHRASE_FONT_SIZE_PX: f64 = 96.0;
const LONG_PHRASE_FONT_SIZE_PX: f64 = 72.0;
const SHORT_PHRASE_MARKER: &str = "short_phrase";
const LONG_PHRASE_MARKER: &str = "long_phrase";

const PRESENTATION_TEXT_FILL: &str = "#000000";

const PRESENTATION_GLOW_COLOR: &str = "#FFFFFF";

const PRESENTATION_GLOW_PX: f64 = 10.0;

/// One registry-valid default for split text. The previous seeded pool
/// contained retired ids (`text_phrase_reveal`/`text_phrase_focus`) that the
/// Go seal rejects, so do not reintroduce an unsynced motion list here.
const SPLIT_TEXT_MOTION: &str = "air_rise_type_on";

/// Maximum certified copy length for one text surface. The budget is applied
/// to candidates *before* admission and chain coalescing; the later split is
/// only a defensive invariant for directly constructed items.
pub(super) const MAX_TEXT_RUNES: usize = 180;

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
///
/// Owner: `overlay-compiler` planner presentation pass.
/// Removal: 2027-03-31, once every producer emits the runtime family.
/// Ticket: `docs/tickets/code-hygiene-2026-11-24/07-LEGACY-BRANCHES-AND-ARGS.md`.
/// Covered by
/// `planner/bullet_pin_tests.rs::legacy_playfair_alias_normalizes_but_other_families_survive`.
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

/// Chooses the visible family of a plain phrase card. Short copy (< 6 words)
/// uses Oswald at display size; long copy keeps Playfair italic at 72 px. Both
/// carry `params.phrase_family`, the marker the Go style projection checks so
/// the run-level overlay style cannot erase the distinction.
fn apply_phrase_family(item: &mut PlanItemV2) {
    // An explicit planner-side family (a pin or an authored parameter) stays
    // authoritative; the family preset only supplies the default voice.
    let explicit = font_in_object(item.params.as_ref().and_then(Value::as_object))
        .or_else(|| font_in_object(item.style.as_ref().and_then(Value::as_object)));
    let (family, font_size, marker) = if word_count(&item.text) < 6 {
        (
            explicit.unwrap_or_else(|| SHORT_PHRASE_FONT_FAMILY.to_string()),
            SHORT_PHRASE_FONT_SIZE_PX,
            SHORT_PHRASE_MARKER,
        )
    } else {
        (
            explicit.unwrap_or_else(|| PLAYFAIR_RUNTIME_FAMILY.to_string()),
            LONG_PHRASE_FONT_SIZE_PX,
            LONG_PHRASE_MARKER,
        )
    };
    let family = family.as_str();
    item.params = Some(presentation_text_params(
        item.params.as_ref(),
        font_size,
        family,
    ));
    item.style = Some(presentation_text_style(
        item.style.as_ref(),
        font_size,
        family,
    ));
    if let Some(params) = item.params.as_mut().and_then(Value::as_object_mut) {
        params.insert("phrase_family".to_string(), json!(marker));
    }
}

/// Chain-segment mirror of [`apply_phrase_family`]. A phrase fused into a
/// mixed chain must keep the same family as a standalone phrase card.
fn apply_phrase_family_to_segment(segment: &mut Value) {
    let Some(object) = segment.as_object_mut() else {
        return;
    };
    let kind = object.get("kind").and_then(Value::as_str).unwrap_or("");
    let template = object
        .get("template_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if kind != "text_phrase" && template != "IMPORTANT_PHRASE" {
        return;
    }
    // Bullet copy rides the same template but owns its typography through the
    // run-level bullet block and the per-scene pins.
    if object
        .get("params")
        .and_then(Value::as_object)
        .and_then(|params| params.get("editorial_role"))
        .and_then(Value::as_str)
        .is_some_and(|role| matches!(role, "scene_bullet" | "chapter_bullet"))
    {
        return;
    }
    let text = object.get("text").and_then(Value::as_str).unwrap_or("");
    if text.trim().is_empty() {
        // A segment without copy has no family to classify; keep its authored
        // typography (the round-trip chain fixture relies on this).
        return;
    }
    let explicit = font_in_object(object.get("params").and_then(Value::as_object))
        .or_else(|| font_in_object(object.get("style").and_then(Value::as_object)));
    let (family, font_size, marker) = if word_count(text) < 6 {
        (
            explicit.unwrap_or_else(|| SHORT_PHRASE_FONT_FAMILY.to_string()),
            SHORT_PHRASE_FONT_SIZE_PX,
            SHORT_PHRASE_MARKER,
        )
    } else {
        (
            explicit.unwrap_or_else(|| PLAYFAIR_RUNTIME_FAMILY.to_string()),
            LONG_PHRASE_FONT_SIZE_PX,
            LONG_PHRASE_MARKER,
        )
    };
    let family = family.as_str();
    let params = object
        .entry("params".to_string())
        .or_insert_with(|| json!({}));
    if let Some(params) = params.as_object_mut() {
        params.insert("font_family".to_string(), json!(family));
        params.insert("font_size_px".to_string(), json!(font_size));
        params.insert("fill".to_string(), json!(PRESENTATION_TEXT_FILL));
        params.insert("glow_size".to_string(), json!(PRESENTATION_GLOW_PX));
        params.insert("glow_color".to_string(), json!(PRESENTATION_GLOW_COLOR));
        params.insert("phrase_family".to_string(), json!(marker));
    }
    let style = object
        .entry("style".to_string())
        .or_insert_with(|| json!({}));
    if let Some(style) = style.as_object_mut() {
        style.insert("font_family".to_string(), json!(family));
        style.insert("font_size".to_string(), json!(font_size));
        style.insert("fill".to_string(), json!(PRESENTATION_TEXT_FILL));
        style.insert(
            "glow".to_string(),
            json!({"color": PRESENTATION_GLOW_COLOR}),
        );
    }
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
                    apply_phrase_family_to_segment(segment);
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
            if presentation_bullet_role(item.params.as_ref()) {
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
            } else {
                apply_phrase_family(item);
            }
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

mod split_text;
pub(crate) use split_text::*;
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
#[path = "bullet_pin_tests.rs"]
mod bullet_pin_tests;
