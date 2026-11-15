//! Frame-accurate map collision resolution for PipelineGen overlay plans.
//!
//! The operation receives only fields needed by the timing algorithm and
//! returns timing edits keyed by original item index. PipelineGen retains the
//! complete overlay DTO and applies those edits without round-tripping opaque
//! render metadata through Rust.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod editorial_plan_v2;
pub use editorial_plan_v2::*;

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveMapOverlapsRequest {
    pub fps_num: i64,
    pub fps_den: i64,
    #[serde(default)]
    pub scene_windows: BTreeMap<String, [i64; 2]>,
    #[serde(default)]
    pub items: Vec<TimingItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimingItem {
    pub id: String,
    #[serde(default)]
    pub scene_id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default)]
    pub start_us: i64,
    #[serde(default)]
    pub duration_us: i64,
    #[serde(default)]
    pub has_map: bool,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingEdit {
    pub index: usize,
    pub id: String,
    pub keep: bool,
    pub start_us: i64,
    pub duration_us: i64,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CascadeEntrancesRequest {
    pub fps_num: i64,
    pub fps_den: i64,
    #[serde(default)]
    pub items: Vec<CascadeEntranceItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CascadeEntranceItem {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default)]
    pub motion_params: Option<Value>,
    #[serde(default)]
    pub image_layer_count: usize,
    #[serde(default)]
    pub asset_ref_count: usize,
    #[serde(default)]
    pub entity_caption: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub editorial_role: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntranceMotionEdit {
    pub index: usize,
    pub id: String,
    pub enter_frames: i64,
}

/// Staggers explicit singleton entrances that begin in the same visual beat.
/// The returned sparse edits leave the complete renderer DTO with PipelineGen.
pub fn cascade_clustered_entrances(request: CascadeEntrancesRequest) -> Vec<EntranceMotionEdit> {
    const EPSILON_MS: i64 = 800;
    const MAX_ENTER_FRAMES: i64 = 240;
    if request.items.len() < 2 {
        return Vec::new();
    }
    let (fps_num, fps_den) = if request.fps_num <= 0 || request.fps_den <= 0 {
        (24_i64, 1_i64)
    } else {
        (request.fps_num, request.fps_den)
    };
    let step = (fps_num / (fps_den * 2)).max(1);
    let mut eligible: Vec<usize> = request
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| cascade_eligible(item).then_some(index))
        .collect();
    eligible.sort_by(|left, right| {
        request.items[*left]
            .start_ms
            .cmp(&request.items[*right].start_ms)
            .then_with(|| left.cmp(right))
    });

    let mut edits = Vec::new();
    let mut group_start = 0;
    while group_start < eligible.len() {
        let mut group_end = group_start + 1;
        while group_end < eligible.len()
            && request.items[eligible[group_end]].start_ms
                - request.items[eligible[group_end - 1]].start_ms
                <= EPSILON_MS
        {
            group_end += 1;
        }
        let mut shaped: Vec<(usize, i64)> = eligible[group_start..group_end]
            .iter()
            .filter_map(|index| {
                let item = &request.items[*index];
                let frames = item.motion_params.as_ref()?.get("enter_frames")?.as_f64()? as i64;
                (frames > 0).then_some((*index, frames))
            })
            .collect();
        shaped.sort_by(|(left_index, _), (right_index, _)| {
            let left = &request.items[*left_index];
            let right = &request.items[*right_index];
            left.start_ms
                .cmp(&right.start_ms)
                .then_with(|| {
                    cascade_content_priority(left)
                        .cmp(&cascade_content_priority(right))
                        .reverse()
                })
                .then_with(|| left.id.cmp(&right.id))
                .then_with(|| left_index.cmp(right_index))
        });
        if let Some((_, base)) = shaped.first().copied() {
            for (rank, (index, current)) in shaped.iter().copied().enumerate().skip(1) {
                let item = &request.items[index];
                let duration = if item.end_ms <= item.start_ms {
                    0
                } else {
                    (((item.end_ms as i128 - item.start_ms as i128) * fps_num as i128)
                        / (1000_i128 * fps_den as i128))
                        .clamp(0, i64::MAX as i128) as i64
                };
                if duration <= 0 {
                    continue;
                }
                let target = (base as i128 + rank as i128 * step as i128)
                    .min(MAX_ENTER_FRAMES as i128)
                    .min(duration as i128) as i64;
                if target > current {
                    edits.push(EntranceMotionEdit {
                        index,
                        id: item.id.clone(),
                        enter_frames: target,
                    });
                }
            }
        }
        group_start = group_end;
    }
    edits.sort_by_key(|edit| edit.index);
    edits
}

