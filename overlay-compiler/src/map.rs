use super::*;

/// The presentation window the v2 planner owns after the spoken-span trim.
/// Go used to re-apply these numbers after materializing a plate, which made
/// the readable floor a second, late owner of the same decision. Keeping the
/// policy in the resolver makes the map window final before the plan is sealed:
/// a map gets its camera minimum, is never trimmed below the readable floor
/// when the next map can move, and never outlives the plan.
#[derive(Debug, Clone, Deserialize)]
pub struct MapPresentationWindow {
    #[serde(default)]
    pub camera_duration_us: i64,
    #[serde(default)]
    pub camera_lead_us: i64,
    #[serde(default)]
    pub pin_read_duration_us: i64,
    #[serde(default)]
    pub readable_floor_us: i64,
    #[serde(default)]
    pub presentation_ceiling_us: i64,
    /// End of the whole plan. Zero disables the tail bound (legacy callers).
    #[serde(default)]
    pub plan_end_us: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveMapOverlapsRequest {
    pub fps_num: i64,
    pub fps_den: i64,
    #[serde(default)]
    pub scene_windows: BTreeMap<String, [i64; 2]>,
    #[serde(default)]
    pub items: Vec<TimingItem>,
    /// Presentation policy for map items. `None` keeps the legacy behavior
    /// (spoken windows only), so the editorial-plan batch endpoint is
    /// unaffected while the v2 planner adopts the final map windows.
    #[serde(default)]
    pub map_presentation: Option<MapPresentationWindow>,
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

pub(super) fn cascade_eligible(item: &CascadeEntranceItem) -> bool {
    item.image_layer_count == 0
        && matches!(
            item.kind.as_str(),
            "text_phrase" | "image" | "entity_image" | "number"
        )
}

pub(super) fn cascade_content_priority(item: &CascadeEntranceItem) -> i32 {
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
    let section_priority = if item.image_layer_count >= 1
        || item.asset_ref_count > 1
        || (item.asset_ref_count == 1
            && entity_kind
            && (!item.entity_caption.trim().is_empty() || !item.text.trim().is_empty()))
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
pub(super) struct Window {
    start_us: i64,
    duration_us: i64,
}

impl Window {
    fn end_us(self) -> i64 {
        self.start_us.saturating_add(self.duration_us)
    }
}

mod timing;
pub use timing::*;

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

pub(super) fn entity_type_weight(entity_type: &str) -> f64 {
    match semantic_core::entity_capabilities::fold_entity_kind(entity_type) {
        EntityKind::Person => 1.0,
        EntityKind::Organization | EntityKind::Logo => 0.9,
        EntityKind::Place => 0.8,
        EntityKind::Product => 0.75,
        EntityKind::Concept | EntityKind::Event => 0.65,
        EntityKind::Number | EntityKind::Money | EntityKind::Percent | EntityKind::Quote => 0.55,
        _ => 0.5,
    }
}
