use super::*;

/// Allocates the producer-verified scene images across the run ceiling.
///
/// Go supplies only the verified candidate sets and the excluded entity
/// hashes; it must not choose which scene wins an image slot. One candidate
/// per scene is selected before any scene gets a second slot, matching the
/// historical round-robin policy without letting the first scene consume the
/// whole ceiling.
pub(super) fn allocate_scene_images(
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
    let sets: Vec<&crate::editorial_plan_v2::request::SceneImageCandidateSetV2> = media
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
pub(super) fn project_recap_briefs(request: &mut EditorialPlanV2Request) {
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

pub(super) fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let end = trimmed.find(['.', '!', '?', '\n']).unwrap_or(trimmed.len());
    trimmed[..end].trim().to_string()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_scene(
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
pub(super) fn project_entity(
    out: &mut SceneAnnotationsV2,
    scene: &SceneV2,
    entity: &RawEntityV2,
    timing: Option<&crate::editorial_plan_v2::request::TimingV2>,
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

pub(super) fn occurrence_for<'a>(
    timeline: Option<&'a EntityTimelineV2>,
    scene: &SceneV2,
    entity: &RawEntityV2,
) -> Option<&'a crate::editorial_plan_v2::request::EntityOccurrenceV2> {
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

pub(super) fn image_candidate(
    image: &crate::editorial_plan_v2::request::RawEntityImageV2,
    occurrence: &crate::editorial_plan_v2::request::EntityOccurrenceV2,
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

pub(super) fn spoken_window_duration(audio_start_us: i64, audio_end_us: i64) -> i64 {
    let spoken = (audio_end_us - audio_start_us).max(0);
    let duration = (spoken + 500_000).clamp(1_000_000, 5_000_000);
    ((duration + 999) / 1000) * 1000
}
