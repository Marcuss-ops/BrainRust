//! WP4 raw-surface projector.
//!
//! The producer supplies grounded phrases, entities, occurrences, chart
//! authorizations and the numeric vocabulary. Every operation that chooses,
//! drops, times or retimes a surface happens here, so the Go request builder
//! cannot be a second planner authority.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use super::request::{
    CanvasV2, EditorialPlanV2Request, EntityTimelineV2, HeavySentenceV2, ImageCandidateV2,
    ImagePlanV2, MediaV2, NumericLexiconV2, RawEntityV2, RawGeoV2, RawMentionV2, RawSceneV2,
    RecapBriefV2, SceneAnnotationsV2, SceneV2, TimedAnnotationV2,
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

fn phrase_tokens(phrase: &str) -> Vec<String> {
    phrase
        .split_whitespace()
        .flat_map(fragment_tokens)
        .collect()
}

fn fragment_tokens(field: &str) -> Vec<String> {
    field
        .split(is_dash_separator)
        .filter_map(|fragment| {
            let token = normalize_token(fragment);
            (!token.is_empty()).then_some(token)
        })
        .collect()
}

fn normalize_token(token: &str) -> String {
    let folded: String = token.nfc().collect::<String>().to_lowercase();
    let folded: String = folded
        .chars()
        .map(|ch| match ch {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{02BC}' => '\'',
            other => other,
        })
        .collect();
    folded
        .trim_matches(|ch: char| ch.is_ascii_punctuation() && ch != '\'')
        .trim_end_matches('\'')
        .trim()
        .to_string()
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

fn is_numeric_token(token: &str) -> bool {
    let mut seen_dot = false;
    let mut seen_digit = false;
    for ch in token.chars() {
        match ch {
            '0'..='9' => seen_digit = true,
            '.' | ',' if !seen_dot => seen_dot = true,
            _ => return false,
        }
    }
    seen_digit
}

fn is_dash_separator(ch: char) -> bool {
    matches!(
        ch,
        '-' | '\u{2010}'
            | '\u{2011}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{2015}'
            | '\u{2212}'
    )
}

fn clean_numeric_value(value: &str, lexicon: &NumericLexiconV2) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if is_numeric_value(value, lexicon) {
        return value.to_string();
    }
    let mut trimmed = value.to_string();
    while !is_numeric_value(&trimmed, lexicon) {
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() < 2 {
            break;
        }
        trimmed = fields[..fields.len() - 1].join(" ");
    }
    if is_numeric_value(&trimmed, lexicon) {
        return trimmed;
    }
    let mut parts: Vec<&str> = value
        .split(|ch: char| ch.is_whitespace() || ch == '\'' || ch == '\u{2019}')
        .filter(|part| !part.is_empty())
        .collect();
    while parts.len() > 1 {
        parts.remove(0);
        let candidate = parts.join(" ");
        if is_numeric_value(&candidate, lexicon) && starts_with_number(&candidate, lexicon) {
            return candidate;
        }
    }
    if parts.len() == 1
        && is_numeric_value(parts[0], lexicon)
        && starts_with_number(parts[0], lexicon)
    {
        return parts[0].to_string();
    }
    String::new()
}

