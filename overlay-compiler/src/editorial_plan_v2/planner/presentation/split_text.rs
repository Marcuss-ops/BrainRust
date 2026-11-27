use super::*;

pub(crate) fn split_long_overlay_text(item: &PlanItemV2) -> Option<Vec<PlanItemV2>> {
    let text = item.text.trim();
    if text.chars().count() <= MAX_TEXT_RUNES
        || item.kind != "text_phrase"
        || !item.asset_refs.is_empty()
    {
        return None;
    }
    let chunks = split_phrase_text(text, MAX_TEXT_RUNES);
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
        for key in [
            "editorial_role",
            "editorial_candidate",
            "priority",
            "timing_anchor",
        ] {
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
        // ANIMRAND-03: a split fragment is a normal phrase card. Its entrance
        // is selected later from the certified pool for its own length; the
        // planner no longer pins one hardcoded motion on every fragment, which
        // also made the long-phrase lane skip these items.
        out.push(part);
        cursor += chunk_duration;
    }
    (!out.is_empty()).then_some(out)
}

/// Chain items bypass `split_long_overlay_text` because the visible copy is
/// stored under `params.chain_segments`, not in a standalone `text_phrase`.
/// Keep the same 180-rune presentation limit inside chains as well: very long
/// full-screen text layers can make native GPU rendering fail, and they are
/// difficult to read as a single overlay. Each clone preserves its source
/// style/motion and receives a proportional, adjacent window within the
/// original segment interval.
pub(crate) fn split_long_chain_text(item: &mut PlanItemV2) {
    if !is_chain_kind(&item.kind) {
        return;
    }
    let Some(params) = item.params.as_mut().and_then(Value::as_object_mut) else {
        return;
    };
    let Some(segments) = params
        .get_mut("chain_segments")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let mut split = Vec::with_capacity(segments.len());
    let mut changed = false;
    for segment in segments.iter() {
        let Some(object) = segment.as_object() else {
            split.push(segment.clone());
            continue;
        };
        let is_text = object.get("kind").and_then(Value::as_str) == Some("text_phrase");
        let Some(text) = object.get("text").and_then(Value::as_str) else {
            split.push(segment.clone());
            continue;
        };
        let chunks = if is_text && text.chars().count() > MAX_TEXT_RUNES {
            split_phrase_text(text, MAX_TEXT_RUNES)
        } else {
            Vec::new()
        };
        if chunks.len() <= 1 {
            split.push(segment.clone());
            continue;
        }

        let start_us = object.get("start_us").and_then(Value::as_i64).unwrap_or(0);
        let duration_us = object
            .get("duration_us")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let total_chars: usize = chunks.iter().map(|chunk| chunk.chars().count()).sum();
        if duration_us <= 0 || total_chars == 0 {
            split.push(segment.clone());
            continue;
        }

        let mut cursor_us = start_us;
        for (index, chunk) in chunks.iter().enumerate() {
            let remaining_us = start_us + duration_us - cursor_us;
            if remaining_us <= 0 {
                break;
            }
            let chunk_us = if index + 1 == chunks.len() {
                remaining_us
            } else {
                (duration_us * chunk.chars().count() as i64 / total_chars as i64)
                    .max(1)
                    .min(remaining_us)
            };
            let mut part = object.clone();
            part.insert("text".to_string(), Value::String(chunk.clone()));
            part.insert("start_us".to_string(), Value::from(cursor_us));
            part.insert("duration_us".to_string(), Value::from(chunk_us));
            split.push(Value::Object(part));
            cursor_us += chunk_us;
        }
        changed = true;
    }
    if changed {
        let segment_count = split.len();
        let motions = split
            .iter()
            .filter_map(|segment| segment.get("motion_id").and_then(Value::as_str))
            .map(|motion| Value::String(motion.to_string()))
            .collect::<Vec<_>>();
        *segments = split;
        params.insert("chain_count".to_string(), Value::from(segment_count));
        params.insert("chain_motions".to_string(), Value::Array(motions));
    }
}

pub(crate) fn split_phrase_text(text: &str, max_runes: usize) -> Vec<String> {
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

pub(crate) fn name_items_for_review(items: &mut [PlanItemV2]) {
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
