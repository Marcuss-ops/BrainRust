use super::*;

impl CanvasV2 {
    pub(super) fn validate(&self) -> Result<(), EditorialPlanError> {
        validate_categories("canvas", &self.overlay_categories)?;
        for (name, limit) in &self.group_limits {
            if name.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "canvas.group_limits",
                    "empty group name",
                ));
            }
            if *limit < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("canvas.group_limits.{name}"),
                    "must not be negative",
                ));
            }
        }
        for (lane, count) in &self.animation_counts {
            if lane.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "canvas.animation_counts",
                    "empty lane name",
                ));
            }
            if *count < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("canvas.animation_counts.{lane}"),
                    "must not be negative",
                ));
            }
        }
        for (field, pool) in [
            ("image_motions", &self.image_motions),
            ("image_with_text_motions", &self.image_with_text_motions),
            ("entity_image_motions", &self.entity_image_motions),
            ("entity_caption_motions", &self.entity_caption_motions),
        ] {
            validate_motion_pool(field, pool)?;
        }
        for (words, pool) in &self.short_phrase_motions_by_word {
            let field = format!("short_phrase_motions_by_word.{words}");
            validate_motion_pool(&field, pool)?;
        }
        validate_motion_pool("long_phrase_motions", &self.long_phrase_motions)?;
        if self.image_motion_offset < 0 {
            return Err(EditorialPlanError::invalid(
                "canvas.image_motion_offset",
                "must not be negative",
            ));
        }
        if !self.entity_style_id.trim().is_empty()
            && !is_certified_entity_style_selector(&self.entity_style_id)
        {
            return Err(EditorialPlanError::invalid(
                "canvas.entity_style_id",
                format!(
                    "{:?} is not a certified entity-style selector",
                    self.entity_style_id
                ),
            ));
        }
        if !self.entity_badge_color.trim().is_empty() && !is_hex_rgb(&self.entity_badge_color) {
            return Err(EditorialPlanError::invalid(
                "canvas.entity_badge_color",
                "must be a #RRGGBB hex color",
            ));
        }
        if DateMotionFamily::parse(&self.date_motion_family).is_none() {
            return Err(EditorialPlanError::invalid(
                "canvas.date_motion_family",
                format!("unsupported family {:?}", self.date_motion_family),
            ));
        }
        for (field, value) in [
            ("max_overlays_per_scene", self.max_overlays_per_scene),
            ("max_phrase_overlays", self.max_phrase_overlays),
            ("max_image_overlays", self.max_image_overlays),
            ("max_map_overlays", self.max_map_overlays),
            (
                "map_presentation.max_pins_per_group",
                self.map_presentation.max_pins_per_group,
            ),
        ] {
            if value < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("canvas.{field}"),
                    "must not be negative",
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn is_hex_rgb(value: &str) -> bool {
    let value = value.trim();
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
}

/// The closed set of `entity_style_id` selector words the pipeline may put on
/// a plan item. Mirrors `overlays.CertifiedEntityStyleSelectors`.
const CERTIFIED_ENTITY_STYLE_SELECTORS: [&str; 10] = [
    "random",
    "premium_random_v1",
    "testo_sotto",
    "below",
    "premium_below_random_v1",
    "caption_below",
    "badge",
    "badge_random",
    "camera",
    "camera_random",
];

/// Legacy Apple Spatial composition ids still transportable by channel
/// profiles (01–05 centered, 06–10 retired side layouts, 11–15 typewriter,
/// 16–20 badges, 21–25 camera moves). Mirrors
/// `overlays.CertifiedAppleSpatialEntityStyles`.
const CERTIFIED_APPLE_SPATIAL_ENTITY_STYLES: [&str; 25] = [
    "01_entity_pitch_lift_text_below",
    "02_entity_yaw_flip_text_below",
    "03_entity_pop_bounce_text_below",
    "04_entity_card_swing_text_below",
    "05_entity_depth_float_text_below",
    "06_entity_swipe_text_right",
    "07_entity_orbit_text_left",
    "08_entity_counter_tilt_text_right",
    "09_entity_dolly_settle_text_left",
    "10_entity_parallax_drift_text_right",
    "11_entity_typewriter_classic",
    "12_entity_terminal_cyber",
    "13_entity_typewriter_editorial_gold",
    "14_entity_code_prompt_reveal",
    "15_entity_typewriter_dolly_breath",
    "16_entity_badge_yellow_wipe",
    "17_entity_badge_red_impact",
    "18_entity_badge_cyan_electric",
    "19_entity_badge_orange_amber",
    "20_entity_badge_green_emerald",
    "21_entity_camera_dolly_push",
    "22_entity_camera_orbit_yaw",
    "23_entity_camera_crane_rise",
    "24_entity_camera_dutch_spatial",
    "25_entity_camera_flyby_parallax",
];

/// True when the request may transport this selector. The renderer owns the
/// live composition registry; the planner only validates the vocabulary.
pub(super) fn is_certified_entity_style_selector(style_id: &str) -> bool {
    let normalized = style_id.trim().to_ascii_lowercase();
    CERTIFIED_ENTITY_STYLE_SELECTORS.contains(&normalized.as_str())
        || CERTIFIED_APPLE_SPATIAL_ENTITY_STYLES.contains(&normalized.as_str())
}

pub(super) fn validate_motion_pool(field: &str, pool: &[String]) -> Result<(), EditorialPlanError> {
    let mut seen = BTreeSet::new();
    for motion in pool {
        let motion = motion.trim();
        if motion.is_empty() {
            return Err(EditorialPlanError::invalid(
                format!("canvas.{field}"),
                "motion id is empty",
            ));
        }
        if !seen.insert(motion) {
            return Err(EditorialPlanError::invalid(
                format!("canvas.{field}"),
                format!("duplicate motion id {motion:?}"),
            ));
        }
    }
    Ok(())
}

impl BudgetsV2 {
    pub(super) fn validate(&self) -> Result<(), EditorialPlanError> {
        for (field, value) in [
            ("phrase_overlays", self.phrase_overlays),
            ("image_overlays", self.image_overlays),
            ("map_overlays", self.map_overlays),
            ("max_per_scene", self.max_per_scene),
            ("short_phrase", self.short_phrase),
            ("long_phrase", self.long_phrase),
            ("bullet", self.bullet),
        ] {
            if value < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("budgets.{field}"),
                    "must not be negative",
                ));
            }
        }
        Ok(())
    }
}

