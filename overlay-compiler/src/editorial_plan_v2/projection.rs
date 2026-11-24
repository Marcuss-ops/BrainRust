//! WP4 raw-surface projector.
//!
//! The producer supplies grounded phrases, entities, occurrences, chart
//! authorizations and the numeric vocabulary. Every operation that chooses,
//! drops, times or retimes a surface happens here, so the Go request builder
//! cannot be a second planner authority.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
use semantic_core::entity_capabilities::{fold_entity_kind, EntityKind};
use semantic_core::text::{fragment_tokens, is_numeric_token, phrase_tokens};

use crate::numeric::parse_chart_value;

use super::request::{
    CanvasV2, EditorialPlanV2Request, EntityTimelineV2, HeavySentenceV2, ImageCandidateV2,
    ImagePlanV2, MediaV2, NumericLexiconV2, RawEntityV2, RawGeoV2, RawMentionV2, RawSceneV2,
    RecapBriefV2, SceneAnnotationsV2, SceneV2, TimedAnnotationV2, VisualRecipeSpecV2,
};

#[cfg(test)]
mod projection_tests;

/// Projects every scene that carries a raw surface. Existing annotations are
/// preserved only for media candidates (the producer's materialized images),
/// never for a text decision.
pub(crate) fn project_request(request: &mut EditorialPlanV2Request) {
    let canvas = request.canvas.clone();
    let lexicon = request.numeric_lexicon.clone().unwrap_or_default();
    let heavy = request.heavy_sentences.clone();
    let language = request.language.clone();
    let source_language = request.source_language.clone();
    let entity_timeline = request.entity_timeline.clone();
    let media = request.media.clone();
    let image_plan = request.image_plan.clone();
    let images_by_scene = allocate_scene_images(media.as_ref(), image_plan.as_ref());
    let mut projected_raw = false;
    for scene in &mut request.scenes {
        let Some(raw) = scene.raw.take() else {
            continue;
        };
        projected_raw = true;
        let existing_images = scene
            .annotations
            .as_ref()
            .map(|annotations| annotations.images.clone())
            .unwrap_or_default();
        let mut projected = project_scene(SceneProjection {
            scene,
            raw: &raw,
            canvas: &canvas,
            lexicon: &lexicon,
            heavy: &heavy,
            language: &language,
            source_language: &source_language,
            timeline: entity_timeline.as_ref(),
        });
        projected.images = if image_plan.is_some() {
            images_by_scene.get(&scene.id).cloned().unwrap_or_default()
        } else {
            existing_images
        };
        scene.annotations = Some(projected);
    }
    if projected_raw {
        project_recap_briefs(request);
    }
}

mod scenes;
use scenes::*;
fn map_candidate(
    entity_id: &str,
    label: &str,
    geo: &RawGeoV2,
    start_us: i64,
    duration_us: i64,
    score: f64,
) -> Option<super::request::MapCandidateV2> {
    if entity_id.trim().is_empty() || label.trim().is_empty() {
        return None;
    }
    if !geo.latitude.is_finite()
        || !geo.longitude.is_finite()
        || !(-90.0..=90.0).contains(&geo.latitude)
        || !(-180.0..=180.0).contains(&geo.longitude)
        || start_us < 0
        || duration_us <= 0
    {
        return None;
    }
    Some(super::request::MapCandidateV2 {
        entity_id: entity_id.to_string(),
        label: label.split_whitespace().collect::<Vec<_>>().join(" "),
        scope: geo.scope.trim().to_lowercase(),
        latitude: geo.latitude,
        longitude: geo.longitude,
        start_ms: start_us / 1000,
        end_ms: (start_us + duration_us + 999) / 1000,
        start_us,
        duration_us,
        score,
    })
}

fn timed(
    text: &str,
    annotation_type: &str,
    start_us: i64,
    end_us: i64,
    score: f64,
) -> TimedAnnotationV2 {
    TimedAnnotationV2 {
        text: text.trim().to_string(),
        annotation_type: annotation_type.to_string(),
        start_ms: start_us / 1000,
        end_ms: (end_us + 999) / 1000,
        start_us,
        duration_us: (end_us - start_us).max(0),
        score,
        presentation_role: String::new(),
        editorial_candidate: String::new(),
    }
}

