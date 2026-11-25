use super::*;

pub(crate) fn append_editorial(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    editorial: &crate::editorial_plan_v2::response::EditorialPlanV2Response,
) {
    let Some(phrases) = editorial.scene_phrases.get(&scene.id) else {
        return;
    };
    let title = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "scene_title");
    // The editorial projection can return several grounded scene bullets.
    // Selecting only the first one per scene discarded the rest before
    // admission, so raising phrase/image budgets could not create the
    // requested overlay density. Keep the candidates and let the canonical
    // run and per-scene budgets make the selection.
    let bullets = phrases
        .iter()
        .filter(|phrase| phrase.presentation_role == "scene_bullet");
    let chapter = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "chapter_bullet");
    let mut selected = Vec::new();
    selected.extend(title);
    selected.extend(bullets);
    selected.extend(chapter);
    for (ordinal, phrase) in selected.into_iter().enumerate() {
        if !scene.short_phrase_overlay && word_count(&phrase.text) < 3 {
            // Go filters every phrase lane through `phrasesAtLeastThreeWords`
            // after the editorial projection. A two-word title is not a
            // renderable block overlay unless the scene explicitly owns the
            // short-phrase lane.
            continue;
        }
        let role = phrase.presentation_role.as_str();
        let mut item = scene_item(
            format!("{}-editorial-{ordinal}", scene.id),
            scene,
            "text_phrase",
            "IMPORTANT_PHRASE",
            phrase.start_us,
            phrase.duration_us,
            phrase.text.clone(),
        );
        item.params = Some(json!({
            "editorial_role": role,
            "editorial_candidate": phrase.candidate,
            "priority": phrase.score,
        }));
        items.push(item);
    }
}

pub(crate) fn append_annotations(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    annotations: &crate::editorial_plan_v2::request::SceneAnnotationsV2,
    request: &EditorialPlanV2Request,
) {
    for (index, phrase) in annotations.phrases.iter().enumerate() {
        if !phrase.presentation_role.is_empty() {
            continue; // editorial projection owns title/bullet candidates
        }
        if !scene.short_phrase_overlay && word_count(&phrase.text) < 3 {
            continue;
        }
        items.push(text_item(
            scene,
            "phrase",
            index,
            phrase,
            "text_phrase",
            "IMPORTANT_PHRASE",
        ));
    }
    for (index, keyword) in annotations.keywords.iter().enumerate() {
        items.push(text_item(
            scene,
            "word",
            index,
            keyword,
            "important_word",
            "IMPORTANT_WORD",
        ));
    }
    for (index, quote) in annotations.quotes.iter().enumerate() {
        items.push(text_item(scene, "quote", index, quote, "quote", "QUOTE"));
    }
    for (index, brand) in annotations.brand_texts.iter().enumerate() {
        items.push(text_item(
            scene,
            "brand",
            index,
            brand,
            "brand_text",
            "logo_default",
        ));
    }
    for (index, number) in annotations.numbers.iter().enumerate() {
        if let Some(item) = number_item(scene, index, number, request) {
            items.push(item);
        }
    }
    for (index, number) in annotations.chart_numbers.iter().enumerate() {
        let mut item = text_item(scene, "chart-number", index, number, "number", "NUMBER");
        if let Some(params) = item.params.as_mut().and_then(Value::as_object_mut) {
            params.insert("chart".to_string(), json!(true));
        }
        items.push(item);
    }
    for (index, image) in annotations.images.iter().enumerate() {
        append_image(items, scene, "image", index, image, "IMAGE_OVERLAY");
    }
    for (index, product) in annotations.products.iter().enumerate() {
        append_image(items, scene, "product", index, product, "PRODUCT");
    }
    for (index, logo) in annotations.logos.iter().enumerate() {
        append_image(items, scene, "logo", index, logo, "LOGO");
    }
    for (index, map) in annotations.maps.iter().enumerate() {
        append_map(items, scene, index, map);
    }
}

