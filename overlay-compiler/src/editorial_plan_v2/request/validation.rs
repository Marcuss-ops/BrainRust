use std::collections::BTreeMap;

use super::super::date_motion_family::DateMotionFamily;
use super::*;

impl EditorialPlanV2Request {
    /// Structural, fail-closed validation. A request that fails here must not
    /// reach any decision code: a malformed manifest is a caller bug, not a
    /// silent no-op.
    pub fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            });
        }
        if !self.operation.trim().is_empty() && self.operation.trim() != EDITORIAL_PLAN_OPERATION {
            return Err(EditorialPlanError::invalid(
                "operation",
                format!("expected {EDITORIAL_PLAN_OPERATION:?}"),
            ));
        }
        if self.plan.plan_id.trim().is_empty() || self.plan.video_id.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                "plan",
                "plan_id and video_id are required",
            ));
        }
        if self.plan.width <= 0 || self.plan.height <= 0 {
            return Err(EditorialPlanError::invalid(
                "plan",
                "width and height must be positive",
            ));
        }
        if self.plan.fps_num <= 0 || self.plan.fps_den <= 0 {
            return Err(EditorialPlanError::invalid(
                "plan",
                "fps_num and fps_den must be positive",
            ));
        }
        if self.language.trim().is_empty() {
            return Err(EditorialPlanError::invalid("language", "is required"));
        }
        if let Some(seal) = &self.seal {
            if seal.audio_duration_us < 0 {
                return Err(EditorialPlanError::invalid(
                    "seal.audio_duration_us",
                    "must not be negative",
                ));
            }
        }
        self.canvas.validate()?;
        if self.canvas.max_gap_us < 0 {
            return Err(EditorialPlanError::invalid(
                "canvas.max_gap_us",
                "must not be negative",
            ));
        }
        if self.canvas.gap_cooldown_us < 0 {
            return Err(EditorialPlanError::invalid(
                "canvas.gap_cooldown_us",
                "must not be negative",
            ));
        }
        if !self.canvas.coverage_min_percent.is_finite()
            || !(0.0..=100.0).contains(&self.canvas.coverage_min_percent)
        {
            return Err(EditorialPlanError::invalid(
                "canvas.coverage_min_percent",
                "must be a finite percentage in [0,100]",
            ));
        }
        if !self.canvas.phrase_per_minute.is_finite() || self.canvas.phrase_per_minute < 0.0 {
            return Err(EditorialPlanError::invalid(
                "canvas.phrase_per_minute",
                "must be a finite non-negative rate",
            ));
        }

        let mut scene_ids = BTreeSet::new();
        for scene in &self.scenes {
            let id = scene.id.trim();
            if id.is_empty() {
                return Err(EditorialPlanError::invalid(
                    "scenes",
                    "scene id is required",
                ));
            }
            if !scene_ids.insert(id.to_string()) {
                return Err(EditorialPlanError::invalid(
                    "scenes",
                    format!("duplicate scene id {id:?}"),
                ));
            }
            if scene.duration_ms < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("scenes.{id}.duration_ms"),
                    "must not be negative",
                ));
            }
            if scene.timeline_start_us < 0 || scene.duration_us < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("scenes.{id}"),
                    "canonical bounds must not be negative",
                ));
            }
            validate_categories(id, &scene.overlay_categories)?;
            if let Some(voiceover) = &scene.voiceover {
                if let Some(timing) = &voiceover.timing {
                    validate_timing(id, timing)?;
                }
            }
            if let Some(annotations) = &scene.annotations {
                validate_annotations(id, scene, annotations)?;
            }
            if let Some(raw) = &scene.raw {
                validate_visual_recipe(
                    &format!("scenes.{id}.raw.visual_recipe"),
                    &format!("scenes.{id}.raw.visual_payload"),
                    &raw.visual_recipe,
                    raw.visual_payload.as_ref(),
                )?;
                validate_visual_recipe_list(&format!("scenes.{id}.raw"), &raw.visual_recipes)?;
            }
        }
        for clip in &self.clips {
            if !scene_ids.contains(clip.scene_id.trim()) {
                return Err(EditorialPlanError::invalid(
                    "clips",
                    format!("unknown scene id {:?}", clip.scene_id),
                ));
            }
            if clip.start_us < 0 || clip.duration_us < 0 {
                return Err(EditorialPlanError::invalid("clips", "negative clip window"));
            }
        }
        for brief in &self.recap_briefs {
            if brief.id.trim().is_empty() || brief.title.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "recap_briefs",
                    "id and title are required",
                ));
            }
            if brief.start_us < 0 || brief.end_us <= brief.start_us {
                return Err(EditorialPlanError::invalid(
                    "recap_briefs",
                    "invalid scene window",
                ));
            }
        }
        for scene_id in self.scene_windows.keys() {
            if !scene_ids.contains(scene_id.trim()) {
                return Err(EditorialPlanError::invalid(
                    "scene_windows",
                    format!("unknown scene id {scene_id:?}"),
                ));
            }
        }
        let known_scenes: BTreeSet<&str> = scene_ids.iter().map(String::as_str).collect();
        if let Some(timeline) = &self.entity_timeline {
            timeline.validate(&known_scenes)?;
        }
        if let Some(media) = &self.media {
            media.validate(&known_scenes)?;
        }
        if let Some(budgets) = &self.budgets {
            budgets.validate()?;
        }
        if let Some(editorial) = &self.editorial {
            editorial.validate()?;
        }
        if let Some(chapters) = &self.chapters {
            chapters.validate()?;
        }
        Ok(())
    }

    /// Requires the sections the single-call planner cannot run without.
    ///
    /// [`Self::validate`] accepts the partial envelope the first landed slice
    /// shipped (identity, scenes, manifests). The one-call compiler is stricter:
    /// a missing extent, media index, budget or entity timeline is an error,
    /// never an implicit empty fallback.
    pub fn validate_compilable(&self) -> Result<(), EditorialPlanError> {
        self.validate()?;
        if self.seal.is_none() {
            return Err(EditorialPlanError::MissingSection("seal"));
        }
        if self.media.is_none() {
            return Err(EditorialPlanError::MissingSection("media"));
        }
        if self.budgets.is_none() {
            return Err(EditorialPlanError::MissingSection("budgets"));
        }
        if self.entity_timeline.is_none() {
            return Err(EditorialPlanError::MissingSection("entity_timeline"));
        }
        if self.scenes.is_empty() {
            return Err(EditorialPlanError::invalid(
                "scenes",
                "at least one scene is required",
            ));
        }
        for scene in &self.scenes {
            if scene.duration_us <= 0 {
                continue;
            }
            let Some(window) = self.scene_windows.get(&scene.id) else {
                return Err(EditorialPlanError::invalid(
                    "scene_windows",
                    format!("missing canonical window for scene {:?}", scene.id),
                ));
            };
            if window[1] - window[0] != scene.duration_us {
                return Err(EditorialPlanError::invalid(
                    format!("scene_windows.{}", scene.id),
                    "window extent does not match the scene duration",
                ));
            }
            if window[0] != scene.timeline_start_us {
                return Err(EditorialPlanError::invalid(
                    format!("scene_windows.{}", scene.id),
                    "window start does not match the scene timeline start",
                ));
            }
        }
        Ok(())
    }
}