fn locate_phrase(
    timing: &super::request::TimingV2,
    timeline_start_us: i64,
    phrase: &str,
) -> Option<(i64, i64)> {
    let query = phrase_tokens(phrase);
    if query.is_empty() {
        return None;
    }
    let mut normalized = Vec::new();
    let mut source_word = Vec::new();
    for (word_index, word) in timing.words.iter().enumerate() {
        for token in fragment_tokens(&word.text) {
            normalized.push(token);
            source_word.push(word_index);
        }
    }
    if query.len() > normalized.len() {
        // Keep the endpoint fallback available even when the query has more
        // tokens than the timing artifact.
        return endpoint_phrase_timing(
            timing,
            timeline_start_us,
            phrase,
            &query,
            &normalized,
            &source_word,
        );
    }
    for start in 0..=normalized.len().saturating_sub(query.len()) {
        if query
            .iter()
            .enumerate()
            .all(|(offset, token)| tokens_match(token, &normalized[start + offset]))
        {
            let first = &timing.words[source_word[start]];
            let last = &timing.words[source_word[start + query.len() - 1]];
            return Some((
                timeline_start_us + first.start_us,
                timeline_start_us + last.end_us,
            ));
        }
    }
    // Endpoint fallback: exact first/last word boundaries, choosing the pair
    // whose certified span length most closely matches the source phrase.
    endpoint_phrase_timing(
        timing,
        timeline_start_us,
        phrase,
        &query,
        &normalized,
        &source_word,
    )
}

fn endpoint_phrase_timing(
    timing: &super::request::TimingV2,
    timeline_start_us: i64,
    _phrase: &str,
    query: &[String],
    normalized: &[String],
    source_word: &[usize],
) -> Option<(i64, i64)> {
    if query.len() < 2 {
        return None;
    }
    let firsts: Vec<usize> = normalized
        .iter()
        .enumerate()
        .filter(|(_, token)| **token == query[0])
        .map(|(index, _)| index)
        .collect();
    let lasts: Vec<usize> = normalized
        .iter()
        .enumerate()
        .filter(|(_, token)| **token == query[query.len() - 1])
        .map(|(index, _)| index)
        .collect();
    let mut best: Option<(i64, i64, usize)> = None;
    for &first_index in &firsts {
        for &last_index in &lasts {
            if last_index <= first_index {
                continue;
            }
            let first = &timing.words[source_word[first_index]];
            let last = &timing.words[source_word[last_index]];
            if last.end_us <= first.start_us {
                continue;
            }
            let delta = (last_index - first_index + 1).abs_diff(query.len());
            if best
                .as_ref()
                .map(|(_, _, best_delta)| delta < *best_delta)
                .unwrap_or(true)
            {
                best = Some((
                    timeline_start_us + first.start_us,
                    timeline_start_us + last.end_us,
                    delta,
                ));
            }
        }
    }
    best.map(|(start, end, _)| (start, end))
}

fn tokens_match(query: &str, candidate: &str) -> bool {
    if query == candidate {
        return true;
    }
    if !is_numeric_token(query) {
        return false;
    }
    candidate
        .strip_suffix(query)
        .map(|tail| tail.ends_with('\''))
        .unwrap_or(false)
}

#[path = "projection/numeric.rs"]
mod numeric;
use numeric::*;

fn apply_chart_values(out: &mut SceneAnnotationsV2, raw: &RawSceneV2) {
    if !raw.charts_certified {
        out.chart_numbers.clear();
        return;
    }
    if raw.chart_values.is_empty() {
        out.chart_numbers.clear();
        return;
    }
    let allowed: Vec<f64> = raw
        .chart_values
        .iter()
        .filter_map(|value| parse_chart_value(value))
        .collect();
    if allowed.len() < 2 {
        out.chart_numbers.clear();
        return;
    }
    let mut seen = BTreeSet::new();
    for number in &out.numbers {
        let Some(parsed) = parse_chart_value(&number.text) else {
            continue;
        };
        if !allowed.iter().any(|value| same_float(*value, parsed)) {
            continue;
        }
        let key = (format!("{parsed:.12}"), number.start_us);
        if seen.insert(key) {
            out.chart_numbers.push(number.clone());
        }
    }
}

fn same_float(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-9 * 1.0_f64.max(left.abs())
}