/// Routes a grounded numeric surface to its certified presentation family.
///
/// Dates are capped at the owning scene and kept on screen for the same
/// five-second target as the Go planner; a date with less than two readable
/// seconds left is dropped before admission instead of consuming a slot.
/// `42`-style values use the metric card family. Chart numbers stay separate
/// candidates and are lowered by the chart lane.
/// Certified numeric presentation lanes. Template, motion lane and typography
/// travel together, so a new family is one variant instead of another
/// `is_date || is_metric` chain in the middle of the item builder.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumericLane {
    Date,
    Metric,
    Plain,
}

impl NumericLane {
    fn classify(annotation_type: &str) -> Self {
        let family = annotation_type.trim().to_ascii_uppercase();
        if matches!(family.as_str(), "DATE" | "TIME") {
            Self::Date
        } else if matches!(
            family.as_str(),
            "NUMBER"
                | "NUM"
                | "CARDINAL"
                | "ORDINAL"
                | "MONEY"
                | "PERCENT"
                | "PERCENTAGE"
                | "METRIC"
                | "METRICS"
                | "STATISTIC"
                | "STATISTICS"
                | "QUANTITY"
        ) {
            Self::Metric
        } else {
            Self::Plain
        }
    }

    fn template_id(self) -> &'static str {
        match self {
            Self::Date => "TIMELINE_DATE_CARD",
            Self::Metric => "METRIC_STAT_CARD",
            Self::Plain => "NUMBER",
        }
    }

    /// `(animation_counts key, seed family)` for the seeded motion sampler.
    fn motion_lane(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Date => Some(("timeline_date", "date_presentation")),
            Self::Metric => Some(("metric_stat", "metric_presentation")),
            Self::Plain => None,
        }
    }

    fn motion_pool(self, request: &EditorialPlanV2Request) -> Vec<&'static str> {
        match self {
            Self::Date => date_motion_pool(request),
            Self::Metric => METRIC_MOTIONS.to_vec(),
            Self::Plain => Vec::new(),
        }
    }

    fn font_size(self) -> f64 {
        match self {
            Self::Date | Self::Metric => 140.0,
            Self::Plain => 112.0,
        }
    }
}

pub(crate) fn number_item(
    scene: &SceneV2,
    _index: usize,
    annotation: &TimedAnnotationV2,
    request: &EditorialPlanV2Request,
) -> Option<PlanItemV2> {
    let lane = NumericLane::classify(&annotation.annotation_type);
    let scene_end = scene.timeline_start_us.saturating_add(scene.duration_us);
    let start_us = annotation.start_us.max(scene.timeline_start_us);
    if lane == NumericLane::Date && scene_end.saturating_sub(start_us) < 2_000_000 {
        return None;
    }
    let id = go_item_id(&scene.id, "number", &annotation.text);
    let mut item = scene_item(
        id.clone(),
        scene,
        "number",
        lane.template_id(),
        start_us,
        annotation.duration_us.max(0),
        annotation.text.clone(),
    );
    if let Some((counts_key, family_key)) = lane.motion_lane() {
        let pool = lane.motion_pool(request);
        let limit = request
            .canvas
            .animation_counts
            .get(counts_key)
            .copied()
            .unwrap_or(0)
            .max(0) as usize;
        let pool = if limit > 0 {
            limited_shuffled_pool(
                &pool,
                limit,
                &format!("{}\x00{}", request.plan.plan_id, family_key),
            )
        } else {
            pool
        };
        item.motion_id = seeded_preset(&request.plan.plan_id, &scene.id, &id, family_key, &pool)
            .unwrap_or_default();
    }
    if lane == NumericLane::Date {
        item.duration_us = item
            .duration_us
            .max(5_000_000)
            .min(scene_end.saturating_sub(item.start_us));
        item.end_ms = (item.start_us + item.duration_us + 999) / 1000;
    }
    item.params = Some(json!({
        "position": "center",
        "style": "stat",
        "priority": annotation.score,
        "font_size_px": lane.font_size(),
    }));
    Some(item)
}

