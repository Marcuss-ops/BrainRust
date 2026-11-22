//! Editorial/chapter scene projection: the first decision slice that runs on
//! the v2 request.
//!
//! This is the TICKET-14 WP2 building block, kept here so the v2 envelope and
//! the projection live behind one contract. It projects every grounded editorial
//! title and bullet that can render for the request language; category toggles,
//! grounding, exact word timing and scene bounds are applied, but no run-level
//! budget or card choice is made. `covered_scenes` is the chapter-bullet slot
//! ownership signal.

use std::collections::BTreeMap;

use semantic_core::text::phrase_tokens;

use super::error::EditorialPlanError;
use super::request::{SceneV2, TimingV2};
use super::response::{EditorialPlanV2Response, ScenePhraseV2};
use super::EDITORIAL_PLAN_V2_SCHEMA_VERSION;

/// Projects every grounded editorial title and bullet that can render for the
/// request's language.
pub fn project_editorial_scenes(
    request: &super::request::EditorialPlanV2Request,
) -> Result<EditorialPlanV2Response, EditorialPlanError> {
    request.validate()?;
    // The manifest only feeds scene titles and bullets. Go builds it in a
    // best-effort enrichment phase, so a request that already disabled both
    // categories must not fail because that phase produced nothing: the
    // correct projection is empty, not an error. When a scene still asks for
    // editorial cards the section stays mandatory, so the caller never loses
    // requested output silently.
    let Some(manifest) = request.editorial.as_ref() else {
        if request.scenes.iter().any(|scene| {
            category_enabled(scene, &request.canvas, "titles")
                || category_enabled(scene, &request.canvas, "bullets")
        }) {
            return Err(EditorialPlanError::MissingSection("editorial"));
        }
        return Ok(EditorialPlanV2Response {
            schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            scene_phrases: BTreeMap::new(),
            covered_scenes: Vec::new(),
        });
    };
    if !manifest.certified {
        return Err(EditorialPlanError::invalid(
            "editorial.certified",
            "manifest is not certified",
        ));
    }
    let localized = !request.source_language.trim().is_empty()
        && !request
            .source_language
            .eq_ignore_ascii_case(request.language.trim());
    let source_manifest_is_safe_for_localized_plan = localized
        && manifest
            .source_language
            .eq_ignore_ascii_case(request.source_language.trim())
        && request.scenes.iter().all(|scene| {
            ["titles", "bullets"].iter().all(|category| {
                scene
                    .overlay_categories
                    .get(*category)
                    .or_else(|| request.canvas.overlay_categories.get(*category))
                    .is_some_and(|value| value.eq_ignore_ascii_case("disabled"))
            })
        });
    if !manifest
        .source_language
        .eq_ignore_ascii_case(request.language.trim())
        && !source_manifest_is_safe_for_localized_plan
    {
        return Err(EditorialPlanError::invalid(
            "editorial.source_language",
            format!(
                "{:?} does not match the request language {:?}",
                manifest.source_language, request.language
            ),
        ));
    }
    // A localized run carries the manifest in the target language, so the
    // source-language equality only applies when the request is not localized.
    if !request.source_language.trim().is_empty()
        && !localized
        && !manifest
            .source_language
            .eq_ignore_ascii_case(request.source_language.trim())
    {
        return Err(EditorialPlanError::invalid(
            "editorial.source_language",
            format!(
                "{:?} does not match the source language {:?}",
                manifest.source_language, request.source_language
            ),
        ));
    }

    let scenes: BTreeMap<&str, &SceneV2> = request
        .scenes
        .iter()
        .map(|scene| (scene.id.as_str(), scene))
        .collect();
    let mut scene_phrases: BTreeMap<String, Vec<ScenePhraseV2>> = BTreeMap::new();
    for editorial in &manifest.scenes {
        let Some(scene) = scenes.get(editorial.scene_id.as_str()) else {
            continue;
        };
        if scene.duration_us <= 0 || scene.timeline_start_us < 0 {
            continue;
        }
        if scene
            .execution_mode
            .trim()
            .eq_ignore_ascii_case("fixed_media")
        {
            continue;
        }
        if scene.short_phrase_overlay {
            continue;
        }
        let text = scene.text.trim();
        if text.is_empty() {
            continue;
        }
        let Some(timing) = scene
            .voiceover
            .as_ref()
            .and_then(|voice| voice.timing.as_ref())
        else {
            continue;
        };
        if locate_phrase(timing, text).is_none() {
            continue;
        }
        let window_start = scene.timeline_start_us;
        let window_end = window_start.saturating_add(scene.duration_us);
        let mut phrases = Vec::new();

        if category_enabled(scene, &request.canvas, "titles")
            && editorial.title_status == "resolved"
        {
            if let Some(title) = editorial
                .title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
            {
                if valid_title_source(&editorial.title_source) && grounded(text, title) {
                    if let Some((start_us, end_us)) = locate_phrase(timing, title) {
                        if let Some(phrase) = scene_phrase(
                            title,
                            start_us,
                            end_us,
                            window_start,
                            window_end,
                            1.0,
                            "scene_title",
                            "title",
                        ) {
                            phrases.push(phrase);
                        }
                    }
                }
            }
        }

        if category_enabled(scene, &request.canvas, "bullets") {
            for bullet in &editorial.bullets {
                let bullet_text = bullet.text.trim();
                if bullet_text.is_empty()
                    || bullet.sentence_start < 0
                    || bullet.sentence_end <= bullet.sentence_start
                {
                    continue;
                }
                if !grounded(text, bullet_text) {
                    continue;
                }
                if !editorial.globally_indexed {
                    let Some(span) = editorial_sentence_span(
                        text,
                        bullet.sentence_start as usize,
                        bullet.sentence_end as usize,
                    ) else {
                        continue;
                    };
                    if !span.eq_ignore_ascii_case(bullet_text) {
                        continue;
                    }
                }
                if let Some((start_us, end_us)) = locate_phrase(timing, bullet_text) {
                    if let Some(phrase) = scene_phrase(
                        bullet_text,
                        start_us,
                        end_us,
                        window_start,
                        window_end,
                        0.99,
                        "scene_bullet",
                        "bullet",
                    ) {
                        phrases.push(phrase);
                    }
                }
            }
        }

        if !phrases.is_empty() {
            scene_phrases.insert(editorial.scene_id.clone(), phrases);
        }
    }
    Ok(EditorialPlanV2Response {
        schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
        scene_phrases,
        covered_scenes: Vec::new(),
    })
}