#[path = "validation/media.rs"]
mod media;
use media::*;

mod media_contracts;
fn validate_annotations(
    scene_id: &str,
    scene: &SceneV2,
    annotations: &SceneAnnotationsV2,
) -> Result<(), EditorialPlanError> {
    let label = format!("scenes.{scene_id}.annotations");
    for (name, spans) in [
        ("phrases", &annotations.phrases),
        ("keywords", &annotations.keywords),
        ("numbers", &annotations.numbers),
        ("chart_numbers", &annotations.chart_numbers),
        ("brand_texts", &annotations.brand_texts),
        ("quotes", &annotations.quotes),
    ] {
        for span in spans {
            if span.text.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    "annotation text is required",
                ));
            }
            if span.start_us < 0 || span.duration_us < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    "negative annotation window",
                ));
            }
            if span.end_ms < span.start_ms && span.end_ms != 0 {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    "annotation end_ms precedes start_ms",
                ));
            }
        }
    }
    if !annotations.chart_layout.is_empty()
        && !matches!(
            annotations.chart_layout.as_str(),
            "line_series"
                | "donut_share"
                | "before_after"
                | "delta_comparison"
                | "timeline_growth"
                | "qualified_growth"
                | "missing_data_timeline"
                | "segmented_total_bar"
                | "scenario_split"
                | "bounded_range_comparison"
                | "dual_comparison_bars"
                | "bar_comparison"
                | "ranking_bar"
                | "before_after_metric"
        )
    {
        return Err(EditorialPlanError::invalid(
            format!("{label}.chart_layout"),
            "unsupported grounded chart layout",
        ));
    }
    validate_visual_recipe(
        &format!("{label}.visual_recipe"),
        &format!("{label}.visual_payload"),
        &annotations.visual_recipe,
        annotations.visual_payload.as_ref(),
    )?;
    validate_visual_recipe_list(&label, &annotations.visual_recipes)?;
    for (name, candidates) in [
        ("images", &annotations.images),
        ("products", &annotations.products),
        ("logos", &annotations.logos),
    ] {
        for candidate in candidates {
            validate_image_candidate(&format!("{label}.{name}"), candidate)?;
        }
    }
    for map in &annotations.maps {
        if map.entity_id.trim().is_empty() || map.label.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                format!("{label}.maps"),
                "entity_id and label are required",
            ));
        }
        if !map.latitude.is_finite()
            || !map.longitude.is_finite()
            || !(-90.0..=90.0).contains(&map.latitude)
            || !(-180.0..=180.0).contains(&map.longitude)
        {
            return Err(EditorialPlanError::invalid(
                format!("{label}.maps"),
                "coordinates are not valid WGS84 degrees",
            ));
        }
        if map.start_us < 0 || map.duration_us < 0 {
            return Err(EditorialPlanError::invalid(
                format!("{label}.maps"),
                "negative map window",
            ));
        }
    }
    for group in &annotations.multi_entity_groups {
        if group.group_id.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                format!("{label}.multi_entity_groups"),
                "group_id is required",
            ));
        }
        if !group.scene_id.is_empty() && group.scene_id != scene_id {
            return Err(EditorialPlanError::invalid(
                format!("{label}.multi_entity_groups"),
                format!("group {:?} belongs to another scene", group.group_id),
            ));
        }
        if group.end_ms < group.start_ms {
            return Err(EditorialPlanError::invalid(
                format!("{label}.multi_entity_groups"),
                "group end precedes start",
            ));
        }
    }
    let window_start = scene.timeline_start_us;
    let window_end = window_start.saturating_add(scene.duration_us);
    for (name, spans) in [
        ("phrases", &annotations.phrases),
        ("keywords", &annotations.keywords),
        ("numbers", &annotations.numbers),
        ("chart_numbers", &annotations.chart_numbers),
        ("brand_texts", &annotations.brand_texts),
        ("quotes", &annotations.quotes),
    ] {
        for span in spans {
            if span.duration_us > 0
                && scene.duration_us > 0
                && (span.start_us < window_start
                    || span.start_us.saturating_add(span.duration_us) > window_end)
            {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    format!("annotation {:?} escapes the owning scene window", span.text),
                ));
            }
        }
    }
    if annotations.end_us < 0 {
        return Err(EditorialPlanError::invalid(
            format!("{label}.end_us"),
            "must not be negative",
        ));
    }
    if annotations.end_us > 0 && scene.duration_us > 0 && annotations.end_us > window_end {
        return Err(EditorialPlanError::invalid(
            format!("{label}.end_us"),
            "ends past the owning scene window",
        ));
    }
    Ok(())
}

