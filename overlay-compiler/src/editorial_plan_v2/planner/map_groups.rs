//! Map grouping, the readable floor and the pin schedule.
//!
//! The planner used to emit one map item per grounded place and let Go group
//! them afterwards. Every downstream rewrite then had to guess which group a
//! rendered map belonged to (the "labels swapped between maps" bug) and had to
//! re-apply the 2.5 s readable floor after the windows were already decided
//! (the "0.94 s map flash" bug). Grouping is an editorial decision, so it is
//! taken here, once, next to the windows it has to respect:
//!
//! * places of one scene named in quick succession share one map item,
//! * a group never exceeds the certified pin cap nor the presentation ceiling,
//! * a final map shorter than the readable floor is folded into the preceding
//!   map of its scene (never dropped), moving its trailing pins instead when
//!   the preceding map is already at the pin cap.
//!
//! The item carries its pins in `params.map_group_pins` and their spoken
//! schedule in `params.map_pin_timing`, so Go only has to attach a certified
//! plate: it never decides membership, order or the window again. Winds are
//! emitted in spoken order and each pin keeps its own window, which is what
//! makes the assignment unambiguous.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Map, Value};

use super::super::request::EditorialPlanV2Request;
use super::super::response::PlanItemV2;

/// Shortest map presentation that reads as a camera beat instead of a flash.
pub(crate) const MAP_READABLE_FLOOR_US: i64 = 2_500_000;

/// Hard ceiling for one map item. A longer spoken span is segmented into
/// consecutive bounded map items instead of stretching one map for minutes.
pub(crate) const MAP_PRESENTATION_CEILING_US: i64 = 10_000_000;

/// Camera minimum for one map item, and the readable slot each place gets
/// after the camera settles. These are the presentation policy the Go
/// materializer used to apply after sealing; the resolver now owns them so the
/// readable floor has a single editor.
pub(crate) const MAP_CAMERA_DURATION_US: i64 = 8_000_000;
pub(crate) const MAP_CAMERA_LEAD_US: i64 = 2_000_000;
pub(crate) const MAP_PIN_READ_DURATION_US: i64 = 1_000_000;

/// Two places of the same scene whose spoken windows are closer than this read
/// as one narrative beat, so they share a map. The value mirrors the Go
/// planner's certified `maxMapNarrativeGapUS` it replaces: moving the decision
/// must not move the policy. It is a grouping hint, not a georeference: the
/// plate that finally covers the group is still certified by the producer.
const MAP_NARRATIVE_GAP_US: i64 = 8_000_000;

/// Certified default when the request carries no explicit pin cap.
const MAP_DEFAULT_PIN_CAP: i64 = 5;

