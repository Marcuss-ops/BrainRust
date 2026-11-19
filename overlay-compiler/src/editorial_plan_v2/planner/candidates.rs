//! Candidate construction: the plan items built from the request
//! (scene/recap/entity-card/editorial/annotation lanes) before admission.
//! Item constructors live here with their motion pools, so the admission and
//! presentation stages cannot grow their own copy of the same defaults.

use super::super::date_motion_family::DateMotionFamily;
use super::*;

pub(super) fn build_base_candidates(
    request: &EditorialPlanV2Request,
    editorial: &super::super::response::EditorialPlanV2Response,
) -> Result<Vec<PlanItemV2>, EditorialPlanError> {
    let mut items = Vec::new();
    for scene in &request.scenes {
        if scene.duration_us <= 0 {
            continue;
        }
        append_editorial(&mut items, scene, editorial);
        if let Some(annotations) = &scene.annotations {
            append_annotations(&mut items, scene, annotations, request);
        }
    }
    Ok(items)
}

pub(super) struct RecapCandidates {
    pub(super) items: Vec<PlanItemV2>,
    pub(super) placements: i64,
}

/// Ports the Automatic Recap Builder over the projected scene briefs: sources
/// are strictly before the display scene, score-ordered, title-deduplicated
/// and never allowed to leak a later chapter into an earlier card.
pub(super) fn build_recap_candidates(
    request: &EditorialPlanV2Request,
) -> Result<RecapCandidates, EditorialPlanError> {
    if request.canvas.disable_recap_overlays || request.recap_briefs.len() < 3 {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let display = (request.recap_briefs.len() / 2 + 1).min(request.recap_briefs.len() - 1);
    if display < 1 {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let mut sources = request.recap_briefs[..display]
        .iter()
        .filter(|brief| !brief.title.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut seen = BTreeSet::new();
    sources.retain(|brief| seen.insert(name_slug(&brief.title)));
    sources.truncate(3);
    if sources.is_empty() {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let window = &request.recap_briefs[display];
    if window.end_us <= window.start_us {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let Some(scene) = request.scenes.iter().find(|scene| scene.id == window.id) else {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    };
    let mut cursor = window.start_us;
    let mut items = Vec::new();
    let mut used_ids = BTreeSet::new();
    for source in sources {
        let remaining = window.end_us.saturating_sub(cursor);
        if remaining < 2_000_000 {
            break;
        }
        let duration = remaining.min(4_000_000);
        let base = format!("{}-recap-{}", scene.id, name_slug(&source.title));
        let mut id = base.clone();
        let mut suffix = 2;
        while !used_ids.insert(id.clone()) {
            id = format!("{base}-{suffix}");
            suffix += 1;
        }
        let mut item = scene_item(
            id,
            scene,
            "quote",
            "QUOTE",
            cursor,
            duration,
            source.title.clone(),
        );
        item.preset_id = "phrase_default".to_string();
        item.params = Some(json!({
            "position": "center",
            "style": "quote",
            "priority": source.score,
            "recap": true,
        }));
        items.push(item);
        cursor = cursor.saturating_add(duration);
    }
    Ok(RecapCandidates {
        placements: if items.is_empty() { 0 } else { 1 },
        items,
    })
}

pub(super) fn build_entity_card_candidates(
    request: &EditorialPlanV2Request,
) -> Result<Vec<PlanItemV2>, EditorialPlanError> {
    let mut items = Vec::new();
    append_entity_cards(&mut items, request)?;
    Ok(items)
}

fn append_editorial(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    editorial: &super::super::response::EditorialPlanV2Response,
) {
    let Some(phrases) = editorial.scene_phrases.get(&scene.id) else {
        return;
    };
    let title = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "scene_title");
    let bullet = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "scene_bullet");
    let chapter = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "chapter_bullet");
    for (ordinal, phrase) in [title, bullet, chapter].into_iter().flatten().enumerate() {
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

fn append_annotations(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    annotations: &super::super::request::SceneAnnotationsV2,
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
enum NumericLane {
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

fn number_item(
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

fn date_motion_pool(request: &EditorialPlanV2Request) -> Vec<&'static str> {
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
pub(super) struct PresetSeed<'a> {
    pub(super) job_fingerprint: &'a str,
    pub(super) scene_id: &'a str,
    pub(super) semantic_id: &'a str,
    pub(super) preset_family: &'a str,
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
/// motion. Scene and chapter bullets keep their own pin contract and are left
/// alone, and a phrase longer than five words belongs to the long lane.
pub(super) fn assign_short_phrase_motions(
    items: &mut [PlanItemV2],
    request: &EditorialPlanV2Request,
) {
    let canvas = &request.canvas;
    let pin = canvas.short_phrase_motion.trim();
    // One cursor per word-count bucket plus one plan-wide used set. The old
    // independent hash per item could pick the same gesture twice (and never
    // guaranteed that the ten certified typewriter gestures appeared at all).
    // Order is still deterministic: the bucket rotates from a plan-stable
    // offset, and the used set only skips a gesture already on screen.
    let mut next_index: BTreeMap<usize, usize> = BTreeMap::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for item in items.iter_mut() {
        if item.kind != "text_phrase" || item.template_id != "IMPORTANT_PHRASE" {
            continue;
        }
        if !item.motion_id.trim().is_empty() || is_editorial_bullet(item.params.as_ref()) {
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
        let ordered = ordered_short_phrase_pool(&bucket, &request.plan.plan_id, words);
        let start = *next_index.entry(words).or_insert(0);
        let mut chosen = None;
        for step in 0..ordered.len() {
            let index = (start + step) % ordered.len();
            let candidate = ordered[index];
            if !used.contains(candidate) {
                chosen = Some((index, candidate.to_string()));
                break;
            }
        }
        // Every candidate was already used in this plan (a pool smaller than
        // the phrase count): keep rotating deterministically instead of
        // leaving the renderer without an entrance.
        let (index, motion) = chosen.unwrap_or_else(|| {
            let index = start % ordered.len();
            (index, ordered[index].to_string())
        });
        used.insert(motion.clone());
        next_index.insert(words, index + 1);
        item.motion_id = motion;
    }
}

/// Orders one short-phrase bucket for an actual run of phrases.
///
/// The certified typewriter family leads the rotation, so the first ten short
/// phrases of a plan really show the ten distinct gestures the catalog and the
/// runtime both certify. The legacy short gestures stay in the pool and appear
/// after the typewriter block, so this narrows no capability. The typewriter
/// block is rotated by a plan-stable offset: every run starts on a different
/// gesture, but replaying the same plan reproduces the same order.
fn ordered_short_phrase_pool<'a>(pool: &[&'a str], plan_id: &str, words: usize) -> Vec<&'a str> {
    let mut typewriter: Vec<&str> = pool
        .iter()
        .copied()
        .filter(|id| id.starts_with("typewriter_"))
        .collect();
    let typewriter_count = typewriter.len();
    if typewriter_count > 0 {
        let offset = seeded_index(
            plan_id,
            "",
            &words.to_string(),
            "short_phrase_typewriter_rotation",
            typewriter_count,
        )
        .unwrap_or(0);
        typewriter.rotate_left(offset % typewriter_count);
    }
    typewriter.extend(
        pool.iter()
            .copied()
            .filter(|id| !id.starts_with("typewriter_")),
    );
    typewriter
}

/// Rotates the long-phrase entrance pool for every 6+ word phrase. The long
/// lane has no pin and no per-length buckets: the only requirement is that two
/// long phrases in the same plan do not share the presentation default, which
/// is what made every long phrase look identical.
pub(super) fn assign_long_phrase_motions(
    items: &mut [PlanItemV2],
    request: &EditorialPlanV2Request,
) {
    let pool: Vec<&str> = request
        .canvas
        .long_phrase_motions
        .iter()
        .map(String::as_str)
        .collect();
    if pool.is_empty() {
        return;
    }
    for item in items.iter_mut() {
        if item.kind != "text_phrase" || item.template_id != "IMPORTANT_PHRASE" {
            continue;
        }
        if !item.motion_id.trim().is_empty() || is_editorial_bullet(item.params.as_ref()) {
            continue;
        }
        if word_count(&item.text) < 6 {
            continue;
        }
        if let Some(motion) = seeded_preset(
            &request.plan.plan_id,
            &item.scene_id,
            &item.id,
            "long_phrase_presentation",
            &pool,
        ) {
            item.motion_id = motion;
        }
    }
}

fn is_editorial_bullet(params: Option<&Value>) -> bool {
    params
        .and_then(Value::as_object)
        .and_then(|object| object.get("editorial_role"))
        .and_then(Value::as_str)
        .is_some_and(|role| matches!(role, "scene_bullet" | "chapter_bullet"))
}

/// Seed index shared by every deterministic presentation sampler. It mirrors
/// Go's `digest.SHA256Bytes`: the first eight ASCII hex bytes form a
/// big-endian integer, modulo the candidate count.
fn seeded_index(
    job_id: &str,
    scene_id: &str,
    item_id: &str,
    family: &str,
    len: usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let encoded = serde_json::to_vec(&PresetSeed {
        job_fingerprint: job_id,
        scene_id,
        semantic_id: item_id,
        preset_family: family,
    })
    .ok()?;
    let hex = format!("{:x}", Sha256::digest(encoded));
    let bytes: [u8; 8] = hex.as_bytes().get(..8)?.try_into().ok()?;
    Some((u64::from_be_bytes(bytes) % len as u64) as usize)
}

pub(super) fn seeded_preset(
    job_id: &str,
    scene_id: &str,
    item_id: &str,
    family: &str,
    candidates: &[&str],
) -> Option<String> {
    let index = seeded_index(job_id, scene_id, item_id, family, candidates.len())?;
    Some(candidates[index].to_string())
}

/// Go's `itemID`: lowercase the concatenated identity, collapse every run of
/// non-letter/non-digit characters to one hyphen and cap the result at 110
/// runes with a stable hash suffix.
pub(super) fn go_item_id(scene_id: &str, kind: &str, value: &str) -> String {
    let raw = format!("{scene_id}-{kind}-{value}").to_lowercase();
    let mut id = String::new();
    let mut separator = false;
    for ch in raw.chars() {
        if ch.is_alphanumeric() {
            if separator && !id.is_empty() {
                id.push('-');
            }
            id.push(ch);
            separator = false;
        } else {
            separator = true;
        }
    }
    let id = id.trim_matches('-').to_string();
    if id.chars().count() <= 110 {
        return id;
    }
    let head: String = id.chars().take(110).collect();
    let sum = format!("{:x}", Sha256::digest(raw.as_bytes()));
    format!("{}-{}", head.trim_end_matches('-'), &sum[..12])
}

/// Go's `limitedShuffledMotionPool`: a Fisher-Yates shuffle seeded by raw
/// SHA-256 bytes over `seed + NUL + rune(i)`, then capped to `limit`.
fn limited_shuffled_pool(pool: &[&'static str], limit: usize, seed: &str) -> Vec<&'static str> {
    let mut order = pool.to_vec();
    for i in (1..order.len()).rev() {
        let digest = Sha256::digest(format!(
            "{seed}\x00{}",
            char::from_u32(i as u32).unwrap_or('\0')
        ));
        let bytes: [u8; 8] = digest[..8].try_into().expect("sha256 prefix");
        let j = (u64::from_be_bytes(bytes) % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order.truncate(limit.min(order.len()));
    order
}

fn text_item(
    scene: &SceneV2,
    lane: &str,
    index: usize,
    annotation: &TimedAnnotationV2,
    kind: &str,
    template: &str,
) -> PlanItemV2 {
    let mut item = scene_item(
        format!("{}-{lane}-{index}", scene.id),
        scene,
        kind,
        template,
        annotation.start_us,
        annotation.duration_us,
        annotation.text.clone(),
    );
    item.params = Some(json!({"priority": annotation.score}));
    item
}

pub(super) fn scene_item(
    id: String,
    scene: &SceneV2,
    kind: &str,
    template: &str,
    start_us: i64,
    duration_us: i64,
    text: String,
) -> PlanItemV2 {
    let start_us = start_us.max(scene.timeline_start_us);
    let end_us = start_us
        .saturating_add(duration_us.max(0))
        .min(scene.timeline_start_us.saturating_add(scene.duration_us));
    let duration_us = (end_us - start_us).max(0);
    PlanItemV2 {
        id,
        scene_id: scene.id.clone(),
        kind: kind.to_string(),
        template_id: template.to_string(),
        start_us,
        duration_us,
        start_ms: start_us / 1000,
        end_ms: (start_us + duration_us + 999) / 1000,
        text,
        ..Default::default()
    }
}

pub(super) fn trim_maps(request: &EditorialPlanV2Request, items: &mut [PlanItemV2]) {
    if !items.iter().any(|item| item.map.is_some()) {
        return;
    }
    let input: Vec<TimingItem> = items.iter().map(timing_item).collect();
    let edits = crate::trim_map_windows_before_budget(crate::ResolveMapOverlapsRequest {
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        scene_windows: request.scene_windows.clone(),
        items: input,
        map_presentation: None,
    });
    for edit in edits {
        if let Some(item) = items.get_mut(edit.index) {
            item.start_us = edit.start_us;
            item.duration_us = edit.duration_us;
            item.start_ms = edit.start_ms;
            item.end_ms = edit.end_ms;
        }
    }
}

pub(super) fn timing_item(item: &PlanItemV2) -> TimingItem {
    TimingItem {
        id: item.id.clone(),
        scene_id: item.scene_id.clone(),
        kind: item.kind.clone(),
        template_id: item.template_id.clone(),
        start_ms: item.start_ms,
        end_ms: item.end_ms,
        start_us: item.start_us,
        duration_us: item.duration_us,
        has_map: item.map.is_some(),
        params: item.params.clone(),
    }
}

/// Generated documentary cards keep the name below the portrait. The generic
/// `random` selector can sample a side composition, so a captioned card is
/// pinned to the below-caption pool exactly like Go's presentation pass.
/// Certified date-card and metric-card pools, in the registration order the
/// Go accessors expose. Motion ids are renderer vocabulary, not presets.
pub(super) const DATE_MOTIONS: &[&str] = &[
    "date_fade_rise",
    "date_timeline_tick",
    "date_chronology_focus",
    "date_calendar_drop",
    "date_month_wipe",
    "date_era_zoom",
    "date_bracket_draw",
    "date_didone_archive_fade",
    "date_didone_chronicle_rise",
    "date_didone_era_settle",
    "date_didone_fade_tracking",
    "date_didone_focus_hold",
    "date_didone_margin_wipe",
    "date_didone_rule_draw",
    "date_didone_stamp_drop",
    "date_didone_underline_hold",
    "date_didone_year_slide",
    "date_digit_flip",
    "date_history_stack",
    "date_marker_drop",
    "date_marker_pop",
    "date_page_turn",
    "date_range_draw",
    "date_segment_stagger",
    "date_split_year",
    "date_stamp_reveal",
    "date_timeline_sweep",
    "date_typewriter_bracket_tap",
    "date_typewriter_day_stamp",
    "date_typewriter_era_carriage",
    "date_typewriter_ink_stamp",
    "date_typewriter_margin_return",
    "date_typewriter_month_strike",
    "date_typewriter_range_type",
    "date_typewriter_timeline_feed",
    "date_typewriter_underline_key",
    "date_typewriter_year_type",
    "date_underline_focus",
    "date_year_count",
];

pub(super) const METRIC_MOTIONS: &[&str] = &[
    "metric_counter_scale_settle",
    "metric_odometer_vertical",
    "metric_digits_stagger",
    "metric_delta_reveal",
    "metric_focus_punch",
    "metric_before_after",
    "metric_count_flip",
    "metric_split_odometer",
    "metric_arc_sweep",
    "metric_bar_grow",
    "metric_bounce_settle",
    "metric_compare_wipe",
    "metric_counter_rise",
    "metric_debt_flip",
    "metric_didone_column_rise",
    "metric_didone_drop_in",
    "metric_didone_focus_pull",
    "metric_didone_hairline_rise",
    "metric_didone_hold_breathe",
    "metric_didone_ink_settle",
    "metric_didone_numeral_settle",
    "metric_didone_rule_wipe",
    "metric_didone_side_slide",
    "metric_didone_tracking_open",
    "metric_digit_cascade",
    "metric_multi_stat_focus",
    "metric_pulse_hold",
    "metric_ribbon_unfold",
    "metric_ring_draw",
    "metric_slide_left",
    "metric_typewriter_backspace_settle",
    "metric_typewriter_carriage_slide",
    "metric_typewriter_cursor_in",
    "metric_typewriter_digit_tick",
    "metric_typewriter_ink_stamp",
    "metric_typewriter_key_strike",
    "metric_typewriter_letter_drop",
    "metric_typewriter_line_feed",
    "metric_typewriter_ribbon_strike",
    "metric_typewriter_stepped_reveal",
];

#[cfg(test)]
mod tests {
    use super::{DateMotionFamily, DATE_MOTIONS};

    #[test]
    fn every_date_motion_belongs_to_exactly_one_family() {
        let families = [
            DateMotionFamily::Classic,
            DateMotionFamily::Didone,
            DateMotionFamily::Typewriter,
        ];
        for motion in DATE_MOTIONS {
            let matches = families
                .iter()
                .filter(|family| family.allows(motion))
                .count();
            assert_eq!(
                1, matches,
                "{motion} must belong to exactly one family, matched {matches}"
            );
        }
    }

    #[test]
    fn family_pools_partition_the_catalogue() {
        let count = |family: DateMotionFamily| {
            DATE_MOTIONS
                .iter()
                .filter(|motion| family.allows(motion))
                .count()
        };
        assert_eq!(
            DATE_MOTIONS.len(),
            count(DateMotionFamily::Classic)
                + count(DateMotionFamily::Didone)
                + count(DateMotionFamily::Typewriter)
        );
    }
}