fn cascade_eligible(item: &CascadeEntranceItem) -> bool {
    item.image_layer_count == 0
        && matches!(
            item.kind.as_str(),
            "text_phrase" | "image" | "entity_image" | "number"
        )
}

fn cascade_content_priority(item: &CascadeEntranceItem) -> i32 {
    if matches!(
        item.editorial_role.as_str(),
        Some("scene_title" | "scene_bullet")
    ) {
        return 100;
    }
    let kind = item.kind.trim().to_ascii_lowercase();
    let template = item.template_id.trim().to_ascii_uppercase();
    let entity_kind = matches!(
        kind.as_str(),
        "entity_image" | "entity_card" | "organization" | "location" | "concept"
    );
    let section_priority = if item.image_layer_count > 1 || item.asset_ref_count > 1 {
        Some(60)
    } else if item.image_layer_count == 1 {
        Some(60)
    } else if item.asset_ref_count == 1
        && entity_kind
        && (!item.entity_caption.trim().is_empty() || !item.text.trim().is_empty())
    {
        Some(60)
    } else if matches!(template.as_str(), "IMPORTANT_PHRASE")
        || kind == "text_phrase"
        || kind == "important_phrase"
    {
        Some(100)
    } else if matches!(
        template.as_str(),
        "NUMBER"
            | "MONEY"
            | "PERCENT"
            | "PERCENTAGE"
            | "DATE"
            | "TIME"
            | "METRIC_STAT_CARD"
            | "TIMELINE_DATE_CARD"
    ) || matches!(
        kind.as_str(),
        "number" | "metric_stat" | "timeline_date" | "date" | "time"
    ) {
        Some(80)
    } else if item.asset_ref_count == 1
        || item.image_layer_count == 1
        || matches!(
            kind.as_str(),
            "image" | "entity_image" | "image_popup" | "product" | "logo"
        )
        || matches!(
            template.as_str(),
            "IMAGE_OVERLAY" | "IMAGE_ENTITY" | "IMAGE_POPUP" | "PRODUCT" | "LOGO"
        )
    {
        Some(60)
    } else {
        None
    };
    if let Some(priority) = section_priority {
        return priority;
    }
    match item.template_id.as_str() {
        "IMPORTANT_PHRASE" => 100,
        "IMPORTANT_WORD" | "NUMBER" | "QUOTE" | "LOCATION" | "person_default" | "org_default"
        | "gpe_default" | "concept_default" | "lower_third" | "quote" => 80,
        "IMAGE_OVERLAY" | "PRODUCT" | "LOGO" | "image_popup" => 60,
        "LIGHT_LEAK" => 20,
        _ => match kind.as_str() {
            "entity_card" | "organization" | "location" | "concept" | "lower_third" | "quote"
            | "important_word" | "number" | "brand_text" => 80,
            "product" | "logo" | "image_popup" | "entity_image" => 60,
            _ => 0,
        },
    }
}

