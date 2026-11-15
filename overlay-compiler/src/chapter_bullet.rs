//! Chapter-bullet projection for the `editorial_plan.v2` envelope.
//!
//! This is the first executable phase of the 13d cutover: the rules that used
//! to live in `refactored/internal/capabilities/scripts/chapter_bullet_overlay.go`
//! are now owned here. The projection is extractive and fail-closed:
//!
//! - a bullet must ground verbatim in exactly the owning body scene;
//! - one bullet per scene, each manifest bullet rendered at most once;
//! - fixed-media scenes are skipped;
//! - no voiceover is required: timing comes from the canonical scene window,
//!   anchored at the first phrase when one exists;
//! - a certified spoken span wins over the fixed readable beat.

use std::collections::{BTreeMap, BTreeSet};

use unicode_normalization::UnicodeNormalization;

use crate::editorial_plan_v2::{
    ChapterBulletV2, EditorialPlanV2Request, EditorialPlanV2Response, ScenePhraseV2, SceneV2,
    TimingV2, CHAPTER_MANIFEST_SCHEMA_VERSION, EDITORIAL_PLAN_V2_SCHEMA_VERSION,
};

/// Wire value of the chapter-card presentation role.
pub const CHAPTER_BULLET_ROLE: &str = "chapter_bullet";
/// Ranks below the editorial title (1.0) and bullet (0.99).
pub const CHAPTER_BULLET_SCORE: f64 = 0.98;
/// Fixed readable beat when no certified spoken span exists.
pub const CHAPTER_BULLET_WINDOW_US: i64 = 4_000_000;
/// The extractor floor: a two-word tail is a fragment, not a point.
const MIN_BULLET_WORDS: usize = 4;
const MIN_BULLET_CHARS: usize = 16;
/// The grounding confirmation uses the first five words: splitter-independent.
const LEAD_WORDS: usize = 5;

/// Projects chapter bullets onto the v2 scenes. The caller appends the returned
/// phrases to its complete `SceneInput` DTOs and skips editorial bullets on the
/// covered scenes.
pub fn project_chapter_bullets(request: &EditorialPlanV2Request) -> EditorialPlanV2Response {
    let mut response = EditorialPlanV2Response {
        schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
        scene_phrases: BTreeMap::new(),
        covered_scenes: Vec::new(),
    };
    if request.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
        return response;
    }
    let Some(chapters) = &request.chapters else {
        return response;
    };
    if chapters.schema_version != CHAPTER_MANIFEST_SCHEMA_VERSION {
        return response;
    }
    if !request.source_language.trim().is_empty()
        && !request
            .source_language
            .eq_ignore_ascii_case(&request.language)
    {
        return response;
    }

    // Sentence offsets map transcript-global bullet spans onto scenes. The
    // transcript is the ordered join of body scenes, so offsets are cumulative
    // sentence counts over the same scenes. Splitter drift is tolerated: the
    // offsets only nominate a scene, verbatim containment always confirms it.
    let mut offsets: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    let mut cursor = 0_i64;
    for scene in &request.scenes {
        if is_fixed_media(&scene.execution_mode) {
            continue;
        }
        let count = editorial_sentences(scene.text.trim()).len() as i64;
        offsets.insert(scene.id.as_str(), (cursor, cursor + count));
        cursor += count;
    }

    let mut ordered: Vec<&ChapterBulletV2> = Vec::new();
    for chapter in &chapters.chapters {
        for bullet in &chapter.bullets {
            if !bullet.text.trim().is_empty() {
                ordered.push(bullet);
            }
        }
    }

    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut covered: BTreeSet<String> = BTreeSet::new();
    let global_categories = &request.canvas.overlay_categories;
    for scene in &request.scenes {
        if covered.contains(&scene.id) {
            continue;
        }
        if category_disabled(scene, "bullets", global_categories) {
            continue;
        }
        if scene.short_phrase_overlay || is_fixed_media(&scene.execution_mode) {
            continue;
        }
        let text = scene.text.trim();
        if text.is_empty() {
            continue;
        }
        let scene_start_us = scene.timeline_start_us;
        let scene_end_us = scene_start_us.saturating_add(scene.duration_us);
        if scene_end_us <= scene_start_us {
            continue;
        }
        let scene_range = offsets.get(scene.id.as_str()).copied();
        for bullet in &ordered {
            let bullet_text = bullet.text.trim();
            let key = bullet_text.to_lowercase();
            if used.contains(&key) || !has_substance(bullet_text) {
                continue;
            }
            if !grounds_in_scene(bullet, bullet_text, text, scene_range) {
                continue;
            }
            let mut start_us = scene_start_us;
            // Anchor on the first certified phrase inside the scene. The v2
            // request carries producer annotations, never an already-projected
            // phrase list, so the anchor reads the certified surface directly.
            if let Some(first) = scene
                .annotations
                .iter()
                .flat_map(|annotations| annotations.phrases.iter())
                .find(|phrase| {
                    phrase.duration_us > 0
                        && phrase.start_us >= scene_start_us
                        && phrase.start_us < scene_end_us
                })
            {
                start_us = first.start_us;
            }
            let spoken_us = scene
                .voiceover
                .as_ref()
                .and_then(|voiceover| voiceover.timing.as_ref())
                .and_then(|timing| located_duration(timing, bullet_text))
                .unwrap_or(CHAPTER_BULLET_WINDOW_US);
            let duration_us = spoken_us.min(scene_end_us - start_us);
            if duration_us <= 0 {
                continue;
            }
            response
                .scene_phrases
                .entry(scene.id.clone())
                .or_default()
                .push(ScenePhraseV2 {
                    text: bullet_text.to_string(),
                    start_us,
                    duration_us,
                    score: CHAPTER_BULLET_SCORE,
                    presentation_role: CHAPTER_BULLET_ROLE.to_string(),
                    candidate: "bullet".to_string(),
                });
            used.insert(key);
            covered.insert(scene.id.clone());
            break; // the visual contract is one bullet per scene
        }
    }
    response.covered_scenes = covered.into_iter().collect();
    response
}