impl EntityTimelineV2 {
    pub(super) fn validate(&self, known_scenes: &BTreeSet<&str>) -> Result<(), EditorialPlanError> {
        if self.version < 1 {
            return Err(EditorialPlanError::invalid(
                "entity_timeline.version",
                "must be at least 1",
            ));
        }
        if self.duration_us < 0 {
            return Err(EditorialPlanError::invalid(
                "entity_timeline.duration_us",
                "must not be negative",
            ));
        }
        let mut scene_ids = BTreeSet::new();
        let mut previous_index = -1_i64;
        for scene in &self.scenes {
            if scene.scene_id.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    "scene id is required",
                ));
            }
            if !scene_ids.insert(scene.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    format!("duplicate scene {:?}", scene.scene_id),
                ));
            }
            if !known_scenes.contains(scene.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    format!("unknown scene {:?}", scene.scene_id),
                ));
            }
            if scene.scene_index <= previous_index {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    "scenes are not strictly ordered by index",
                ));
            }
            previous_index = scene.scene_index;
            if scene.timeline_start_us < 0 {
                return Err(EditorialPlanError::invalid(
                    format!(
                        "entity_timeline.scenes.{}.timeline_start_us",
                        scene.scene_id
                    ),
                    "must not be negative",
                ));
            }
            for occurrence in &scene.entities {
                occurrence.validate(&scene.scene_id, scene.timeline_start_us, self.duration_us)?;
            }
        }
        Ok(())
    }
}

impl EntityOccurrenceV2 {
    pub(super) fn validate(
        &self,
        scene_id: &str,
        scene_timeline_start_us: i64,
        timeline_duration_us: i64,
    ) -> Result<(), EditorialPlanError> {
        let label = format!("entity_timeline entity {:?}", self.name);
        if self.entity_id.trim().is_empty()
            || self.name.trim().is_empty()
            || self.entity_type.trim().is_empty()
        {
            return Err(EditorialPlanError::invalid(
                label,
                "entity_id, name and type are required",
            ));
        }
        if !self.scene_id.is_empty() && self.scene_id != scene_id {
            return Err(EditorialPlanError::invalid(
                label,
                format!("occurrence scene {:?} does not own it", self.scene_id),
            ));
        }
        if self.text_start < 0 || self.text_end <= self.text_start {
            return Err(EditorialPlanError::invalid(label, "invalid text span"));
        }
        if self.word_start < 0 || self.word_end < self.word_start {
            return Err(EditorialPlanError::invalid(label, "invalid word span"));
        }
        if self.local_start_us < 0 || self.local_end_us < self.local_start_us {
            return Err(EditorialPlanError::invalid(label, "inverted local span"));
        }
        if self.timeline_start_us != scene_timeline_start_us {
            return Err(EditorialPlanError::invalid(
                label,
                "carries a different timeline offset than its scene",
            ));
        }
        if self.audio_start_us != self.timeline_start_us + self.local_start_us
            || self.audio_end_us != self.timeline_start_us + self.local_end_us
        {
            return Err(EditorialPlanError::invalid(
                label,
                "audio window is not the canonical local->global mapping",
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(EditorialPlanError::invalid(
                label,
                "confidence is outside [0,1]",
            ));
        }
        if self.audio_end_us > timeline_duration_us {
            return Err(EditorialPlanError::invalid(
                label,
                "audio window ends past the timeline duration",
            ));
        }
        Ok(())
    }
}

impl MediaV2 {
    pub(super) fn validate(&self, known_scenes: &BTreeSet<&str>) -> Result<(), EditorialPlanError> {
        for asset in &self.materialized_assets {
            validate_asset_ref("media.materialized_assets", asset)?;
        }
        for verified in &self.verified_entity_assets {
            if verified.entity_id.trim().is_empty()
                && verified.canonical_entity_id.trim().is_empty()
            {
                return Err(EditorialPlanError::invalid(
                    "media.verified_entity_assets",
                    "entity_id or canonical_entity_id is required",
                ));
            }
            for asset in &verified.assets {
                validate_asset_ref("media.verified_entity_assets", asset)?;
            }
        }
        let mut seen_scenes = BTreeSet::new();
        for set in &self.scene_image_candidates {
            if !known_scenes.contains(set.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "media.scene_image_candidates",
                    format!("unknown scene {:?}", set.scene_id),
                ));
            }
            if !seen_scenes.insert(set.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "media.scene_image_candidates",
                    format!("duplicate scene {:?}", set.scene_id),
                ));
            }
            for candidate in &set.candidates {
                validate_image_candidate("media.scene_image_candidates", candidate)?;
            }
        }
        Ok(())
    }
}