fn category_enabled(scene: &SceneV2, canvas: &super::request::CanvasV2, category: &str) -> bool {
    if let Some(toggle) = scene.overlay_categories.get(category) {
        if !toggle.is_empty() && toggle != "default" {
            return toggle != "disabled";
        }
    }
    canvas
        .overlay_categories
        .get(category)
        .map(|toggle| toggle != "disabled")
        .unwrap_or(true)
}

fn valid_title_source(source: &str) -> bool {
    matches!(source.trim(), "topic_validated" | "extractive_keyphrase")
}

fn grounded(scene_text: &str, phrase: &str) -> bool {
    !phrase.is_empty() && scene_text.to_lowercase().contains(&phrase.to_lowercase())
}

#[allow(clippy::too_many_arguments)]
fn scene_phrase(
    text: &str,
    start_us: i64,
    end_us: i64,
    window_start: i64,
    window_end: i64,
    score: f64,
    presentation_role: &str,
    candidate: &str,
) -> Option<ScenePhraseV2> {
    let clipped_start = window_start.saturating_add(start_us);
    let clipped_end = window_start.saturating_add(end_us);
    if clipped_end <= clipped_start || clipped_start < window_start || clipped_end > window_end {
        return None;
    }
    Some(ScenePhraseV2 {
        text: text.to_string(),
        start_us: clipped_start,
        duration_us: clipped_end - clipped_start,
        score,
        presentation_role: presentation_role.to_string(),
        candidate: candidate.to_string(),
    })
}

