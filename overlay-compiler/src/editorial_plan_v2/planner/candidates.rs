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
            let first = items.len();
            append_annotations(&mut items, scene, annotations, request);
            super::visual_recipes::apply_visual_recipes(
                &mut items,
                scene,
                annotations,
                request,
                first,
            );
        }
    }
    // Budget text while the candidates are still a flat list: each fragment
    // then passes admission and can never enter a fused chain as one oversized
    // segment. The presentation split remains only a defensive invariant.
    Ok(super::split_long_text_candidates(items))
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
    decisions: &mut Vec<PlanDecisionV2>,
) -> Result<Vec<PlanItemV2>, EditorialPlanError> {
    let mut items = Vec::new();
    append_entity_cards(&mut items, request, decisions)?;
    Ok(items)
}

/// Drops a plain text phrase that repeats a numeric card already admitted with
/// the same surface and an overlapping window. `13 settembre 2026` used to
/// arrive twice at the same instant: once as `TIMELINE_DATE_CARD` and once as
/// a typewriter `text_phrase`. The numeric card owns the fact and its readable
/// hold; the duplicate phrase would only stack a second copy on top of it.
///
/// The pass runs after admission so a numeric card that lost its budget slot
/// never takes the phrase down with it. Editorial titles and bullets keep their
/// authored role and are never treated as a duplicate.
pub(super) fn drop_text_phrases_shadowed_by_numeric_cards(
    items: Vec<PlanItemV2>,
) -> Vec<PlanItemV2> {
    let numeric: Vec<(String, i64, i64)> = items
        .iter()
        .filter(|item| {
            matches!(
                item.template_id.as_str(),
                "TIMELINE_DATE_CARD" | "METRIC_STAT_CARD"
            )
        })
        .filter_map(|item| {
            let key = normalized_overlay_surface(&item.text);
            let end = item.start_us.saturating_add(item.duration_us);
            (!key.is_empty() && end > item.start_us).then_some((key, item.start_us, end))
        })
        .collect();
    if numeric.is_empty() {
        return items;
    }
    items
        .into_iter()
        .filter(|item| {
            if item.kind != "text_phrase" || has_editorial_role(item) {
                return true;
            }
            let key = normalized_overlay_surface(&item.text);
            if key.is_empty() {
                return true;
            }
            let end = item.start_us.saturating_add(item.duration_us);
            !numeric.iter().any(|(numeric_key, start, numeric_end)| {
                *numeric_key == key && item.start_us < *numeric_end && *start < end
            })
        })
        .collect()
}

fn has_editorial_role(item: &PlanItemV2) -> bool {
    item.params
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|params| params.get("editorial_role"))
        .and_then(Value::as_str)
        .is_some_and(|role| !role.is_empty())
}

