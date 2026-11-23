use super::*;

/// Resolves map collisions with the same stable ordering and frame rounding
/// used by PipelineGen's former Go implementation.
pub fn resolve_map_overlaps(request: ResolveMapOverlapsRequest) -> Vec<TimingEdit> {
    let (fps_num, fps_den) = if request.fps_num <= 0 || request.fps_den <= 0 {
        (24_i64, 1_i64)
    } else {
        (request.fps_num, request.fps_den)
    };
    let presentation = request.map_presentation.clone();
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
        &request.scene_windows,
        fps_num,
        fps_den,
    );
    // The planner's map window is final: extend every map to its camera
    // minimum, then resolve the readable floor and the tail bound exactly like
    // the retired Go clamp. Non-map items are shifted against THESE windows,
    // so no chain is later sealed on top of a stretched map.
    if let Some(policy) = presentation.as_ref() {
        extend_map_presentation(&input, &map_indices, &mut maps, policy);
        clamp_map_presentation_windows(&mut maps, policy);
    }
    for (map_position, item_index) in map_indices.iter().enumerate() {
        set_window(&mut edits[*item_index], maps[map_position]);
    }

    if input.len() < 2 {
        return edits;
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

/// Grows a map's spoken window to the certified camera beat. The old Go
/// materializer applied the same constants after the plan was sealed; doing it
/// here means the sealed plan already carries the window the renderer uses.
pub(super) fn extend_map_presentation(
    items: &[TimingItem],
    map_indices: &[usize],
    maps: &mut [Window],
    policy: &MapPresentationWindow,
) {
    let ceiling = if policy.presentation_ceiling_us > 0 {
        policy.presentation_ceiling_us
    } else {
        i64::MAX
    };
    for (position, item_index) in map_indices.iter().enumerate() {
        let item = &items[*item_index];
        let span = maps[position].duration_us.clamp(0, ceiling);
        let mut duration = span;
        if policy.camera_duration_us > 0 {
            duration = duration.max(policy.camera_duration_us);
        }
        // Every location after the camera settles gets its own readable slot.
        let pins = item
            .params
            .as_ref()
            .and_then(|params| params.get("map_group_pins"))
            .and_then(Value::as_array)
            .map(|pins| pins.len())
            .unwrap_or(1)
            .max(1) as i64;
        if policy.camera_lead_us > 0 || policy.pin_read_duration_us > 0 {
            duration = duration.max(
                policy
                    .camera_lead_us
                    .saturating_add(policy.pin_read_duration_us.saturating_mul(pins)),
            );
        }
        if ceiling < i64::MAX {
            duration = duration.min(ceiling);
        }
        if duration > maps[position].duration_us {
            maps[position].duration_us = duration;
        }
    }
}

/// Ported from PipelineGen's late clamp: two basemaps must never composite on
/// the same frame. A map is trimmed to the next map's start, but when that
/// would squeeze it below the readable floor the next map moves later instead,
/// as long as it still ends inside the plan. The final pass bounds the tail.
pub(super) fn clamp_map_presentation_windows(maps: &mut [Window], policy: &MapPresentationWindow) {
    let floor_us = policy.readable_floor_us.max(0);
    let plan_end_us = policy.plan_end_us;
    for index in 0..maps.len().saturating_sub(1) {
        let current = maps[index];
        let next = maps[index + 1];
        if next.start_us <= current.start_us || current.end_us() <= next.start_us {
            continue;
        }
        let mut end_us = next.start_us;
        let floor_end_us = current.start_us.saturating_add(floor_us);
        if floor_end_us > next.start_us && floor_end_us < current.end_us() {
            let shift_us = floor_end_us - next.start_us;
            if plan_end_us <= 0 || next.end_us().saturating_add(shift_us) <= plan_end_us {
                maps[index + 1].start_us = next.start_us.saturating_add(shift_us);
                end_us = floor_end_us;
            }
        }
        maps[index].duration_us = (end_us - current.start_us).max(0);
    }
    if plan_end_us > 0 {
        for map in maps.iter_mut() {
            if map.start_us >= plan_end_us || map.end_us() <= plan_end_us {
                continue;
            }
            map.duration_us = plan_end_us - map.start_us;
        }
    }
}

pub(super) fn item_window(item: &TimingItem) -> Window {
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

pub(super) fn set_window(edit: &mut TimingEdit, window: Window) {
    edit.start_us = window.start_us;
    edit.duration_us = window.duration_us;
    edit.start_ms = window.start_us / 1000;
    edit.end_ms = window.end_us().saturating_add(999) / 1000;
}

pub(super) fn frames(window: Window, fps_num: i64, fps_den: i64) -> (i128, i128) {
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

pub(super) fn overlap(left: Window, right: Window, fps_num: i64, fps_den: i64) -> bool {
    if left.duration_us <= 0 || right.duration_us <= 0 {
        return false;
    }
    let (left_start, left_end) = frames(left, fps_num, fps_den);
    let (right_start, right_end) = frames(right, fps_num, fps_den);
    left_start < right_end && right_start < left_end
}

pub(super) fn trim_map_windows(
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

pub(super) fn map_pins_fit_duration(item: &TimingItem, duration_us: i64) -> bool {
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

pub(super) fn integral_i64(value: Option<&Value>) -> Option<i64> {
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

pub(super) fn shift_past_maps(
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

pub(super) fn is_background(item: &TimingItem) -> bool {
    matches!(
        item.template_id.trim().to_ascii_lowercase().as_str(),
        "background" | "video_background"
    )
}

pub(super) fn is_anchored(item: &TimingItem) -> bool {
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

pub(super) fn is_chain_kind(kind: &str) -> bool {
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