fn editorial_sentence_span(text: &str, start: usize, end: usize) -> Option<String> {
    if end <= start {
        return None;
    }
    let mut sentences = Vec::new();
    let mut sentence_start = 0;
    for (index, character) in text.char_indices() {
        if !matches!(character, '.' | '!' | '?' | '\n') {
            continue;
        }
        let sentence_end = index + character.len_utf8();
        let sentence = text[sentence_start..sentence_end].trim();
        if !sentence.is_empty() {
            sentences.push(sentence.to_string());
        }
        sentence_start = sentence_end;
    }
    let tail = text[sentence_start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    if end > sentences.len() {
        return None;
    }
    Some(sentences[start..end].join(" "))
}

/// Exact contiguous phrase location over the certified word timing. Tokens are
/// lowercased and split on non-alphanumeric characters on both sides, so
/// punctuation and dash-joined TTS words align without fabricating timing.
fn locate_phrase(timing: &TimingV2, phrase: &str) -> Option<(i64, i64)> {
    let query = phrase_tokens(phrase);
    if query.is_empty() {
        return None;
    }
    let mut words: Vec<(String, usize)> = Vec::new();
    for (index, word) in timing.words.iter().enumerate() {
        for token in phrase_tokens(&word.text) {
            words.push((token, index));
        }
    }
    if query.len() > words.len() {
        return None;
    }
    for start in 0..=words.len() - query.len() {
        if words[start..start + query.len()]
            .iter()
            .zip(query.iter())
            .all(|((token, _), expected)| token == expected)
        {
            let first = words[start].1;
            let last = words[start + query.len() - 1].1;
            return Some((timing.words[first].start_us, timing.words[last].end_us));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::project_editorial_scenes;
    use crate::editorial_plan_v2::EditorialPlanV2Request;

    const REQUEST: &str = r#"{
      "operation": "compile_editorial_plan",
      "schema_version": "editorial_plan.v2",
      "plan": {
        "plan_id": "plan-1", "video_id": "video-1", "project_id": "project-1",
        "motion_seed": "seed-1",
        "width": 1920, "height": 1080, "fps_num": 24, "fps_den": 1
      },
      "language": "it",
      "source_language": "it",
      "scenes": [{
        "id": "scene-1",
        "text": "Nike corre al museo.",
        "duration_ms": 2000,
        "execution_mode": "body",
        "timeline_start_us": 0,
        "duration_us": 2000000,
        "voiceover": {"timing": {"duration_us": 2000000, "words": [
          {"index": 0, "text": "Nike", "start_us": 100000, "end_us": 400000},
          {"index": 1, "text": "corre", "start_us": 400000, "end_us": 800000},
          {"index": 2, "text": "al", "start_us": 800000, "end_us": 1000000},
          {"index": 3, "text": "museo", "start_us": 1000000, "end_us": 1300000}
        ]}}
      }],
      "editorial": {
        "schema_version": "editorial.v1",
        "source_language": "it",
        "profile_version": "highlights.v1",
        "certified": true,
        "fingerprint": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "scenes": [{
          "scene_id": "scene-1",
          "title": "Nike corre",
          "title_status": "resolved",
          "title_source": "extractive_keyphrase",
          "bullets": [{"sentence_start": 0, "sentence_end": 1, "text": "Nike corre al museo."}]
        }]
      }
    }"#;

    fn request() -> EditorialPlanV2Request {
        serde_json::from_str(REQUEST).expect("fixture request")
    }

    #[test]
    fn projects_a_grounded_title_and_bullet_with_certified_timing() {
        let response = project_editorial_scenes(&request()).expect("projection");
        let phrases = response
            .scene_phrases
            .get("scene-1")
            .expect("scene phrases");
        assert_eq!(phrases.len(), 2);
        assert_eq!(phrases[0].text, "Nike corre");
        assert_eq!(phrases[0].presentation_role, "scene_title");
        assert_eq!(phrases[0].start_us, 100_000);
        assert_eq!(phrases[0].duration_us, 700_000);
        assert_eq!(phrases[1].text, "Nike corre al museo.");
        assert_eq!(phrases[1].presentation_role, "scene_bullet");
        assert_eq!(phrases[1].duration_us, 1_200_000);
    }

    #[test]
    fn drops_a_title_with_an_uncertified_source() {
        let mut request = request();
        request.editorial.as_mut().expect("editorial").scenes[0].title_source =
            "sentence".to_string();
        let response = project_editorial_scenes(&request).expect("projection");
        let phrases = response
            .scene_phrases
            .get("scene-1")
            .expect("scene phrases");
        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases[0].presentation_role, "scene_bullet");
    }

    #[test]
    fn honours_a_scene_level_category_toggle() {
        let mut request = request();
        request.scenes[0]
            .overlay_categories
            .insert("titles".to_string(), "disabled".to_string());
        let response = project_editorial_scenes(&request).expect("projection");
        let phrases = response
            .scene_phrases
            .get("scene-1")
            .expect("scene phrases");
        assert!(
            phrases
                .iter()
                .all(|phrase| phrase.presentation_role != "scene_title"),
            "a scene-level titles toggle must suppress the title"
        );
    }

    #[test]
    fn omits_editorial_cards_without_a_manifest_when_both_categories_are_disabled() {
        let mut request = request();
        request.editorial = None;
        request.scenes[0]
            .overlay_categories
            .insert("titles".to_string(), "disabled".to_string());
        request.scenes[0]
            .overlay_categories
            .insert("bullets".to_string(), "disabled".to_string());
        let response = project_editorial_scenes(&request).expect("empty projection");
        assert!(response.scene_phrases.is_empty());
        assert!(response.covered_scenes.is_empty());
    }

    #[test]
    fn rejects_a_missing_manifest_when_titles_are_still_enabled() {
        let mut request = request();
        request.editorial = None;
        let error = project_editorial_scenes(&request).expect_err("missing editorial");
        assert_eq!(
            error,
            super::EditorialPlanError::MissingSection("editorial")
        );
    }

    #[test]
    fn rejects_an_uncertified_manifest() {
        let mut request = request();
        request.editorial.as_mut().expect("editorial").certified = false;
        let error = project_editorial_scenes(&request).expect_err("uncertified");
        assert!(error.to_string().contains("not certified"));
    }

    #[test]
    fn rejects_a_language_mismatch() {
        let mut request = request();
        request.language = "en".to_string();
        let error = project_editorial_scenes(&request).expect_err("language");
        assert!(error
            .to_string()
            .contains("does not match the request language"));
    }

    #[test]
    fn localized_plan_allows_source_manifest_when_editorial_cards_are_disabled() {
        let mut request = request();
        request.language = "en".to_string();
        request.scenes[0]
            .overlay_categories
            .insert("titles".to_string(), "disabled".to_string());
        request.scenes[0]
            .overlay_categories
            .insert("bullets".to_string(), "disabled".to_string());
        let response = project_editorial_scenes(&request).expect("localized projection");
        assert!(!response.scene_phrases.contains_key("scene-1"));
    }
}