fn is_numeric_value(value: &str, lexicon: &NumericLexiconV2) -> bool {
    let words: BTreeSet<String> = lexicon
        .words
        .iter()
        .map(|word| word.trim().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    let context: BTreeSet<String> = lexicon
        .context
        .iter()
        .map(|word| word.trim().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    let mut token = String::new();
    let mut saw_number = false;
    let mut valid = true;
    let flush = |token: &mut String, saw_number: &mut bool, valid: &mut bool| {
        if token.is_empty() {
            return;
        }
        let word = token.to_lowercase();
        token.clear();
        if words.contains(&word) {
            *saw_number = true;
        } else if !context.contains(&word) {
            *valid = false;
        }
    };
    for ch in value.chars() {
        if ch.is_numeric() {
            saw_number = true;
        } else if ch.is_alphabetic() {
            token.push(ch);
        } else {
            flush(&mut token, &mut saw_number, &mut valid);
        }
    }
    flush(&mut token, &mut saw_number, &mut valid);
    valid && saw_number
}

fn starts_with_number(value: &str, lexicon: &NumericLexiconV2) -> bool {
    let Some(first) = value.split_whitespace().next() else {
        return false;
    };
    let first = first.trim_matches(|ch: char| {
        matches!(
            ch,
            '\'' | '\u{2019}' | '.' | '-' | ',' | '%' | '€' | '$' | '£' | '¥'
        )
    });
    if first.is_empty() {
        return false;
    }
    if first.chars().any(|ch| ch.is_numeric()) {
        return true;
    }
    if first.chars().any(|ch| !ch.is_alphabetic()) {
        return false;
    }
    lexicon
        .words
        .iter()
        .any(|word| word.eq_ignore_ascii_case(first))
}

fn recover_numeric_suffix(text: &str, value: &str, mentions: &[RawMentionV2]) -> String {
    let Some(last) = value.chars().last() else {
        return value.to_string();
    };
    if !last.is_numeric() {
        return value.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for mention in mentions {
        let start = mention.start_rune.max(0) as usize;
        let end = mention.end_rune.max(0) as usize;
        if start >= end || end > chars.len() {
            continue;
        }
        let segment: String = chars[start..end].iter().collect();
        let Some(byte_at) = segment.find(value) else {
            continue;
        };
        let rune_at = segment[..byte_at].chars().count();
        let absolute_end = start + rune_at + value.chars().count();
        if absolute_end >= chars.len() {
            return value.to_string();
        }
        if matches!(chars[absolute_end], '%' | '€' | '$' | '£' | '¥') {
            return format!("{value}{}", chars[absolute_end]);
        }
        return value.to_string();
    }
    value.to_string()
}

fn numeric_entity_grounded(
    language: &str,
    source_language: &str,
    value: &str,
    _canonical_name: &str,
    source_values: &[String],
) -> bool {
    if language.eq_ignore_ascii_case(source_language) {
        return true;
    }
    let allowed: BTreeSet<i64> = source_values
        .iter()
        .flat_map(|value| numeric_values(value))
        .collect();
    let values = numeric_values(value);
    !values.is_empty() && values.iter().all(|number| allowed.contains(number))
}

fn numeric_values(value: &str) -> Vec<i64> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if ch.is_alphabetic() || ch.is_numeric() {
            current.push(ch.to_lowercase().next().unwrap_or(ch));
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    let mut values = Vec::new();
    let mut running = 0i64;
    let flush = |values: &mut Vec<i64>, running: &mut i64| {
        if *running != 0 {
            values.push(*running);
            *running = 0;
        }
    };
    for token in tokens {
        if let Ok(number) = token.parse::<i64>() {
            flush(&mut values, &mut running);
            values.push(number);
            continue;
        }
        if matches!(token.as_str(), "and" | "e" | "y" | "de" | "del") {
            continue;
        }
        let Some(number) = numeric_word(&token) else {
            flush(&mut values, &mut running);
            continue;
        };
        if number >= 20 && number % 10 == 0 {
            running += number;
        } else if number == 100 {
            if running == 0 {
                running = 100;
            } else {
                running *= 100;
            }
        } else {
            running += number;
        }
    }
    flush(&mut values, &mut running);
    values
}

fn numeric_word(token: &str) -> Option<i64> {
    Some(match token {
        "zero" | "cero" => 0,
        "uno" | "una" | "one" | "um" | "uma" | "un" => 1,
        "two" | "dos" | "due" | "dois" | "duas" => 2,
        "three" | "tres" | "tre" | "três" => 3,
        "four" | "cuatro" | "quattro" | "quatro" => 4,
        "five" | "cinco" | "cinque" => 5,
        "six" | "seis" | "sei" => 6,
        "seven" | "siete" | "sette" => 7,
        "eight" | "ocho" | "otto" | "oito" => 8,
        "nine" | "nueve" | "nove" => 9,
        "ten" | "diez" | "dieci" | "dez" => 10,
        "eleven" | "once" | "undici" | "onze" => 11,
        "twelve" | "doce" | "dodici" | "doze" => 12,
        "thirteen" | "trece" | "tredici" | "treze" => 13,
        "fourteen" | "catorce" | "quattordici" | "quatorze" => 14,
        "fifteen" | "quince" | "quindici" | "quinze" => 15,
        "sixteen" | "dieciséis" | "sedici" | "dezesseis" => 16,
        "seventeen" | "diciassette" => 17,
        "eighteen" | "diciotto" => 18,
        "nineteen" | "diciannove" => 19,
        "twenty" | "veinte" | "venti" | "vinte" => 20,
        "thirty" | "treinta" | "trenta" | "trinta" => 30,
        "forty" | "cuarenta" | "quaranta" => 40,
        "fifty" | "cincuenta" | "cinquanta" => 50,
        "sixty" | "sesenta" | "sessanta" => 60,
        "seventy" | "setenta" | "settanta" => 70,
        "eighty" | "ochenta" | "ottanta" => 80,
        "ninety" | "noventa" | "novanta" => 90,
        "hundred" | "cien" | "ciento" | "cento" | "cem" => 100,
        _ => return None,
    })
}

fn date_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\b(?:[0-9]{1,2}\s+(?:gennaio|febbraio|marzo|aprile|maggio|giugno|luglio|agosto|settembre|ottobre|novembre|dicembre)\s+[0-9]{4}|[0-9]{4})\b")
            .expect("date regex")
    })
}

fn date_identity_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)^([0-9]{1,2})\s+(gennaio|febbraio|marzo|aprile|maggio|giugno|luglio|agosto|settembre|ottobre|novembre|dicembre)(?:\s+([0-9]{4}))?$")
            .expect("date identity regex")
    })
}

fn explicit_date_surfaces(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for matched in date_pattern().find_iter(text) {
        let value = matched.as_str().trim();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.to_lowercase()) {
            out.push(value.to_string());
        }
    }
    out
}