/// Trims map windows before PipelineGen spends editorial overlap budgets.
/// Non-map items are returned unchanged so callers can apply the edits by
/// original index without projecting their full DTO through Rust.
pub fn trim_map_windows_before_budget(request: ResolveMapOverlapsRequest) -> Vec<TimingEdit> {
    let (fps_num, fps_den) = if request.fps_num <= 0 || request.fps_den <= 0 {
        (24_i64, 1_i64)
    } else {
        (request.fps_num, request.fps_den)
    };
    let input = request.items;
    let mut edits: Vec<_> = input
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let window = item_window(item);
            TimingEdit {
                index,
                id: item.id.clone(),
                keep: true,
                start_us: window.start_us,
                duration_us: window.duration_us,
                start_ms: item.start_ms,
                end_ms: item.end_ms,
            }
        })
        .collect();
    let mut map_indices: Vec<usize> = input
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item.has_map && !is_background(item)).then_some(index))
        .collect();
    if map_indices.is_empty() {
        return edits;
    }
    map_indices.sort_by(|left, right| {
        let left_start = item_window(&input[*left]).start_us;
        let right_start = item_window(&input[*right]).start_us;
        left_start
            .cmp(&right_start)
            .then_with(|| input[*left].id.cmp(&input[*right].id))
    });
    let mut maps: Vec<Window> = map_indices
        .iter()
        .map(|index| item_window(&input[*index]))
        .collect();
    trim_map_windows(
        &input,
        &map_indices,
        &mut maps,
        &BTreeMap::new(),
        fps_num,
        fps_den,
    );
    for (map_position, item_index) in map_indices.iter().enumerate() {
        set_window(&mut edits[*item_index], maps[map_position]);
    }
    edits
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start_us: i64,
    duration_us: i64,
}

impl Window {
    fn end_us(self) -> i64 {
        self.start_us.saturating_add(self.duration_us)
    }
}

/// Resolves map collisions with the same stable ordering and frame rounding
/// used by PipelineGen's former Go implementation.
pub fn resolve_map_overlaps(request: ResolveMapOverlapsRequest) -> Vec<TimingEdit> {
    let (fps_num, fps_den) = if request.fps_num <= 0 || request.fps_den <= 0 {
        (24_i64, 1_i64)
    } else {
        (request.fps_num, request.fps_den)
    };
    let input = request.items;
    let mut edits: Vec<_> = input
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let window = item_window(item);
            TimingEdit {
                index,
                id: item.id.clone(),
                keep: true,
                start_us: window.start_us,
                duration_us: window.duration_us,
                start_ms: item.start_ms,
                end_ms: item.end_ms,
            }
        })
        .collect();
    if input.len() < 2 {
        return edits;
    }

    let mut map_indices: Vec<usize> = input
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item.has_map && !is_background(item)).then_some(index))
        .collect();
    if map_indices.is_empty() {
        return edits;
    }
    map_indices.sort_by(|left, right| {
        let left_start = item_window(&input[*left]).start_us;
        let right_start = item_window(&input[*right]).start_us;
        left_start
            .cmp(&right_start)
            .then_with(|| input[*left].id.cmp(&input[*right].id))
    });

    let mut maps: Vec<Window> = map_indices
        .iter()
        .map(|index| item_window(&input[*index]))
        .collect();
    trim_map_windows(
        &input,
        &map_indices,
        &mut maps,
        &request.scene_windows,
        fps_num,
        fps_den,
    );
    for (map_position, item_index) in map_indices.iter().enumerate() {
        set_window(&mut edits[*item_index], maps[map_position]);
    }

    for (index, item) in input.iter().enumerate() {
        if item.has_map && !is_background(item) {
            continue;
        }
        if is_background(item) {
            continue;
        }
        let current = item_window(item);
        let Some(resolved) = shift_past_maps(
            item,
            current,
            &maps,
            &map_indices,
            &request.scene_windows,
            fps_num,
            fps_den,
        ) else {
            edits[index].keep = false;
            continue;
        };
        set_window(&mut edits[index], resolved);
    }
    edits
}

fn item_window(item: &TimingItem) -> Window {
    if item.duration_us > 0 {
        Window {
            start_us: item.start_us,
            duration_us: item.duration_us,
        }
    } else {
        Window {
            start_us: item.start_ms.saturating_mul(1000),
            duration_us: item
                .end_ms
                .saturating_sub(item.start_ms)
                .saturating_mul(1000),
        }
    }
}