pub(crate) fn date_motion_pool(request: &EditorialPlanV2Request) -> Vec<&'static str> {
    // `validate_compilable` rejects unknown selectors before this phase, so an
    // invalid value cannot reach the pool; `All` also covers the absent
    // selector and keeps the historical fail-open to every date motion.
    let family = DateMotionFamily::parse(&request.canvas.date_motion_family)
        .unwrap_or(DateMotionFamily::All);
    DATE_MOTIONS
        .iter()
        .copied()
        .filter(|motion| family.allows(motion))
        .collect()
}

#[derive(Serialize)]
pub(crate) struct PresetSeed<'a> {
    pub(crate) job_fingerprint: &'a str,
    pub(crate) scene_id: &'a str,
    pub(crate) semantic_id: &'a str,
    pub(crate) preset_family: &'a str,
}

/// Mirrors Go's `DeterministicPresetSampler`: a SHA-256 over the stable JSON
/// seed maps its first eight bytes onto the candidate list. This is the one
/// sampler used by date cards, light leaks and every seeded presentation
/// choice; wall-clock time never participates.
/// Assigns the short-phrase entrance to every 1-5-word phrase that carries
/// none.
///
/// The lane used to be written by the Go producer and was retired with its
/// selectors, which left `motion_id` empty on the wire: the renderer then fell
/// back to the preset's single motion and every phrase looked identical. The
/// pin is authoritative; with no pin the item rotates the certified bucket for
/// its own word count, seeded by (plan, scene, item) so a replay picks the same
/// motion.
///
/// Editorial titles and bullets are part of the lane. They were left to a
/// separate pin contract, but production plans usually ship them WITHOUT a
/// motion pin, so the renderer fell back to the preset's single entrance and a
/// run of short editorial cards looked identical. Only a bullet pin that
/// actually names a motion keeps the item out of the rotation; a font-only pin
/// still receives its entrance here. A phrase longer than five words belongs
/// to the long lane.
pub(crate) fn assign_short_phrase_motions(
    items: &mut [PlanItemV2],
    request: &EditorialPlanV2Request,
) {
    let canvas = &request.canvas;
    let pin = canvas.short_phrase_motion.trim();
    // Sample independently for each phrase from the run seed and item identity.
    // The used set prevents repeats until the available pool is exhausted.
    let mut used: BTreeSet<String> = BTreeSet::new();
    for item in items.iter_mut() {
        if item.kind != "text_phrase" || item.template_id != "IMPORTANT_PHRASE" {
            continue;
        }
        if !item.motion_id.trim().is_empty() || has_pinned_bullet_motion(request, item) {
            continue;
        }
        let words = word_count(&item.text);
        if words == 0 || words > 5 {
            continue;
        }
        if !pin.is_empty() {
            item.motion_id = pin.to_string();
            continue;
        }
        let bucket: Vec<&str> = canvas
            .short_phrase_motions_by_word
            .get(&words.to_string())
            .map(|bucket| bucket.iter().map(String::as_str).collect())
            .unwrap_or_default();
        if bucket.is_empty() {
            continue;
        }
        let filtered: Vec<&str> = bucket
            .iter()
            .copied()
            .filter(|id| family_matches(id, &canvas.phrase_motion_family))
            .collect();
        let eligible = if filtered.is_empty() {
            &bucket
        } else {
            &filtered
        };
        let ordered = ordered_short_phrase_pool(eligible, motion_seed(request), words);
        let start = seeded_index(
            motion_seed(request),
            &item.scene_id,
            &item.id,
            &format!("short_phrase_{words}"),
            ordered.len(),
        )
        .unwrap_or(0);
        let motion = (0..ordered.len())
            .map(|step| ordered[(start + step) % ordered.len()])
            .find(|candidate| !used.contains(*candidate))
            .unwrap_or(ordered[start])
            .to_string();
        used.insert(motion.clone());
        item.motion_id = motion;
    }
}