impl EditorialManifestV2 {
    fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != EDITORIAL_MANIFEST_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: EDITORIAL_MANIFEST_SCHEMA_VERSION.to_string(),
            });
        }
        if !self
            .profile_version
            .eq_ignore_ascii_case(EDITORIAL_PROFILE_VERSION)
        {
            return Err(EditorialPlanError::invalid(
                "editorial.profile_version",
                format!("unsupported profile {:?}", self.profile_version),
            ));
        }
        if !is_lower_hex_64(&self.fingerprint) {
            return Err(EditorialPlanError::invalid(
                "editorial.fingerprint",
                "must be 64 lowercase hex characters",
            ));
        }
        let mut scene_ids = BTreeSet::new();
        for scene in &self.scenes {
            let id = scene.scene_id.trim();
            if id.is_empty() {
                return Err(EditorialPlanError::invalid(
                    "editorial.scenes",
                    "scene_id is required",
                ));
            }
            if !scene_ids.insert(id.to_string()) {
                return Err(EditorialPlanError::invalid(
                    "editorial.scenes",
                    format!("duplicate editorial scene {id:?}"),
                ));
            }
            for bullet in &scene.bullets {
                if bullet.sentence_start < 0 || bullet.sentence_end <= bullet.sentence_start {
                    return Err(EditorialPlanError::invalid(
                        format!("editorial.scenes.{id}.bullets"),
                        "invalid sentence span",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ChapterManifestV2 {
    fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != CHAPTER_MANIFEST_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: CHAPTER_MANIFEST_SCHEMA_VERSION.to_string(),
            });
        }
        for chapter in &self.chapters {
            for bullet in &chapter.bullets {
                if bullet.text.trim().is_empty() {
                    return Err(EditorialPlanError::invalid(
                        "chapters.bullets",
                        "chapter bullet text is required",
                    ));
                }
                if bullet.start_sentence < 0 || bullet.end_sentence <= bullet.start_sentence {
                    return Err(EditorialPlanError::invalid(
                        "chapters.bullets",
                        "invalid sentence span",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_categories(
    label: &str,
    categories: &BTreeMap<String, String>,
) -> Result<(), EditorialPlanError> {
    for (category, toggle) in categories {
        if !OVERLAY_CATEGORIES.contains(&category.as_str()) {
            return Err(EditorialPlanError::invalid(
                format!("{label}.overlay_categories"),
                format!("unknown overlay category {category:?}"),
            ));
        }
        if !matches!(toggle.as_str(), "" | "default" | "enabled" | "disabled") {
            return Err(EditorialPlanError::invalid(
                format!("{label}.overlay_categories.{category}"),
                format!("invalid toggle {toggle:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_timing(scene_id: &str, timing: &TimingV2) -> Result<(), EditorialPlanError> {
    let label = format!("scenes.{scene_id}.voiceover.timing");
    if timing.duration_us <= 0 {
        return Err(EditorialPlanError::invalid(
            label,
            "duration must be positive",
        ));
    }
    if timing.words.is_empty() {
        return Err(EditorialPlanError::invalid(label, "has no words"));
    }
    let mut previous_end = 0_i64;
    for (position, word) in timing.words.iter().enumerate() {
        if word.index != position as i64 {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word index {} at position {position}", word.index),
            ));
        }
        if word.start_us < 0 || word.end_us < word.start_us {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word {position} has an invalid range"),
            ));
        }
        if position > 0 && word.start_us < previous_end {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word {position} is not monotonic"),
            ));
        }
        if word.end_us > timing.duration_us {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word {position} exceeds the audio duration"),
            ));
        }
        previous_end = word.end_us;
    }
    Ok(())
}

/// True for a lowercase 64-character SHA-256 spelling. Anything else is a
/// different identity and is rejected rather than normalized.
pub fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