fn same_grounded_date(left: &str, right: &str) -> bool {
    let (Some(left), Some(right)) = (
        date_identity_pattern().captures(left.trim()),
        date_identity_pattern().captures(right.trim()),
    ) else {
        return false;
    };
    if left.get(1).map(|value| value.as_str()) != right.get(1).map(|value| value.as_str())
        || !left
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or("")
            .eq_ignore_ascii_case(right.get(2).map(|value| value.as_str()).unwrap_or(""))
    {
        return false;
    }
    let left_year = left.get(3).map(|value| value.as_str()).unwrap_or("");
    let right_year = right.get(3).map(|value| value.as_str()).unwrap_or("");
    left_year.is_empty() || right_year.is_empty() || left_year == right_year
}

fn date_has_year(text: &str) -> bool {
    let fields: Vec<&str> = text.split_whitespace().collect();
    fields.len() == 3 && fields[2].len() == 4
}

fn score_for_date(score: f64) -> f64 {
    if score <= 0.0 {
        0.8
    } else {
        score
    }
}

fn dedupe_grounded_dates(numbers: Vec<TimedAnnotationV2>) -> Vec<TimedAnnotationV2> {
    let mut dropped = vec![false; numbers.len()];
    for left_index in 0..numbers.len() {
        if !numbers[left_index]
            .annotation_type
            .eq_ignore_ascii_case("DATE")
        {
            continue;
        }
        let Some(left) = date_tokens(&numbers[left_index].text) else {
            continue;
        };
        let left_start = if numbers[left_index].start_us > 0 {
            numbers[left_index].start_us
        } else {
            numbers[left_index].start_ms * 1000
        };
        let left_end =
            if numbers[left_index].start_us + numbers[left_index].duration_us > left_start {
                numbers[left_index].start_us + numbers[left_index].duration_us
            } else {
                numbers[left_index].end_ms * 1000
            };
        for right_index in 0..numbers.len() {
            if left_index == right_index
                || dropped[right_index]
                || !numbers[right_index]
                    .annotation_type
                    .eq_ignore_ascii_case("DATE")
            {
                continue;
            }
            let Some(right) = date_tokens(&numbers[right_index].text) else {
                continue;
            };
            if !(subset(&left, &right) || subset(&right, &left)) {
                continue;
            }
            let right_start = if numbers[right_index].start_us > 0 {
                numbers[right_index].start_us
            } else {
                numbers[right_index].start_ms * 1000
            };
            let right_end =
                if numbers[right_index].start_us + numbers[right_index].duration_us > right_start {
                    numbers[right_index].start_us + numbers[right_index].duration_us
                } else {
                    numbers[right_index].end_ms * 1000
                };
            let overlap = left_end.min(right_end) - left_start.max(right_start);
            let shorter = (left_end - left_start).min(right_end - right_start);
            if overlap <= 0 || shorter <= 0 || overlap * 2 < shorter {
                continue;
            }
            if right.len() > left.len()
                || (right.len() == left.len()
                    && numbers[right_index].text.len() > numbers[left_index].text.len())
            {
                dropped[left_index] = true;
                break;
            }
            dropped[right_index] = true;
        }
    }
    numbers
        .into_iter()
        .enumerate()
        .filter_map(|(index, number)| (!dropped[index]).then_some(number))
        .collect()
}

fn date_tokens(text: &str) -> Option<BTreeSet<String>> {
    let months = [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ];
    let tokens: Vec<String> = text
        .split_whitespace()
        .map(|token| token.to_lowercase())
        .collect();
    if tokens.is_empty() {
        return None;
    }
    let mut out = BTreeSet::new();
    for token in tokens {
        if months.contains(&token.as_str()) {
            out.insert(token);
            continue;
        }
        if token.len() != 4 && token.len() > 2 {
            return None;
        }
        if !token.chars().all(|ch| ch.is_numeric()) {
            return None;
        }
        out.insert(token);
    }
    Some(out)
}

fn subset(left: &BTreeSet<String>, right: &BTreeSet<String>) -> bool {
    !left.is_empty() && left.len() <= right.len() && left.iter().all(|token| right.contains(token))
}

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

fn parse_chart_value(text: &str) -> Option<f64> {
    let mut value = text.trim();
    if value.is_empty() {
        return None;
    }
    value = value.trim_matches(|ch| matches!(ch, '%' | '€' | '$' | '£' | '¥'));
    let compact = value.trim().replace(' ', "");
    if compact.is_empty() {
        return None;
    }
    let numeric = if compact.matches(',').count() == 1 && !compact.contains('.') {
        compact.replace(',', ".")
    } else {
        compact.replace(',', "")
    };
    if numeric.is_empty() || matches!(numeric.as_str(), "-" | "." | "-.") {
        return None;
    }
    let mut digits = 0;
    let mut separator = false;
    for (index, ch) in numeric.char_indices() {
        match ch {
            '0'..='9' => digits += 1,
            '.' if !separator && index > 0 => separator = true,
            '-' if index == 0 => {}
            _ => return None,
        }
    }
    if digits == 0 {
        return None;
    }
    let parsed: f64 = numeric.parse().ok()?;
    (!parsed.is_nan() && !parsed.is_infinite()).then_some(parsed)
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