fn set_window(edit: &mut TimingEdit, window: Window) {
    edit.start_us = window.start_us;
    edit.duration_us = window.duration_us;
    edit.start_ms = window.start_us / 1000;
    edit.end_ms = window.end_us().saturating_add(999) / 1000;
}

fn frames(window: Window, fps_num: i64, fps_den: i64) -> (i128, i128) {
    let unit = 1_000_000_i128 * i128::from(fps_den);
    let fps = i128::from(fps_num);
    let start = i128::from(window.start_us) * fps / unit;
    let end_us = i128::from(window.start_us) + i128::from(window.duration_us);
    let mut end = (end_us * fps + unit - 1) / unit;
    if end <= start {
        end = start + 1;
    }
    (start, end)
}

fn overlap(left: Window, right: Window, fps_num: i64, fps_den: i64) -> bool {
    if left.duration_us <= 0 || right.duration_us <= 0 {
        return false;
    }
    let (left_start, left_end) = frames(left, fps_num, fps_den);
    let (right_start, right_end) = frames(right, fps_num, fps_den);
    left_start < right_end && right_start < left_end
}

fn trim_map_windows(
    items: &[TimingItem],
    map_indices: &[usize],
    maps: &mut [Window],
    scene_windows: &BTreeMap<String, [i64; 2]>,
    fps_num: i64,
    fps_den: i64,
) {
    for index in 0..map_indices.len() {
        let item = &items[map_indices[index]];
        let current = maps[index];
        let start_us = current.start_us;
        let mut limit_us = current.end_us();
        if index + 1 < map_indices.len() {
            let next_start = maps[index + 1].start_us;
            if next_start > start_us && next_start < limit_us {
                limit_us = next_start;
            }
        }
        if let Some(scene_window) = scene_windows.get(item.scene_id.trim()) {
            if scene_window[1] > start_us && scene_window[1] < limit_us {
                limit_us = scene_window[1];
            }
        }
        if start_us < 0
            || current.duration_us <= 0
            || limit_us >= current.end_us()
            || limit_us <= start_us
        {
            continue;
        }
        let (start_frame, _) = frames(current, fps_num, fps_den);
        let (_, limit_frame) = frames(
            Window {
                start_us: limit_us,
                duration_us: 1,
            },
            fps_num,
            fps_den,
        );
        if limit_frame <= start_frame {
            continue;
        }
        let unit = 1_000_000_i128 * i128::from(fps_den);
        let end_us = ((limit_frame - 1) * unit + i128::from(fps_num) - 1) / i128::from(fps_num);
        let shortened = end_us - i128::from(start_us);
        if shortened <= 0 || shortened > i128::from(i64::MAX) {
            continue;
        }
        let shortened = shortened as i64;
        if map_pins_fit_duration(item, shortened) {
            maps[index].duration_us = shortened;
        }
    }
}

fn map_pins_fit_duration(item: &TimingItem, duration_us: i64) -> bool {
    let Some(pins) = item
        .params
        .as_ref()
        .and_then(|params| params.get("map_pin_timing"))
    else {
        return true;
    };
    let Some(pins) = pins.as_object() else {
        return true;
    };
    if duration_us <= 0 {
        return false;
    }
    if pins.values().any(|entry| !entry.is_object()) {
        return true;
    }
    pins.values().all(|entry| {
        let entry = entry.as_object().expect("object shape checked above");
        let Some(start) = integral_i64(entry.get("start_us")) else {
            return false;
        };
        let Some(duration) = integral_i64(entry.get("duration_us")) else {
            return false;
        };
        start >= 0
            && duration > 0
            && start <= duration_us
            && duration <= duration_us
            && start <= duration_us - duration
    })
}

fn integral_i64(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(integer) = value.as_i64() {
        return Some(integer);
    }
    let number = value.as_f64()?;
    if number.fract() != 0.0 || number < i64::MIN as f64 || number > i64::MAX as f64 {
        return None;
    }
    Some(number as i64)
}