/// Projects the scene's recipe pair and list. The legacy pair stays the first
/// element so a frozen producer that only knows `visual_recipe` keeps working;
/// when the producer already listed the same first recipe, it is not repeated.
fn apply_visual_recipes(out: &mut SceneAnnotationsV2, raw: &RawSceneV2) {
    out.visual_recipe = raw.visual_recipe.clone();
    out.visual_payload = raw.visual_payload.clone();
    out.visual_recipes = raw.visual_recipes.clone();
    if raw.visual_recipe.trim().is_empty() {
        return;
    }
    let first_matches = out.visual_recipes.first().is_some_and(|spec| {
        spec.recipe.trim() == raw.visual_recipe.trim() && spec.payload == raw.visual_payload
    });
    if !first_matches {
        out.visual_recipes.insert(
            0,
            VisualRecipeSpecV2 {
                recipe: raw.visual_recipe.clone(),
                payload: raw.visual_payload.clone(),
            },
        );
    }
}

fn apply_heavy_sentence_boost(out: &mut SceneAnnotationsV2, heavy: &[HeavySentenceV2]) {
    if out.phrases.is_empty() || heavy.is_empty() {
        return;
    }
    let normalized: Vec<(String, f64)> = heavy
        .iter()
        .filter_map(|sentence| {
            let text = normalize_text(&sentence.text);
            (!text.is_empty() && sentence.importance > 0.0).then_some((text, sentence.importance))
        })
        .collect();
    for phrase in &mut out.phrases {
        let text = normalize_text(&phrase.text);
        if text.is_empty() {
            continue;
        }
        let mut best = phrase.score;
        for (sentence, importance) in &normalized {
            if sentence.contains(&text) || text.contains(sentence) {
                best = best.max(*importance);
            }
        }
        phrase.score = best;
    }
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .map(|token| token.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn drop_disabled_categories(
    out: &mut SceneAnnotationsV2,
    scene: &SceneV2,
    canvas: &CanvasV2,
    charts_enabled: bool,
) {
    if !category_enabled(scene, canvas, "phrases") {
        out.phrases.clear();
    }
    let numbers_enabled = category_enabled(scene, canvas, "numbers");
    if (!numbers_enabled && !charts_enabled) || canvas.disable_number_overlays {
        out.numbers.clear();
    }
    if !category_enabled(scene, canvas, "images") {
        out.images.clear();
        out.products.clear();
        out.logos.clear();
    }
    if !category_enabled(scene, canvas, "maps") {
        out.maps.clear();
    }
    if !category_enabled(scene, canvas, "brand_text") {
        out.brand_texts.clear();
    }
    if !charts_enabled {
        out.chart_numbers.clear();
        out.chart_layout.clear();
    }
}

fn category_enabled(scene: &SceneV2, canvas: &CanvasV2, category: &str) -> bool {
    let scene_value = scene
        .overlay_categories
        .get(category)
        .map(|value| value.trim().to_lowercase())
        .unwrap_or_default();
    if scene_value == "disabled" {
        return false;
    }
    if scene_value == "enabled" {
        return true;
    }
    canvas
        .overlay_categories
        .get(category)
        .map(|value| !value.trim().eq_ignore_ascii_case("disabled"))
        .unwrap_or(true)
}

fn whole_scene_phrase_text(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty() || word_count(trimmed) > 16 {
        None
    } else {
        Some(trimmed)
    }
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn normalize_name(name: &str) -> String {
    name.split_whitespace()
        .map(|token| token.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn find_span(text: &str, value: &str) -> Option<(usize, usize)> {
    let value = value.trim();
    if value.is_empty() || text.is_empty() {
        return None;
    }
    if let Some(byte_index) = text.find(value) {
        let start = text[..byte_index].chars().count();
        return Some((start, start + value.chars().count()));
    }
    let wanted = value.chars().count();
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len().saturating_sub(wanted).saturating_add(1) {
        let candidate: String = chars[start..start + wanted].iter().collect();
        if candidate.to_lowercase() == value.to_lowercase() {
            return Some((start, start + wanted));
        }
    }
    None
}

fn is_place_entity_type(entity_type: &str) -> bool {
    fold_entity_kind(entity_type) == EntityKind::Place
}

/// The annotation lane one entity feeds, or `""` when the entity is only a
/// card/identity subject. The family comes from the one fold in
/// `semantic_core::entity_capabilities`; this function only names the lanes.
fn entity_kind(entity_type: &str) -> String {
    match fold_entity_kind(entity_type) {
        EntityKind::Brand => "brand_text",
        EntityKind::Place => "location",
        EntityKind::Number
        | EntityKind::Money
        | EntityKind::Percent
        | EntityKind::Date
        | EntityKind::Time => "number",
        EntityKind::Quote => "quote",
        EntityKind::Product => "product",
        EntityKind::Logo => "logo",
        _ => "",
    }
    .to_string()
}