/// Mirrors Go's `SceneExecutionMode.Normalize`: empty and `generated` are
/// ordinary scenes; `fixed_media` and every unknown non-empty mode are
/// protected fixed media (fail closed).
fn is_fixed_media(mode: &str) -> bool {
    !matches!(mode, "" | "generated")
}

fn category_disabled(scene: &SceneV2, category: &str, global: &BTreeMap<String, String>) -> bool {
    if let Some(override_value) = scene.overlay_categories.get(category) {
        if override_value != "default" {
            return override_value == "disabled";
        }
    }
    global
        .get(category)
        .map(|value| value == "disabled")
        .unwrap_or(false)
}

fn has_substance(text: &str) -> bool {
    if text.split_whitespace().count() >= MIN_BULLET_WORDS {
        return true;
    }
    text.chars()
        .filter(|character| character.is_ascii_alphanumeric() || (*character as u32) > 127)
        .count()
        >= MIN_BULLET_CHARS
}

fn grounds_in_scene(
    bullet: &ChapterBulletV2,
    bullet_text: &str,
    scene_text: &str,
    scene_range: Option<(i64, i64)>,
) -> bool {
    if scene_text
        .to_lowercase()
        .contains(&bullet_text.to_lowercase())
    {
        return true;
    }
    let Some((range_start, range_end)) = scene_range else {
        return false;
    };
    if range_end <= range_start {
        return false;
    }
    let overlaps = bullet.start_sentence < range_end && bullet.end_sentence > range_start;
    if !overlaps {
        return false;
    }
    let lead = bullet_text
        .split_whitespace()
        .take(LEAD_WORDS)
        .collect::<Vec<_>>()
        .join(" ");
    !lead.is_empty() && scene_text.to_lowercase().contains(&lead.to_lowercase())
}

/// Splits the scene text into sentences exactly like the Go projection: the
/// terminator stays on the sentence, and whitespace-only tails are dropped.
fn editorial_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if !matches!(character, '.' | '!' | '?' | '\n') {
            continue;
        }
        let end = index + character.len_utf8();
        let sentence = text[start..end].trim();
        if !sentence.is_empty() {
            sentences.push(sentence.to_string());
        }
        start = end;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    sentences
}

#[derive(Debug, Clone)]
struct Located {
    word_start: i64,
    word_end: i64,
    start_us: i64,
    end_us: i64,
}