fn shift_past_maps(
    item: &TimingItem,
    mut current: Window,
    maps: &[Window],
    map_indices: &[usize],
    scene_windows: &BTreeMap<String, [i64; 2]>,
    fps_num: i64,
    fps_den: i64,
) -> Option<Window> {
    if is_anchored(item) || is_chain_kind(&item.kind) {
        return (!maps
            .iter()
            .any(|map| overlap(current, *map, fps_num, fps_den)))
        .then_some(current);
    }
    for _ in 0..=map_indices.len() {
        let mut latest_end = -1_i128;
        for map in maps {
            if overlap(current, *map, fps_num, fps_den) {
                let (_, map_end) = frames(*map, fps_num, fps_den);
                latest_end = latest_end.max(map_end);
            }
        }
        if latest_end < 0 {
            return Some(current);
        }
        let unit = 1_000_000_i128 * i128::from(fps_den);
        let start_us = latest_end * unit / i128::from(fps_num);
        if start_us < 0 || start_us > i128::from(i64::MAX) {
            return None;
        }
        let start_us = start_us as i64;
        if let Some(window) = scene_windows.get(item.scene_id.trim()) {
            if start_us >= window[1] || start_us.saturating_add(current.duration_us) > window[1] {
                return None;
            }
        } else if !item.scene_id.trim().is_empty() {
            return None;
        }
        if current.duration_us <= 0 {
            return None;
        }
        current.start_us = start_us;
    }
    None
}

fn is_background(item: &TimingItem) -> bool {
    matches!(
        item.template_id.trim().to_ascii_lowercase().as_str(),
        "background" | "video_background"
    )
}

fn is_anchored(item: &TimingItem) -> bool {
    let role = item
        .params
        .as_ref()
        .and_then(|params| params.get("editorial_role"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        role.as_str(),
        "scene_title" | "scene_bullet" | "chapter_bullet"
    )
}

fn is_chain_kind(kind: &str) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "phrase_chain" | "image_chain" | "mixed_chain" | "map_chain"
    )
}

