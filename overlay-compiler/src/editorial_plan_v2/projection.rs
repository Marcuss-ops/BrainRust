//! WP4 raw-surface projector.
//!
//! The producer supplies grounded phrases, entities, occurrences, chart
//! authorizations and the numeric vocabulary. Every operation that chooses,
//! drops, times or retimes a surface happens here, so the Go request builder
//! cannot be a second planner authority.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
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
        let mut projected = project_scene(
            scene,
            &raw,
            &canvas,
            &lexicon,
            &heavy,
            &language,
            &source_language,
            entity_timeline.as_ref(),
        );
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

/// Allocates the producer-verified scene images across the run ceiling.
///
/// Go supplies only the verified candidate sets and the excluded entity
/// hashes; it must not choose which scene wins an image slot. One candidate
/// per scene is selected before any scene gets a second slot, matching the
/// historical round-robin policy without letting the first scene consume the
/// whole ceiling.
fn allocate_scene_images(
    media: Option<&MediaV2>,
    plan: Option<&ImagePlanV2>,
) -> std::collections::BTreeMap<String, Vec<ImageCandidateV2>> {
    let mut out = std::collections::BTreeMap::new();
    let (Some(media), Some(plan)) = (media, plan) else {
        return out;
    };
    if !plan.enabled || plan.ceiling <= 0 {
        return out;
    }
    let mut used: BTreeSet<String> = plan
        .excluded_sha256
        .iter()
        .map(|hash| hash.trim().to_lowercase())
        .filter(|hash| !hash.is_empty())
        .collect();
    let sets: Vec<&super::request::SceneImageCandidateSetV2> = media
        .scene_image_candidates
        .iter()
        .filter(|set| !set.candidates.is_empty())
        .collect();
    if sets.is_empty() {
        return out;
    }
    let mut cursors = vec![0usize; sets.len()];
    let mut count = 0i64;
    while count < plan.ceiling {
        let mut progressed = false;
        for index in 0..sets.len() {
            while cursors[index] < sets[index].candidates.len() {
                let candidate = sets[index].candidates[cursors[index]].clone();
                cursors[index] += 1;
                let hash = candidate.sha256.trim().to_lowercase();
                if hash.is_empty() || used.contains(&hash) {
                    continue;
                }
                used.insert(hash);
                out.entry(sets[index].scene_id.clone())
                    .or_insert_with(Vec::new)
                    .push(candidate);
                count += 1;
                progressed = true;
                break;
            }
            if count >= plan.ceiling {
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}
/// Projects the scene-level recap sources. Placement, score order, dedupe and
/// card timing stay in `build_recap_candidates`; the producer must not supply
/// a pre-selected chain.
fn project_recap_briefs(request: &mut EditorialPlanV2Request) {
    if request.canvas.disable_recap_overlays {
        return;
    }
    request.recap_briefs.clear();
    for scene in &request.scenes {
        if scene.duration_us <= 0 {
            continue;
        }
        let (title, score) = scene
            .annotations
            .as_ref()
            .and_then(|annotations| {
                annotations
                    .phrases
                    .iter()
                    .max_by(|left, right| left.score.total_cmp(&right.score))
                    .map(|phrase| (phrase.text.trim().to_string(), phrase.score))
            })
            .filter(|(title, _)| !title.is_empty())
            .unwrap_or_else(|| (first_sentence(&scene.text), 0.8));
        if title.is_empty() {
            continue;
        }
        request.recap_briefs.push(RecapBriefV2 {
            id: scene.id.clone(),
            title,
            score,
            start_us: scene.timeline_start_us,
            end_us: scene.timeline_start_us.saturating_add(scene.duration_us),
        });
    }
}

fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let end = trimmed.find(['.', '!', '?', '\n']).unwrap_or(trimmed.len());
    trimmed[..end].trim().to_string()
}

#[allow(clippy::too_many_arguments)]
fn project_scene(
    scene: &SceneV2,
    raw: &RawSceneV2,
    canvas: &CanvasV2,
    lexicon: &NumericLexiconV2,
    heavy: &[HeavySentenceV2],
    language: &str,
    source_language: &str,
    timeline: Option<&EntityTimelineV2>,
) -> SceneAnnotationsV2 {
    let mut out = SceneAnnotationsV2 {
        end_us: scene.timeline_start_us.saturating_add(scene.duration_us),
        ..SceneAnnotationsV2::default()
    };
    let timing = scene
        .voiceover
        .as_ref()
        .and_then(|voiceover| voiceover.timing.as_ref())
        .filter(|timing| !timing.words.is_empty());
    let has_timing = timing.is_some();
    let phrases_enabled = category_enabled(scene, canvas, "phrases");
    let maps_enabled = category_enabled(scene, canvas, "maps");
    let numbers_enabled = category_enabled(scene, canvas, "numbers");
    let charts_enabled = canvas.enable_charts && category_enabled(scene, canvas, "charts");

    if let Some(timing) = timing {
        for phrase in &raw.important_phrases {
            let text = phrase.text.trim();
            // A two-word phrase is a complete grounded callout; a single word
            // is too fragmentary for this lane.
            if word_count(text) < 2 {
                continue;
            }
            if let Some((start_us, end_us)) = locate_phrase(timing, scene.timeline_start_us, text) {
                out.phrases
                    .push(timed(text, "", start_us, end_us, phrase.score));
            }
        }
    }

    for entity in raw
        .primary_entities
        .iter()
        .chain(raw.secondary_entities.iter())
    {
        project_entity(
            &mut out,
            scene,
            entity,
            timing,
            timeline,
            lexicon,
            language,
            source_language,
            &raw.source_numeric_values,
        );
    }

    if numbers_enabled {
        out.numbers = dedupe_grounded_dates(out.numbers);
        for date in explicit_date_surfaces(&scene.text) {
            let mut duplicate = false;
            for index in 0..out.numbers.len() {
                if out.numbers[index].text.eq_ignore_ascii_case(&date) {
                    duplicate = true;
                    break;
                }
                if same_grounded_date(&out.numbers[index].text, &date) {
                    if date_has_year(&date) && !date_has_year(&out.numbers[index].text) {
                        if let Some((start_us, end_us)) = timing.and_then(|timing| {
                            locate_phrase(timing, scene.timeline_start_us, &date)
                        }) {
                            let replacement = timed(
                                &date,
                                "DATE",
                                start_us,
                                end_us,
                                score_for_date(out.numbers[index].score),
                            );
                            out.numbers[index] = replacement;
                        }
                    }
                    duplicate = true;
                    break;
                }
            }
            if duplicate {
                continue;
            }
            let Some((start_us, end_us)) =
                timing.and_then(|timing| locate_phrase(timing, scene.timeline_start_us, &date))
            else {
                continue;
            };
            out.numbers
                .push(timed(&date, "DATE", start_us, end_us, 0.8));
        }
        out.numbers = dedupe_grounded_metric_numbers(out.numbers);
    }

    // The no-word-timing fallback turns a short scene into one phrase card,
    // but only inside the same 16-word renderability ceiling as the long lane.
    let phrase_window = scene.duration_ms > 0 && phrases_enabled;
    if scene.short_phrase_overlay || (phrase_window && !has_timing) {
        if let Some(text) = whole_scene_phrase_text(&scene.text) {
            out.phrases = vec![TimedAnnotationV2 {
                text: text.to_string(),
                annotation_type: String::new(),
                start_ms: scene.timeline_start_us / 1000,
                end_ms: (scene.timeline_start_us + scene.duration_us + 999) / 1000,
                start_us: scene.timeline_start_us,
                duration_us: scene.duration_us,
                score: 1.0,
                presentation_role: String::new(),
                editorial_candidate: String::new(),
            }];
        }
    }

    // Short/map-only scenes may have no certified word timing. Grounded place
    // names still produce map candidates on the canonical whole-scene window;
    // an unbacked plate or an absent source span remains a no-op.
    if maps_enabled && scene.duration_ms > 0 && !has_timing {
        for entity in raw
            .primary_entities
            .iter()
            .chain(raw.secondary_entities.iter())
        {
            let Some(geo) = entity.geo.as_ref() else {
                continue;
            };
            if !is_place_entity_type(&entity.entity_type)
                || !entity.plate_covered
                || find_span(&scene.text, &entity.canonical_name).is_none()
            {
                continue;
            }
            if let Some(candidate) = map_candidate(
                &entity.stable_entity_id,
                &entity.canonical_name,
                geo,
                scene.timeline_start_us,
                scene.duration_us,
                entity.confidence,
            ) {
                out.maps.push(candidate);
            }
        }
    }

    apply_chart_values(&mut out, raw);
    out.chart_layout = raw.chart_layout.clone();
    apply_visual_recipes(&mut out, raw);
    apply_heavy_sentence_boost(&mut out, heavy);
    drop_disabled_categories(&mut out, scene, canvas, charts_enabled);
    out
}

#[allow(clippy::too_many_arguments)]
fn project_entity(
    out: &mut SceneAnnotationsV2,
    scene: &SceneV2,
    entity: &RawEntityV2,
    timing: Option<&super::request::TimingV2>,
    timeline: Option<&EntityTimelineV2>,
    lexicon: &NumericLexiconV2,
    language: &str,
    source_language: &str,
    source_numeric_values: &[String],
) {
    let kind = entity_kind(&entity.entity_type);
    let score = if entity.confidence > 0.0 {
        entity.confidence
    } else {
        0.9
    };
    if kind == "brand_text" {
        if let Some((start_us, end_us)) = timing.and_then(|timing| {
            locate_phrase(timing, scene.timeline_start_us, &entity.canonical_name)
        }) {
            out.brand_texts.push(timed(
                &entity.canonical_name,
                &entity.entity_type,
                start_us,
                end_us,
                score,
            ));
        }
        return;
    }
    let Some(occurrence) = occurrence_for(timeline, scene, entity) else {
        return;
    };
    match kind.as_str() {
        "number" => {
            let mut value = clean_numeric_value(&entity.canonical_name, lexicon);
            if value.is_empty() {
                return;
            }
            value = recover_numeric_suffix(&scene.text, &value, &entity.mentions);
            if value.is_empty()
                || !numeric_entity_grounded(
                    language,
                    source_language,
                    &value,
                    &entity.canonical_name,
                    source_numeric_values,
                )
            {
                return;
            }
            let (start_us, end_us) = if value != entity.canonical_name {
                timing
                    .and_then(|timing| locate_phrase(timing, scene.timeline_start_us, &value))
                    .unwrap_or((occurrence.audio_start_us, occurrence.audio_end_us))
            } else {
                (occurrence.audio_start_us, occurrence.audio_end_us)
            };
            out.numbers
                .push(timed(&value, &entity.entity_type, start_us, end_us, score));
        }
        "quote" => out.quotes.push(timed(
            &entity.canonical_name,
            &entity.entity_type,
            occurrence.audio_start_us,
            occurrence.audio_end_us,
            score,
        )),
        "product" => {
            if let Some(image) = entity.image.as_ref() {
                if let Some(candidate) = image_candidate(image, occurrence, score) {
                    out.products.push(candidate);
                }
            }
        }
        "logo" => {
            if let Some(image) = entity.image.as_ref() {
                if let Some(candidate) = image_candidate(image, occurrence, score) {
                    out.logos.push(candidate);
                }
            } else if let Some((start_us, end_us)) = timing.and_then(|timing| {
                locate_phrase(timing, scene.timeline_start_us, &entity.canonical_name)
            }) {
                out.brand_texts.push(timed(
                    &entity.canonical_name,
                    &entity.entity_type,
                    start_us,
                    end_us,
                    score,
                ));
            }
        }
        "location" => {
            let Some(geo) = entity.geo.as_ref() else {
                return;
            };
            if !entity.plate_covered {
                return;
            }
            if let Some(candidate) = map_candidate(
                &occurrence.entity_id,
                &entity.canonical_name,
                geo,
                occurrence.audio_start_us,
                occurrence.audio_end_us - occurrence.audio_start_us,
                score,
            ) {
                out.maps.push(candidate);
            }
        }
        _ => {}
    }
}

fn occurrence_for<'a>(
    timeline: Option<&'a EntityTimelineV2>,
    scene: &SceneV2,
    entity: &RawEntityV2,
) -> Option<&'a super::request::EntityOccurrenceV2> {
    let scene_timeline = timeline?
        .scenes
        .iter()
        .find(|item| item.scene_id == scene.id)?;
    if !entity.stable_entity_id.is_empty() {
        if let Some(found) = scene_timeline
            .entities
            .iter()
            .find(|occurrence| occurrence.entity_id == entity.stable_entity_id)
        {
            return Some(found);
        }
    }
    if !is_place_entity_type(&entity.entity_type) {
        return None;
    }
    let wanted = normalize_name(&entity.canonical_name);
    scene_timeline.entities.iter().find(|occurrence| {
        is_place_entity_type(&occurrence.entity_type) && normalize_name(&occurrence.name) == wanted
    })
}