/// Returns the duration of the certified spoken span. A full exact match wins;
/// otherwise the first/last endpoint fallback picks the ordered pair whose
/// word count is closest to the source phrase length.
fn located_duration(timing: &TimingV2, phrase: &str) -> Option<i64> {
    if let Some(first) = locate_all(timing, phrase).into_iter().next() {
        let duration = first.end_us - first.start_us;
        return (duration > 0).then_some(duration);
    }
    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let first_matches = locate_all(timing, words[0]);
    let last_matches = locate_all(timing, words[words.len() - 1]);
    let mut best: Option<(i64, Located, Located)> = None;
    for start in &first_matches {
        for end in &last_matches {
            if end.word_end <= start.word_start {
                continue;
            }
            let span_words = end.word_end - start.word_start + 1;
            let delta = (span_words - words.len() as i64).abs();
            if best
                .as_ref()
                .map(|(best_delta, _, _)| delta < *best_delta)
                .unwrap_or(true)
            {
                best = Some((delta, start.clone(), end.clone()));
            }
        }
    }
    let (_, start, end) = best?;
    let duration = end.end_us - start.start_us;
    (duration > 0).then_some(duration)
}

fn locate_all(timing: &TimingV2, phrase: &str) -> Vec<Located> {
    let query = phrase_tokens(phrase);
    if query.is_empty() {
        return Vec::new();
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
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut start = 0;
    while start + query.len() <= normalized.len() {
        let matched = query
            .iter()
            .zip(&normalized[start..start + query.len()])
            .all(|(query_token, timing_token)| token_match(query_token, timing_token));
        if matched {
            let first = &timing.words[source_word[start]];
            let last = &timing.words[source_word[start + query.len() - 1]];
            found.push(Located {
                word_start: first.index,
                word_end: last.index,
                start_us: first.start_us,
                end_us: last.end_us,
            });
            // Non-overlapping: continue scanning right after the match.
            start += query.len();
        } else {
            start += 1;
        }
    }
    found
}

fn phrase_tokens(phrase: &str) -> Vec<String> {
    phrase
        .split_whitespace()
        .flat_map(fragment_tokens)
        .collect()
}

fn fragment_tokens(field: &str) -> Vec<String> {
    field
        .split(|character: char| character.is_whitespace() || is_dash(character))
        .filter_map(|fragment| {
            let token = normalize_token(fragment);
            (!token.is_empty()).then_some(token)
        })
        .collect()
}

fn normalize_token(token: &str) -> String {
    let folded: String = token
        .nfc()
        .flat_map(|character| character.to_lowercase())
        .map(fold_apostrophe)
        .collect();
    let trimmed = folded.trim_matches(is_edge_punctuation);
    trimmed.trim_end_matches('\'').to_string()
}

fn fold_apostrophe(character: char) -> char {
    match character {
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{02BC}' => '\'',
        other => other,
    }
}

fn is_dash(character: char) -> bool {
    matches!(
        character,
        '-' | '\u{2010}'
            | '\u{2011}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{2015}'
            | '\u{2212}'
    )
}

fn is_edge_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '«' | '»' | '“' | '”' | '‘' | '’' | '…' | '–' | '—' | '¡' | '¿'
        )
}

fn token_match(query: &str, timing: &str) -> bool {
    if query == timing {
        return true;
    }
    if query.is_empty() || timing.is_empty() || !is_numeric(query) {
        return false;
    }
    let Some(tail) = timing.strip_suffix(query) else {
        return false;
    };
    !tail.is_empty() && tail.ends_with('\'')
}

fn is_numeric(token: &str) -> bool {
    let mut seen_dot = false;
    let mut seen_digit = false;
    for character in token.chars() {
        match character {
            '0'..='9' => seen_digit = true,
            '.' | ',' if !seen_dot => seen_dot = true,
            _ => return false,
        }
    }
    seen_digit
}

#[cfg(test)]
mod tests {
    use super::{project_chapter_bullets, CHAPTER_BULLET_ROLE, CHAPTER_BULLET_WINDOW_US};
    use crate::editorial_plan_v2::{
        CanvasV2, ChapterBulletV2, ChapterManifestV2, ChapterV2, EditorialPlanV2Request,
        PlanIdentityV2, SceneAnnotationsV2, ScenePhraseV2, SceneV2, TimedAnnotationV2, TimingV2,
        VoiceoverV2, WordTimingV2, CHAPTER_MANIFEST_SCHEMA_VERSION,
        EDITORIAL_PLAN_V2_SCHEMA_VERSION,
    };
    use std::collections::BTreeMap;

    const S1: &str = "Il caso riguarda la morte di Chiara Poggi, avvenuta il 13 agosto 2007 nella sua abitazione a Garlasco.";
    const S2: &str = "Alberto Stasi e stato condannato in via definitiva a sedici anni di reclusione per lomicidio.";

    fn scene(id: &str, text: &str, start_us: i64, duration_us: i64) -> SceneV2 {
        SceneV2 {
            id: id.to_string(),
            text: text.to_string(),
            duration_ms: duration_us / 1000,
            execution_mode: "generated".to_string(),
            short_phrase_overlay: false,
            overlay_categories: BTreeMap::new(),
            voiceover: None,
            timeline_start_us: start_us,
            duration_us,
            annotations: None,
        }
    }