#[derive(Debug, Clone, Deserialize)]
pub struct RankEntityOverlaysRequest {
    #[serde(default)]
    pub scenes: Vec<EntityRankingScene>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityRankingScene {
    #[serde(default)]
    pub entities: Vec<EntityRankingInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityRankingInput {
    pub entity_id: String,
    pub overlay_id: String,
    pub entity_type: String,
    #[serde(default)]
    pub confidence: f64,
}

/// Ranks canonical entity occurrences for PipelineGen's named-card resolver.
/// The caller supplies the canonical type and the exact render identity; Rust
/// owns run-level frequency/novelty scoring, stable ordering and per-scene
/// identity deduplication. Returned indices preserve the original DTOs in Go.
pub fn rank_entity_overlays(request: RankEntityOverlaysRequest) -> Vec<Vec<usize>> {
    use std::collections::{HashMap, HashSet};

    let mut frequencies: HashMap<&str, usize> = HashMap::new();
    for scene in &request.scenes {
        for entity in &scene.entities {
            *frequencies.entry(entity.entity_id.as_str()).or_default() += 1;
        }
    }
    let max_frequency = frequencies.values().copied().max().unwrap_or_default();

    request
        .scenes
        .iter()
        .map(|scene| {
            let mut ranked: Vec<(usize, f64)> = scene
                .entities
                .iter()
                .enumerate()
                .map(|(index, entity)| {
                    let frequency = frequencies
                        .get(entity.entity_id.as_str())
                        .copied()
                        .unwrap_or_default();
                    let normalized_frequency = if max_frequency == 0 {
                        0.0
                    } else {
                        frequency as f64 / max_frequency as f64
                    };
                    let novelty = if frequency == 1 { 1.0 } else { 0.5 };
                    let scene_relevance = if entity.confidence > 0.0 {
                        entity.confidence.clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    let importance = 0.30 * entity_type_weight(&entity.entity_type)
                        + 0.25 * normalized_frequency
                        + 0.20 * scene_relevance
                        + 0.15 * novelty
                        + 0.10 * 0.0;
                    (index, importance)
                })
                .collect();
            ranked.sort_by(|left, right| {
                right
                    .1
                    .total_cmp(&left.1)
                    .then_with(|| left.0.cmp(&right.0))
            });
            let mut seen = HashSet::with_capacity(ranked.len());
            ranked
                .into_iter()
                .filter_map(|(index, _)| {
                    seen.insert(scene.entities[index].overlay_id.clone())
                        .then_some(index)
                })
                .collect()
        })
        .collect()
}

fn entity_type_weight(entity_type: &str) -> f64 {
    match entity_type.trim().to_ascii_uppercase().as_str() {
        "PERSON" => 1.0,
        "ORG" | "LOGO" => 0.9,
        "PLACE" => 0.8,
        "PRODUCT" => 0.75,
        "CONCEPT" | "EVENT" => 0.65,
        "NUMBER" | "MONEY" | "PERCENT" | "QUOTE" => 0.55,
        _ => 0.5,
    }
}

/// One request for the editorial-plan batch. The caller supplies the planned
/// overlay items and the already-extracted entity/entrance facts; Rust owns
/// the timing, ranking and cascade decisions in one worker round-trip.
#[derive(Debug, Clone, Deserialize)]
pub struct CompileEditorialPlanRequest {
    #[serde(default)]
    pub fps_num: i64,
    #[serde(default)]
    pub fps_den: i64,
    #[serde(default)]
    pub scene_windows: BTreeMap<String, [i64; 2]>,
    #[serde(default)]
    pub items: Vec<TimingItem>,
    #[serde(default)]
    pub entity_scenes: Vec<EntityRankingScene>,
    #[serde(default)]
    pub entrance_items: Vec<CascadeEntranceItem>,
}

/// The four batch sections PipelineGen used to request separately. Keeping
/// them as sparse edits means the caller still applies them to its complete
/// overlay DTO without round-tripping opaque render metadata through Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompiledEditorialPlan {
    pub pre_budget_edits: Vec<TimingEdit>,
    pub timing_edits: Vec<TimingEdit>,
    pub ranked_entities: Vec<Vec<usize>>,
    pub entrance_edits: Vec<EntranceMotionEdit>,
}

/// Compiles the editorial/timing sections of an overlay plan in one call.
/// This is the batch entry point; the per-concern functions remain the pure
/// building blocks and keep their dedicated protocol handlers for compatibility.
pub fn compile_editorial_plan(request: CompileEditorialPlanRequest) -> CompiledEditorialPlan {
    let pre_budget_edits = trim_map_windows_before_budget(ResolveMapOverlapsRequest {
        fps_num: request.fps_num,
        fps_den: request.fps_den,
        scene_windows: request.scene_windows.clone(),
        items: request.items.clone(),
    });
    let timing_edits = resolve_map_overlaps(ResolveMapOverlapsRequest {
        fps_num: request.fps_num,
        fps_den: request.fps_den,
        scene_windows: request.scene_windows,
        items: request.items,
    });
    let ranked_entities = rank_entity_overlays(RankEntityOverlaysRequest {
        scenes: request.entity_scenes,
    });
    let entrance_edits = cascade_clustered_entrances(CascadeEntrancesRequest {
        fps_num: request.fps_num,
        fps_den: request.fps_den,
        items: request.entrance_items,
    });
    CompiledEditorialPlan {
        pre_budget_edits,
        timing_edits,
        ranked_entities,
        entrance_edits,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cascade_clustered_entrances, compile_editorial_plan, rank_entity_overlays,
        resolve_map_overlaps, trim_map_windows_before_budget, CascadeEntranceItem,
        CascadeEntrancesRequest, CompileEditorialPlanRequest, EntityRankingInput,
        EntityRankingScene, RankEntityOverlaysRequest, ResolveMapOverlapsRequest, TimingItem,
    };
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    #[test]
    fn entity_overlay_ranking_uses_run_frequency_and_deduplicates_stably() {
        let scenes = vec![
            EntityRankingScene {
                entities: vec![
                    EntityRankingInput {
                        entity_id: "person-tim".to_owned(),
                        overlay_id: "overlay-s0-tim".to_owned(),
                        entity_type: "PERSON".to_owned(),
                        confidence: 0.98,
                    },
                    EntityRankingInput {
                        entity_id: "org-apple".to_owned(),
                        overlay_id: "overlay-s0-apple".to_owned(),
                        entity_type: "ORG".to_owned(),
                        confidence: 0.9,
                    },
                    EntityRankingInput {
                        entity_id: "concept-ai".to_owned(),
                        overlay_id: "overlay-s0-ai".to_owned(),
                        entity_type: "CONCEPT".to_owned(),
                        confidence: 0.85,
                    },
                    EntityRankingInput {
                        entity_id: "person-tim".to_owned(),
                        overlay_id: "overlay-s0-tim".to_owned(),
                        entity_type: "PERSON".to_owned(),
                        confidence: 0.98,
                    },
                ],
            },
            EntityRankingScene {
                entities: vec![EntityRankingInput {
                    entity_id: "place-la".to_owned(),
                    overlay_id: "overlay-s1-la".to_owned(),
                    entity_type: "PLACE".to_owned(),
                    confidence: 0.9,
                }],
            },
        ];
        assert_eq!(
            rank_entity_overlays(RankEntityOverlaysRequest { scenes }),
            vec![vec![0, 1, 2], vec![0]],
            "the repeated person must rank first, duplicate overlay IDs must collapse, and ties must keep source order"
        );
    }

    fn cascade_item(
        id: &str,
        kind: &str,
        start_ms: i64,
        end_ms: i64,
        enter_frames: Option<i64>,
    ) -> CascadeEntranceItem {
        CascadeEntranceItem {
            id: id.to_owned(),
            kind: kind.to_owned(),
            template_id: String::new(),
            start_ms,
            end_ms,
            motion_params: enter_frames.map(|frames| json!({"enter_frames":frames})),
            image_layer_count: 0,
            asset_ref_count: 0,
            entity_caption: String::new(),
            text: String::new(),
            editorial_role: Value::Null,
        }
    }

    #[test]
    fn clustered_entrances_are_priority_ordered_and_only_lengthened() {
        let edits = cascade_clustered_entrances(CascadeEntrancesRequest {
            fps_num: 24,
            fps_den: 1,
            items: vec![
                cascade_item("image", "entity_image", 100, 3_000, Some(8)),
                cascade_item("phrase", "text_phrase", 0, 3_000, Some(8)),
                cascade_item("map", "location", 0, 3_000, Some(8)),
            ],
        });
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].index, 0);
        assert_eq!(edits[0].enter_frames, 20);
    }