/// Regroups the admitted map items scene by scene. Items that are not maps
/// keep their position, their window and their identity; consecutive maps of
/// one scene are replaced by one item per group, anchored at the position of
/// the group's first map.
pub(crate) fn group_map_items(
    items: Vec<PlanItemV2>,
    request: &EditorialPlanV2Request,
) -> Vec<PlanItemV2> {
    if !items.iter().any(is_map_item) {
        return items;
    }
    let presentation = &request.canvas.map_presentation;
    let cap = if presentation.max_pins_per_group > 0 {
        presentation.max_pins_per_group
    } else {
        MAP_DEFAULT_PIN_CAP
    };
    let group_nearby = presentation.group_nearby_places;

    // Map positions per scene, in plan order.
    let mut by_scene: Vec<(String, Vec<usize>)> = Vec::new();
    let mut scene_slot: HashMap<&str, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if !is_map_item(item) {
            continue;
        }
        let scene_id = item.scene_id.trim();
        let slot = match scene_slot.get(scene_id) {
            Some(&slot) => slot,
            None => {
                by_scene.push((scene_id.to_string(), Vec::new()));
                scene_slot.insert(scene_id, by_scene.len() - 1);
                by_scene.len() - 1
            }
        };
        by_scene[slot].1.push(index);
    }

    // Replacement value per anchor index, and the indices a regrouped scene
    // stops emitting on its own.
    let mut replacements: BTreeMap<usize, PlanItemV2> = BTreeMap::new();
    let mut consumed: Vec<usize> = Vec::new();
    for (scene_id, indices) in by_scene {
        let groups = build_groups(&items, &indices, cap, group_nearby);
        if groups.len() == indices.len() {
            // One group per place: nothing was grouped and no tail was folded,
            // so the original items stay byte for byte.
            continue;
        }
        let count = groups.len();
        for (group_index, group) in groups.iter().enumerate() {
            let Some(anchor) = group.first().copied() else {
                continue;
            };
            let Some(item) = regroup_item(&items, &scene_id, group, group_index, count) else {
                continue;
            };
            replacements.insert(anchor, item);
            for &member in group.iter().skip(1) {
                consumed.push(member);
            }
        }
        // A member that ended up in no group can only be a dropped tail; the
        // builder never drops a place, so this stays empty in practice.
        for &index in &indices {
            let mut kept = false;
            for group in &groups {
                if group.contains(&index) {
                    kept = true;
                    break;
                }
            }
            if !kept {
                consumed.push(index);
            }
        }
    }
    if replacements.is_empty() && consumed.is_empty() {
        return items;
    }
    let mut consumed_mask = vec![false; items.len()];
    for index in consumed {
        if let Some(flag) = consumed_mask.get_mut(index) {
            *flag = true;
        }
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        if consumed_mask[index] {
            continue;
        }
        out.push(match replacements.remove(&index) {
            Some(regrouped) => regrouped,
            None => item,
        });
    }
    out
}

fn is_map_item(item: &PlanItemV2) -> bool {
    item.kind.trim().eq_ignore_ascii_case("map") && item.map.is_some()
}

/// Builds the groups of one scene: narrative proximity first, then the pin cap
/// and the presentation ceiling, then the readable floor on the final group.
fn build_groups(
    items: &[PlanItemV2],
    indices: &[usize],
    cap: i64,
    group_nearby: bool,
) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    if group_nearby {
        for &index in indices {
            let mergeable = match groups.last() {
                Some(group) => can_join(items, group, index, cap),
                None => false,
            };
            if mergeable {
                groups.last_mut().expect("checked above").push(index);
            } else {
                groups.push(vec![index]);
            }
        }
    } else {
        groups = indices.iter().map(|&index| vec![index]).collect();
    }
    fold_short_tail(items, groups, cap)
}

/// True when one more place can share the group without breaking the pin cap,
/// the ceiling or the narrative proximity of the beat.
fn can_join(items: &[PlanItemV2], group: &[usize], candidate: usize, cap: i64) -> bool {
    if group.len() as i64 + 1 > cap {
        return false;
    }
    let (_, group_end) = group_window(items, group);
    let (candidate_start, candidate_end) = items[candidate].window_us();
    if candidate_start < group_end {
        // Overlapping maps are the same beat; merging is what stops them
        // fighting for the same frames.
        return candidate_end - group_start(items, group) <= MAP_PRESENTATION_CEILING_US;
    }
    if candidate_start - group_end > MAP_NARRATIVE_GAP_US {
        return false;
    }
    candidate_end - group_start(items, group) <= MAP_PRESENTATION_CEILING_US
}

fn group_start(items: &[PlanItemV2], group: &[usize]) -> i64 {
    group
        .iter()
        .map(|&index| items[index].window_us().0)
        .min()
        .unwrap_or(0)
}

fn group_window(items: &[PlanItemV2], group: &[usize]) -> (i64, i64) {
    let mut start = i64::MAX;
    let mut end = i64::MIN;
    for &index in group {
        let (item_start, item_end) = items[index].window_us();
        start = start.min(item_start);
        end = end.max(item_end);
    }
    if start == i64::MAX || end == i64::MIN {
        return (0, 0);
    }
    (start, end)
}