fn normalized_overlay_surface(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

mod editorial;
pub(crate) use editorial::*;
/// Interleaves the certified motion families so short runs sample more than
/// the first family in the registry. Every family and member order is stable.
fn ordered_short_phrase_pool<'a>(pool: &[&'a str], seed: &str, words: usize) -> Vec<&'a str> {
    let mut families: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for id in pool.iter().copied() {
        families
            .entry(phrase_motion_family(id).to_string())
            .or_default()
            .push(id);
    }
    for (family, motions) in families.iter_mut() {
        if let Some(offset) = seeded_index(seed, "", &words.to_string(), family, motions.len()) {
            motions.rotate_left(offset);
        }
    }
    let names: Vec<String> = families.keys().cloned().collect();
    let family_offset = seeded_index(
        seed,
        "",
        &words.to_string(),
        "short_phrase_family_order",
        names.len(),
    )
    .unwrap_or(0);
    let mut ordered = Vec::with_capacity(pool.len());
    let mut indexes = vec![0; names.len()];
    loop {
        let mut added = false;
        for step in 0..names.len() {
            let index = (family_offset + step) % names.len();
            let motions = &families[&names[index]];
            if indexes[index] < motions.len() {
                ordered.push(motions[indexes[index]]);
                indexes[index] += 1;
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    ordered
}

fn phrase_motion_family(id: &str) -> &str {
    [
        "typewriter",
        "short_phrase_web",
        "short_phrase_product",
        "short_phrase_editorial",
        "classic_playfair",
        "doc",
        "phrase_apple_clean",
        "text_3d",
        "brush_phrase",
    ]
    .into_iter()
    .find(|prefix| id.starts_with(prefix))
    .unwrap_or("other")
}

fn family_matches(id: &str, requested: &str) -> bool {
    let requested = requested.trim().to_ascii_lowercase();
    if requested.is_empty() || requested == "auto" {
        return true;
    }
    let family = phrase_motion_family(id);
    let alias = if requested.contains("product") {
        "short_phrase_product"
    } else if requested.contains("apple") {
        "phrase_apple_clean"
    } else if requested.contains("typewriter") {
        "typewriter"
    } else if requested.contains("web") {
        "short_phrase_web"
    } else if requested.contains("editorial") {
        "short_phrase_editorial"
    } else if requested.contains("playfair") {
        "classic_playfair"
    } else if requested.contains("doc") {
        "doc"
    } else if requested.contains("3d") {
        "text_3d"
    } else if requested.contains("brush") {
        "brush_phrase"
    } else {
        requested.as_str()
    };
    family == alias
}

fn motion_seed(request: &EditorialPlanV2Request) -> &str {
    let seed = request.plan.motion_seed.trim();
    if seed.is_empty() {
        &request.plan.plan_id
    } else {
        seed
    }
}

/// Rotates the long-phrase entrance pool for every 6+ word phrase. The long
/// lane has no pin and no per-length buckets: the only requirement is that two
/// long phrases in the same plan do not share the presentation default, which
/// is what made every long phrase look identical. Editorial titles and bullets
/// over five words belong to this lane for the same reason as the short one: a
/// production plan normally pins only their font, not their entrance.
pub(super) fn assign_long_phrase_motions(
    items: &mut [PlanItemV2],
    request: &EditorialPlanV2Request,
) {
    let all_pool: Vec<&str> = request
        .canvas
        .long_phrase_motions
        .iter()
        .map(String::as_str)
        .collect();
    if all_pool.is_empty() {
        return;
    }
    let filtered: Vec<&str> = all_pool
        .iter()
        .copied()
        .filter(|id| family_matches(id, &request.canvas.phrase_motion_family))
        .collect();
    let pool = if filtered.is_empty() {
        all_pool
    } else {
        filtered
    };
    let mut ordered = Vec::with_capacity(pool.len());
    let mut seen = BTreeSet::new();
    for motion in pool {
        if seen.insert(motion) {
            ordered.push(motion);
        }
    }
    let seed = motion_seed(request);
    let mut used = BTreeSet::new();
    for item in items.iter_mut() {
        if item.kind != "text_phrase" || item.template_id != "IMPORTANT_PHRASE" {
            continue;
        }
        if !item.motion_id.trim().is_empty() || has_pinned_bullet_motion(request, item) {
            continue;
        }
        if word_count(&item.text) < 6 {
            continue;
        }
        let start = seeded_index(
            seed,
            &item.scene_id,
            &item.id,
            "long_phrase_motion",
            ordered.len(),
        )
        .unwrap_or(0);
        let motion = (0..ordered.len())
            .map(|step| ordered[(start + step) % ordered.len()])
            .find(|candidate| !used.contains(candidate))
            .unwrap_or(ordered[start]);
        used.insert(motion);
        item.motion_id = motion.to_string();
    }
}

/// Rotates the certified date and metric presentation pools for every numeric
/// card. The three certified families (classic, didone, typewriter) are
/// interleaved, so a short run samples a typewriter gesture within its first
/// few cards, and a plan-wide used set prevents repeating a motion until that
/// lane's pool is exhausted. Selection is seeded by the run's `motion_seed`,
/// so a replay reproduces the same order while each new run rotates.
pub(super) fn assign_numeric_motions(items: &mut [PlanItemV2], request: &EditorialPlanV2Request) {
    // The request's motion_seed is shared by runs that pin one entropy value.
    // Mixing in the plan id keeps that pin from freezing the numeric entrances
    // across different plans, while a replay of the same plan stays identical.
    let seed = format!("{}\x00{}", motion_seed(request), request.plan.plan_id);
    let seed = seed.as_str();
    let mut used: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    // A lane cursor walks the interleaved pool in reading order. A per-item
    // hash alone could land three times in the classic block before reaching
    // the typewriter family; the cursor guarantees one family per round.
    let mut cursors: BTreeMap<&'static str, usize> = BTreeMap::new();
    for item in items.iter_mut() {
        let lane = match item.template_id.as_str() {
            "TIMELINE_DATE_CARD" => NumericLane::Date,
            "METRIC_STAT_CARD" => NumericLane::Metric,
            _ => continue,
        };
        let Some((counts_key, family_key)) = lane.motion_lane() else {
            continue;
        };
        let mut pool = lane.motion_pool(request);
        if pool.is_empty() {
            continue;
        }
        let limit = request
            .canvas
            .animation_counts
            .get(counts_key)
            .copied()
            .unwrap_or(0)
            .max(0) as usize;
        if limit > 0 && limit < pool.len() {
            pool = limited_shuffled_pool(
                &pool,
                limit,
                &format!("{seed}\x00{}\x00{family_key}", request.plan.plan_id),
            );
        }
        let ordered = ordered_numeric_motion_pool(&pool, seed, family_key);
        if ordered.is_empty() {
            continue;
        }
        // Start on a family round boundary. A random offset inside the flat
        // list could land in the classic tail (classic owns 20 of the 39/40
        // motions), so three cards in a row could never reach typewriter.
        // The seed already rotates every family and the family order itself.
        let cursor = *cursors.entry(family_key).or_insert(0);
        let lane_used = used.entry(family_key).or_default();
        let motion_index = (0..ordered.len())
            .map(|step| (cursor + step) % ordered.len())
            .find(|index| !lane_used.contains(ordered[*index]))
            .unwrap_or(cursor % ordered.len());
        let motion = ordered[motion_index];
        lane_used.insert(motion.to_string());
        cursors.insert(family_key, (motion_index + 1) % ordered.len());
        item.motion_id = motion.to_string();
    }
}

/// Interleaves classic, didone and typewriter members so a three-card run owns
/// one entrance from each family instead of three neighbours from the same
/// family. Each family keeps its registry order, rotated by the run seed.
fn ordered_numeric_motion_pool<'a>(pool: &[&'a str], seed: &str, lane: &str) -> Vec<&'a str> {
    let mut families: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for id in pool.iter().copied() {
        families
            .entry(numeric_motion_family(id).to_string())
            .or_default()
            .push(id);
    }
    for (family, motions) in families.iter_mut() {
        if let Some(offset) = seeded_index(seed, "", lane, family, motions.len()) {
            motions.rotate_left(offset);
        }
    }
    let names: Vec<String> = families.keys().cloned().collect();
    let family_offset =
        seeded_index(seed, "", lane, "numeric_family_order", names.len()).unwrap_or(0);
    let mut ordered = Vec::with_capacity(pool.len());
    let mut indexes = vec![0; names.len()];
    loop {
        let mut added = false;
        for step in 0..names.len() {
            let index = (family_offset + step) % names.len();
            let motions = &families[&names[index]];
            if indexes[index] < motions.len() {
                ordered.push(motions[indexes[index]]);
                indexes[index] += 1;
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    ordered
}

fn numeric_motion_family(id: &str) -> &str {
    if id.starts_with("date_didone_") || id.starts_with("metric_didone_") {
        "didone"
    } else if id.starts_with("date_typewriter_") || id.starts_with("metric_typewriter_") {
        "typewriter"
    } else {
        "classic"
    }
}

/// A bullet pin that names a motion is authoritative; `apply_editorial_bullet_pins`
/// writes it after this phase. A pin that only carries font/size does not
/// choose an entrance, so the item still rotates the certified pool.
fn has_pinned_bullet_motion(request: &EditorialPlanV2Request, item: &PlanItemV2) -> bool {
    let role = item
        .params
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|object| object.get("editorial_role"))
        .and_then(Value::as_str);
    if !matches!(role, Some("scene_bullet" | "chapter_bullet")) {
        return false;
    }
    request
        .canvas
        .editorial_bullet_styles
        .iter()
        .any(|pin| pin.scene_id.trim() == item.scene_id.trim() && !pin.motion_id.trim().is_empty())
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