    #[test]
    fn clustered_entrances_skip_composites_and_never_exceed_window() {
        let mut composite = cascade_item("composite", "entity_image", 0, 3_000, Some(8));
        composite.image_layer_count = 2;
        let edits = cascade_clustered_entrances(CascadeEntrancesRequest {
            fps_num: 24,
            fps_den: 1,
            items: vec![
                cascade_item("phrase", "text_phrase", 0, 3_000, Some(8)),
                cascade_item("number", "number", 0, 200, Some(5)),
                composite,
            ],
        });
        assert!(
            edits.is_empty(),
            "a short window must never shorten its authored entrance"
        );
    }

    fn item(id: &str, start_us: i64, duration_us: i64, has_map: bool) -> TimingItem {
        TimingItem {
            id: id.to_owned(),
            scene_id: "scene-1".to_owned(),
            kind: if has_map { "map" } else { "image" }.to_owned(),
            template_id: String::new(),
            start_ms: 0,
            end_ms: 0,
            start_us,
            duration_us,
            has_map,
            params: None,
        }
    }

    fn request(items: Vec<TimingItem>) -> ResolveMapOverlapsRequest {
        ResolveMapOverlapsRequest {
            fps_num: 24,
            fps_den: 1,
            scene_windows: BTreeMap::from([("scene-1".to_owned(), [0, 5_000_000])]),
            items,
        }
    }