/// Folds a final group shorter than the readable floor into the preceding
/// group of the same scene. The tail is never dropped: when the preceding
/// group already holds the pin cap, its trailing pins move to the front of the
/// tail so both groups stay readable (the 0.94 s Campo tail becomes one
/// two-pin map instead of a flash).
fn fold_short_tail(items: &[PlanItemV2], mut groups: Vec<Vec<usize>>, cap: i64) -> Vec<Vec<usize>> {
    while groups.len() >= 2 {
        let tail = groups.last().expect("len checked").clone();
        let previous = groups[groups.len() - 2].clone();
        let (tail_start, tail_end) = group_window(items, &tail);
        if tail_end - tail_start >= MAP_READABLE_FLOOR_US {
            break;
        }
        let (previous_start, _) = group_window(items, &previous);
        if previous.len() as i64 + tail.len() as i64 > cap {
            let overflow = (previous.len() + tail.len()) as i64 - cap;
            if overflow <= 0 || overflow as usize >= previous.len() {
                break;
            }
            let split = previous.len() - overflow as usize;
            let moved = previous[split..].to_vec();
            let kept = previous[..split].to_vec();
            let mut widened = moved;
            widened.extend(tail.iter().copied());
            let (widened_start, widened_end) = group_window(items, &widened);
            if widened_end - widened_start < MAP_READABLE_FLOOR_US
                || widened_end - widened_start > MAP_PRESENTATION_CEILING_US
            {
                break;
            }
            let previous_index = groups.len() - 2;
            groups[previous_index] = kept;
            let last = groups.len() - 1;
            groups[last] = widened;
            break;
        }
        // Folding must actually buy readability: a merge that still lands
        // below the floor would only hide a flash inside a longer item and
        // silently steal the tail's pin window, so the tail stays its own map.
        if tail_end - previous_start < MAP_READABLE_FLOOR_US {
            break;
        }
        if tail_end - previous_start > MAP_PRESENTATION_CEILING_US {
            break;
        }
        let last = groups.len() - 1;
        let mut merged = previous;
        merged.extend(tail);
        groups[last - 1] = merged;
        groups.pop();
    }
    groups
}