    fn request() -> EditorialPlanV2Request {
        EditorialPlanV2Request {
            schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            plan: PlanIdentityV2 {
                plan_id: "plan".to_string(),
                video_id: "video".to_string(),
                project_id: "project".to_string(),
                motion_seed: String::new(),
                coherence_seed: String::new(),
                width: 1920,
                height: 1080,
                fps_num: 24,
                fps_den: 1,
            },
            language: "it".to_string(),
            source_language: "it".to_string(),
            operation: "compile_editorial_plan".to_string(),
            seal: None,
            entity_timeline: None,
            media: None,
            budgets: None,
            scene_windows: BTreeMap::new(),
            canvas: CanvasV2::default(),
            scenes: vec![
                scene("s1", S1, 0, 6_000_000),
                scene("s2", S2, 6_000_000, 3_000_000),
            ],
            editorial: None,
            chapters: Some(ChapterManifestV2 {
                schema_version: CHAPTER_MANIFEST_SCHEMA_VERSION.to_string(),
                chapters: vec![
                    ChapterV2 {
                        bullets: vec![ChapterBulletV2 {
                            text: S1.to_string(),
                            start_sentence: 0,
                            end_sentence: 1,
                        }],
                    },
                    ChapterV2 {
                        bullets: vec![ChapterBulletV2 {
                            text: S2.to_string(),
                            start_sentence: 1,
                            end_sentence: 2,
                        }],
                    },
                ],
            }),
        }
    }

    fn chapter_phrase(
        scene_id: &str,
        response: &crate::editorial_plan_v2::EditorialPlanV2Response,
    ) -> Option<ScenePhraseV2> {
        response
            .scene_phrases
            .get(scene_id)
            .and_then(|phrases| {
                phrases
                    .iter()
                    .find(|phrase| phrase.presentation_role == CHAPTER_BULLET_ROLE)
            })
            .cloned()
    }

    #[test]
    fn chapter_bullets_render_without_voiceover() {
        let response = project_chapter_bullets(&request());
        assert_eq!(response.covered_scenes, vec!["s1", "s2"]);
        let first = chapter_phrase("s1", &response).expect("s1 bullet");
        assert_eq!(first.text, S1);
        assert_eq!(first.start_us, 0);
        assert_eq!(first.duration_us, CHAPTER_BULLET_WINDOW_US);
        let second = chapter_phrase("s2", &response).expect("s2 bullet");
        assert_eq!(second.start_us, 6_000_000);
        assert_eq!(second.duration_us, 3_000_000);
    }

    #[test]
    fn chapter_bullet_skips_explicit_short_phrase_scene() {
        let mut request = request();
        request.scenes[0].short_phrase_overlay = true;
        let response = project_chapter_bullets(&request);
        assert!(!response.covered_scenes.contains(&"s1".to_string()));
        assert!(chapter_phrase("s1", &response).is_none());
        assert!(response.covered_scenes.contains(&"s2".to_string()));
    }

    #[test]
    fn chapter_bullet_respects_disabled_category() {
        let mut request = request();
        request
            .canvas
            .overlay_categories
            .insert("bullets".to_string(), "disabled".to_string());
        request.scenes[1]
            .overlay_categories
            .insert("bullets".to_string(), "enabled".to_string());
        let response = project_chapter_bullets(&request);
        assert!(!response.covered_scenes.contains(&"s1".to_string()));
        assert!(response.covered_scenes.contains(&"s2".to_string()));
    }

    #[test]
    fn chapter_bullet_holds_certified_spoken_duration() {
        let mut request = request();
        let words: Vec<&str> = S1.split_whitespace().collect();
        let boundaries: Vec<WordTimingV2> = words
            .iter()
            .enumerate()
            .map(|(index, word)| WordTimingV2 {
                index: index as i64,
                text: (*word).to_string(),
                start_us: index as i64 * 300_000 + 100_000,
                end_us: (index as i64 + 1) * 300_000 + 100_000,
            })
            .collect();
        let duration_us = words.len() as i64 * 300_000 + 100_000;
        request.scenes[0].voiceover = Some(VoiceoverV2 {
            timing: Some(TimingV2 {
                duration_us,
                words: boundaries,
            }),
        });
        let response = project_chapter_bullets(&request);
        let bullet = chapter_phrase("s1", &response).expect("s1 bullet");
        assert_eq!(bullet.duration_us, duration_us - 100_000);
        assert!(bullet.duration_us > CHAPTER_BULLET_WINDOW_US);
    }