    #[test]
    fn trims_an_earlier_map_to_the_next_maps_frame_boundary() {
        let edits = trim_map_windows_before_budget(request(vec![
            item("map-a", 0, 2_000_000, true),
            item("map-b", 1_500_000, 1_000_000, true),
        ]));

        assert_eq!(edits[0].duration_us, 1_500_000);
        assert_eq!(edits[0].end_ms, 1_500);
        assert_eq!(edits[1].start_us, 1_500_000);
    }

    #[test]
    fn shifts_unanchored_items_after_a_colliding_map() {
        let edits = resolve_map_overlaps(request(vec![
            item("map", 1_000_000, 1_000_000, true),
            item("image", 1_500_000, 500_000, false),
        ]));

        assert!(edits[1].keep);
        assert_eq!(edits[1].start_us, 2_000_000);
    }

    #[test]
    fn compile_editorial_plan_batches_map_ranking_and_cascade() {
        let request = CompileEditorialPlanRequest {
            fps_num: 24,
            fps_den: 1,
            scene_windows: BTreeMap::from([("scene-1".to_owned(), [0, 5_000_000])]),
            items: vec![
                item("map-a", 0, 2_000_000, true),
                item("map-b", 1_500_000, 1_000_000, true),
                item("image", 1_600_000, 500_000, false),
            ],
            entity_scenes: vec![EntityRankingScene {
                entities: vec![EntityRankingInput {
                    entity_id: "person-tim".to_owned(),
                    overlay_id: "overlay-s0-tim".to_owned(),
                    entity_type: "PERSON".to_owned(),
                    confidence: 0.98,
                }],
            }],
            entrance_items: vec![
                cascade_item("image", "entity_image", 100, 3_000, Some(8)),
                cascade_item("phrase", "text_phrase", 0, 3_000, Some(8)),
            ],
        };
        let plan = compile_editorial_plan(request.clone());
        assert_eq!(
            plan.pre_budget_edits,
            trim_map_windows_before_budget(ResolveMapOverlapsRequest {
                fps_num: request.fps_num,
                fps_den: request.fps_den,
                scene_windows: request.scene_windows.clone(),
                items: request.items.clone(),
            })
        );
        assert_eq!(
            plan.timing_edits,
            resolve_map_overlaps(ResolveMapOverlapsRequest {
                fps_num: request.fps_num,
                fps_den: request.fps_den,
                scene_windows: request.scene_windows.clone(),
                items: request.items.clone(),
            })
        );
        assert_eq!(
            plan.ranked_entities,
            rank_entity_overlays(RankEntityOverlaysRequest {
                scenes: request.entity_scenes.clone(),
            })
        );
        assert_eq!(
            plan.entrance_edits,
            cascade_clustered_entrances(CascadeEntrancesRequest {
                fps_num: request.fps_num,
                fps_den: request.fps_den,
                items: request.entrance_items.clone(),
            })
        );
    }

    #[test]
    fn compile_editorial_plan_is_empty_without_inputs() {
        let plan = compile_editorial_plan(CompileEditorialPlanRequest {
            fps_num: 0,
            fps_den: 0,
            scene_windows: BTreeMap::new(),
            items: Vec::new(),
            entity_scenes: Vec::new(),
            entrance_items: Vec::new(),
        });
        assert!(plan.pre_budget_edits.is_empty());
        assert!(plan.timing_edits.is_empty());
        assert!(plan.ranked_entities.is_empty());
        assert!(plan.entrance_edits.is_empty());
    }

    #[test]
    fn drops_a_scene_title_that_overlaps_a_map() {
        let mut title = item("title", 1_500_000, 500_000, false);
        title.params = Some(json!({"editorial_role": "scene_title"}));
        let edits = resolve_map_overlaps(request(vec![
            item("map", 1_000_000, 1_000_000, true),
            title,
        ]));

        assert!(!edits[1].keep);
    }
}