fn image_candidate(
    image: &super::request::RawEntityImageV2,
    occurrence: &super::request::EntityOccurrenceV2,
    score: f64,
) -> Option<ImageCandidateV2> {
    if image.asset_id.trim().is_empty() && image.url.trim().is_empty() {
        return None;
    }
    let start_us = (occurrence.audio_start_us / 1000) * 1000;
    let duration_us = spoken_window_duration(occurrence.audio_start_us, occurrence.audio_end_us);
    Some(ImageCandidateV2 {
        asset_id: image.asset_id.clone(),
        url: image.url.clone(),
        sha256: image.sha256.clone(),
        media_type: image.media_type.clone(),
        start_ms: start_us / 1000,
        end_ms: (start_us + duration_us + 999) / 1000,
        start_us,
        duration_us,
        score,
    })
}

fn spoken_window_duration(audio_start_us: i64, audio_end_us: i64) -> i64 {
    let spoken = (audio_end_us - audio_start_us).max(0);
    let duration = (spoken + 500_000).clamp(1_000_000, 5_000_000);
    ((duration + 999) / 1000) * 1000
}

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
    fold_entity_type(entity_type) == "PLACE"
}

fn entity_kind(entity_type: &str) -> String {
    if entity_type.trim().eq_ignore_ascii_case("BRAND") {
        return "brand_text".to_string();
    }
    match fold_entity_type(entity_type).as_str() {
        "PLACE" => "location",
        "NUMBER" | "MONEY" | "PERCENT" | "DATE" | "TIME" => "number",
        "QUOTE" => "quote",
        "PRODUCT" => "product",
        "LOGO" => "logo",
        _ => "",
    }
    .to_string()
}