/// Builds the grouped item. The anchor map object (the first place) keeps the
/// wire shape of a compact map candidate; the group itself travels in params.
fn regroup_item(
    items: &[PlanItemV2],
    scene_id: &str,
    group: &[usize],
    group_index: usize,
    group_count: usize,
) -> Option<PlanItemV2> {
    let anchor_index = *group.first()?;
    let mut item = items[anchor_index].clone();
    let (start_us, end_us) = group_window(items, group);
    item.id = format!("{scene_id}-map-{group_index}");
    item.start_us = start_us;
    item.duration_us = (end_us - start_us).max(0);
    item.start_ms = start_us / 1000;
    item.end_ms = (end_us + 999) / 1000;

    let mut pins: Vec<Value> = Vec::with_capacity(group.len());
    let mut timing = Map::new();
    let mut priority = 0.0_f64;
    for &index in group {
        let member = &items[index];
        let Some(map) = member.map.as_ref().and_then(Value::as_object) else {
            continue;
        };
        let entity_id = map
            .get("entity_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if entity_id.is_empty() {
            continue;
        }
        let label = map
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let scope = map.get("scope").and_then(Value::as_str).unwrap_or_default();
        let latitude = map.get("latitude").and_then(Value::as_f64).unwrap_or(0.0);
        let longitude = map.get("longitude").and_then(Value::as_f64).unwrap_or(0.0);
        let (pin_start, pin_end) = member.window_us();
        if pin_end <= pin_start {
            continue;
        }
        pins.push(json!({
            "entity_id": entity_id,
            "label": label,
            "scope": scope,
            "latitude": latitude,
            "longitude": longitude,
            "start_us": pin_start,
            "duration_us": pin_end - pin_start,
        }));
        // The schedule is relative to the map that draws it: the renderer
        // activates a pin at its offset inside the map's own window.
        timing.insert(
            entity_id,
            json!({
                "start_us": (pin_start - start_us).max(0),
                "duration_us": pin_end - pin_start,
            }),
        );
        if let Some(score) = member
            .params
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|params| params.get("priority"))
            .and_then(Value::as_f64)
        {
            priority = priority.max(score);
        }
    }
    if pins.is_empty() {
        return None;
    }
    let mut params = item
        .params
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    params.insert("priority".to_string(), json!(priority));
    params.insert(
        "map_group".to_string(),
        json!({
            "index": group_index + 1,
            "count": group_count,
            "size": pins.len(),
        }),
    );
    params.insert("map_group_pins".to_string(), Value::Array(pins));
    if !timing.is_empty() {
        params.insert("map_pin_timing".to_string(), Value::Object(timing));
    }
    item.params = Some(Value::Object(params));
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editorial_plan_v2::request::{
        CanvasV2, EditorialPlanV2Request, MapPresentationV2, PlanIdentityV2,
    };
    use serde_json::json;

    fn request(group_nearby: bool, cap: i64) -> EditorialPlanV2Request {
        EditorialPlanV2Request {
            operation: String::new(),
            schema_version: "editorial_plan.v2".to_string(),
            plan: PlanIdentityV2 {
                plan_id: "plan".to_string(),
                video_id: "video".to_string(),
                project_id: String::new(),
                motion_seed: String::new(),
                width: 1920,
                height: 1080,
                fps_num: 30,
                fps_den: 1,
            },
            language: "it".to_string(),
            source_language: String::new(),
            seal: None,
            canvas: CanvasV2 {
                map_presentation: MapPresentationV2 {
                    max_pins_per_group: cap,
                    group_nearby_places: group_nearby,
                    allow_route: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            editorial: None,
            chapters: None,
            image_plan: None,
            heavy_sentences: Vec::new(),
            numeric_lexicon: None,
            scenes: Vec::new(),
            clips: Vec::new(),
            recap_briefs: Vec::new(),
            entity_timeline: None,
            media: None,
            budgets: None,
            scene_windows: Default::default(),
        }
    }

    fn map_item(
        id: &str,
        entity: &str,
        scene: &str,
        start_us: i64,
        duration_us: i64,
    ) -> PlanItemV2 {
        PlanItemV2 {
            id: format!("{id}-map-0"),
            scene_id: scene.to_string(),
            kind: "map".to_string(),
            template_id: "MAP".to_string(),
            start_us,
            duration_us,
            start_ms: start_us / 1000,
            end_ms: (start_us + duration_us + 999) / 1000,
            map: Some(json!({
                "entity_id": entity,
                "label": entity.to_uppercase(),
                "scope": "city",
                "latitude": 41.9,
                "longitude": 12.5,
            })),
            params: Some(json!({"priority": 0.5})),
            ..Default::default()
        }
    }

    fn phrase_item(id: &str, start_us: i64) -> PlanItemV2 {
        PlanItemV2 {
            id: id.to_string(),
            scene_id: "s1".to_string(),
            kind: "phrase".to_string(),
            template_id: "IMPORTANT_PHRASE".to_string(),
            start_us,
            duration_us: 1_000_000,
            ..Default::default()
        }
    }

    #[test]
    fn nearby_maps_of_one_scene_share_one_item_with_their_pins() {
        let items = vec![
            map_item("a", "ent_a", "s1", 0, 3_000_000),
            map_item("b", "ent_b", "s1", 3_200_000, 3_000_000),
            phrase_item("p1", 7_000_000),
        ];
        let out = group_map_items(items, &request(true, 5));
        assert_eq!(2, out.len());
        assert_eq!("s1-map-0", out[0].id);
        let pins = out[0].params.as_ref().unwrap()["map_group_pins"]
            .as_array()
            .unwrap();
        assert_eq!(2, pins.len());
        assert_eq!("ent_a", pins[0]["entity_id"]);
        assert_eq!(6_200_000, out[0].duration_us);
        let timing = out[0].params.as_ref().unwrap()["map_pin_timing"]
            .as_object()
            .unwrap();
        assert_eq!(0, timing["ent_a"]["start_us"]);
        assert_eq!(3_200_000, timing["ent_b"]["start_us"]);
        assert_eq!("p1", out[1].id);
    }

    #[test]
    fn distant_maps_stay_separate() {
        let items = vec![
            map_item("a", "ent_a", "s1", 0, 3_000_000),
            map_item("b", "ent_b", "s1", 12_000_000, 3_000_000),
        ];
        let out = group_map_items(items, &request(true, 5));
        assert_eq!(2, out.len());
    }

    #[test]
    fn grouping_disabled_keeps_one_map_per_place() {
        let items = vec![
            map_item("a", "ent_a", "s1", 0, 3_000_000),
            map_item("b", "ent_b", "s1", 3_200_000, 3_000_000),
        ];
        let out = group_map_items(items, &request(false, 5));
        assert_eq!(2, out.len());
    }

    #[test]
    fn pin_cap_splits_one_beat_into_consecutive_maps() {
        let mut items = Vec::new();
        for index in 0..6 {
            items.push(map_item(
                &format!("m{index}"),
                &format!("ent_{index}"),
                "s1",
                index * 1_000_000,
                1_000_000,
            ));
        }
        let out = group_map_items(items, &request(true, 3));
        assert_eq!(2, out.len());
        assert_eq!(
            3,
            out[0].params.as_ref().unwrap()["map_group_pins"]
                .as_array()
                .unwrap()
                .len()
        );
        assert_eq!(2, out[0].params.as_ref().unwrap()["map_group"]["count"]);
    }

    #[test]
    fn a_merge_that_stays_below_the_floor_is_rejected() {
        // 1.167 s + 1.2 s = 2.4 s: still a flash, so the tail keeps its map.
        let items = vec![
            map_item("a", "ent_a", "s1", 0, 1_166_667),
            map_item("b", "ent_b", "s1", 1_200_000, 1_200_000),
        ];
        let out = group_map_items(items, &request(false, 5));
        assert_eq!(2, out.len());
    }

    #[test]
    fn short_tail_is_folded_into_the_previous_map() {
        let items = vec![
            map_item("a", "ent_a", "s1", 0, 3_000_000),
            map_item("b", "ent_b", "s1", 7_000_000, 900_000),
        ];
        let out = group_map_items(items, &request(false, 5));
        // Grouping is off, so the two places do not share a beat by proximity;
        // the readable floor still folds the 0.9 s tail into the previous map.
        assert_eq!(1, out.len());
        assert_eq!(7_900_000, out[0].duration_us);
        assert_eq!(
            2,
            out[0].params.as_ref().unwrap()["map_group_pins"]
                .as_array()
                .unwrap()
                .len()
        );
    }

    #[test]
    fn short_tail_at_the_pin_cap_moves_trailing_pins_instead_of_dropping_them() {
        let items = vec![
            map_item("a", "ent_a", "s1", 0, 2_000_000),
            map_item("b", "ent_b", "s1", 2_000_000, 2_000_000),
            map_item("c", "ent_c", "s1", 4_000_000, 2_000_000),
            map_item("d", "ent_d", "s1", 9_000_000, 800_000),
        ];
        // The first three places are one beat at the pin cap; the short tail
        // cannot join it without breaking the cap, so the trailing pin moves.
        let out = group_map_items(items, &request(true, 3));
        assert_eq!(2, out.len());
        let tail = out[1].params.as_ref().unwrap()["map_group_pins"]
            .as_array()
            .unwrap();
        assert_eq!(2, tail.len());
        assert_eq!("ent_c", tail[0]["entity_id"]);
        assert_eq!("ent_d", tail[1]["entity_id"]);
    }

    #[test]
    fn items_without_maps_are_untouched() {
        let items = vec![phrase_item("p1", 0)];
        let out = group_map_items(items, &request(true, 5));
        assert_eq!(1, out.len());
        assert!(out[0].params.is_none());
    }
}