    #[test]
    fn chapter_bullet_anchors_to_first_phrase() {
        let mut request = request();
        request.scenes[0].annotations = Some(SceneAnnotationsV2 {
            phrases: vec![TimedAnnotationV2 {
                text: "Il caso riguarda".to_string(),
                annotation_type: String::new(),
                start_ms: 500,
                end_ms: 2_000,
                start_us: 500_000,
                duration_us: 1_500_000,
                score: 0.5,
                presentation_role: String::new(),
                editorial_candidate: String::new(),
            }],
            ..Default::default()
        });
        let response = project_chapter_bullets(&request);
        let bullet = chapter_phrase("s1", &response).expect("s1 bullet");
        assert_eq!(bullet.start_us, 500_000);
    }

    #[test]
    fn chapter_bullet_fails_closed() {
        let mut no_manifest = request();
        no_manifest.chapters = None;
        let response = project_chapter_bullets(&no_manifest);
        assert!(response.covered_scenes.is_empty());
        assert!(response.scene_phrases.is_empty());

        let mut language_mismatch = request();
        language_mismatch.language = "en".to_string();
        let response = project_chapter_bullets(&language_mismatch);
        assert!(response.scene_phrases.is_empty());

        let mut ungrounded = request();
        ungrounded.chapters.as_mut().expect("chapters").chapters[0].bullets[0].text =
            "Not spoken anywhere in these scenes at all.".to_string();
        let response = project_chapter_bullets(&ungrounded);
        assert!(!response.covered_scenes.contains(&"s1".to_string()));
        assert!(chapter_phrase("s1", &response).is_none());

        let mut fragment = request();
        fragment.chapters.as_mut().expect("chapters").chapters[1].bullets[0].text =
            "sedici anni".to_string();
        let response = project_chapter_bullets(&fragment);
        assert!(!response.covered_scenes.contains(&"s2".to_string()));

        let mut missing_bounds = request();
        missing_bounds.scenes[0].duration_us = 0;
        let response = project_chapter_bullets(&missing_bounds);
        assert!(!response.covered_scenes.contains(&"s1".to_string()));
    }

    #[test]
    fn one_bullet_renders_once() {
        let mut request = request();
        request.chapters.as_mut().expect("chapters").chapters[1].bullets[0].text = S1.to_string();
        request.scenes[1].text = format!("{} {}", S2, S1);
        let response = project_chapter_bullets(&request);
        let rendered: usize = response
            .scene_phrases
            .values()
            .map(|phrases| {
                phrases
                    .iter()
                    .filter(|phrase| phrase.presentation_role == CHAPTER_BULLET_ROLE)
                    .count()
            })
            .sum();
        assert_eq!(rendered, 1);
        assert!(response.covered_scenes.contains(&"s1".to_string()));
        assert!(!response.covered_scenes.contains(&"s2".to_string()));
    }

    #[test]
    fn batch_envelope_carries_the_chapter_projection() {
        let plan = crate::compile_editorial_plan(crate::CompileEditorialPlanRequest {
            fps_num: 0,
            fps_den: 0,
            scene_windows: BTreeMap::new(),
            items: Vec::new(),
            entity_scenes: Vec::new(),
            entrance_items: Vec::new(),
            v2: Some(request()),
        });
        let projection = plan.v2.expect("v2 projection");
        assert_eq!(projection.covered_scenes, vec!["s1", "s2"]);
        assert_eq!(projection.scene_phrases.len(), 2);
        assert_eq!(
            projection.scene_phrases["s1"][0].presentation_role,
            CHAPTER_BULLET_ROLE
        );
    }

    #[test]
    fn merged_span_opens_chapter() {
        let mut request = request();
        let merged = format!("{} {}", S1.trim(), S2.trim());
        request.chapters.as_mut().expect("chapters").chapters = vec![ChapterV2 {
            bullets: vec![ChapterBulletV2 {
                text: merged.clone(),
                start_sentence: 0,
                end_sentence: 2,
            }],
        }];
        let response = project_chapter_bullets(&request);
        assert!(response.covered_scenes.contains(&"s1".to_string()));
        assert!(!response.covered_scenes.contains(&"s2".to_string()));
        let bullet = chapter_phrase("s1", &response).expect("s1 bullet");
        assert_eq!(bullet.text, merged);
        assert_eq!(bullet.start_us, 0);
    }
}