fn fold_entity_type(entity_type: &str) -> String {
    let normalized = entity_type.trim().to_uppercase();
    match normalized.as_str() {
        "PER" | "HUMAN" | "CEO" | "FOUNDER" | "PRESIDENT" => "PERSON",
        "ORGANIZATION" | "ORGANISATION" | "COMPANY" | "CORP" | "CORPORATION" | "BUSINESS"
        | "INSTITUTION" | "AGENCY" | "COURT" | "NGO" | "GOVERNMENT" | "UNIVERSITY" | "TEAM" => {
            "ORG"
        }
        "GPE" | "LOC" | "LOCATION" | "CITY" | "COUNTRY" | "NATION" | "REGION" | "STATE"
        | "PROVINCE" | "TOWN" | "VILLAGE" | "LANDMARK" | "SITE" | "GEO" | "ADDRESS" | "FAC"
        | "FACILITY" => "PLACE",
        "BRAND" | "LOGO" => "LOGO",
        "NUM" | "CARDINAL" | "ORDINAL" | "QUANTITY" | "METRIC" | "METRICS" | "STATISTIC"
        | "STATISTICS" | "COUNT" => "NUMBER",
        "CURRENCY" | "MONEY" => "MONEY",
        "PERCENTAGE" => "PERCENT",
        "DATETIME" | "CALENDAR_DATE" => "DATE",
        other => other,
    }
    .to_string()
}
