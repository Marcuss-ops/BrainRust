use super::{
    cascade_clustered_entrances, compile_editorial_plan, rank_entity_overlays,
    resolve_map_overlaps, trim_map_windows_before_budget, CascadeEntranceItem,
    CascadeEntrancesRequest, CompileEditorialPlanRequest, EntityRankingInput, EntityRankingScene,
    RankEntityOverlaysRequest, ResolveMapOverlapsRequest, TimingItem,
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
        map_presentation: None,
    }
}

fn presentation_request(
    items: Vec<TimingItem>,
    scene_end_us: i64,
    plan_end_us: i64,
) -> ResolveMapOverlapsRequest {
    ResolveMapOverlapsRequest {
        fps_num: 24,
        fps_den: 1,
        scene_windows: BTreeMap::from([("scene-1".to_owned(), [0, scene_end_us])]),
        items,
        map_presentation: Some(crate::MapPresentationWindow {
            camera_duration_us: 8_000_000,
            camera_lead_us: 2_000_000,
            pin_read_duration_us: 1_000_000,
            readable_floor_us: 2_500_000,
            presentation_ceiling_us: 10_000_000,
            plan_end_us,
        }),
    }
}

fn map_item(id: &str, start_us: i64, duration_us: i64, pins: usize) -> TimingItem {
    let pins = (0..pins)
        .map(|index| json!({"entity_id": format!("pin-{index}")}))
        .collect::<Vec<_>>();
    let mut item = item(id, start_us, duration_us, true);
    item.params = Some(json!({"map_group_pins": pins}));
    item
}

#[test]
fn map_presentation_extends_a_short_map_to_the_camera_minimum() {
    let edits = resolve_map_overlaps(presentation_request(
        vec![map_item("map-a", 1_000_000, 1_166_667, 1)],
        40_000_000,
        40_000_000,
    ));
    assert_eq!(edits[0].duration_us, 8_000_000);
    assert_eq!(edits[0].end_ms, 9_000);
}

#[test]
fn map_presentation_grows_with_the_pin_slots() {
    let edits = resolve_map_overlaps(presentation_request(
        vec![map_item("map-a", 0, 3_000_000, 7)],
        40_000_000,
        40_000_000,
    ));
    assert_eq!(edits[0].duration_us, 9_000_000);
}

#[test]
fn readable_floor_shifts_the_next_map_instead_of_squeezing_the_current_one() {
    let edits = resolve_map_overlaps(presentation_request(
        vec![
            map_item("map-a", 20_569_000, 8_000_000, 1),
            map_item("map-b", 21_233_000, 8_001_000, 1),
        ],
        40_000_000,
        40_000_000,
    ));
    assert_eq!(edits[0].duration_us, 2_500_000);
    assert_eq!(edits[1].start_us, 23_069_000);
    assert_eq!(edits[1].duration_us, 8_001_000);
}

#[test]
fn readable_floor_yields_when_the_shift_would_overrun_the_plan() {
    let edits = resolve_map_overlaps(presentation_request(
        vec![
            map_item("map-a", 20_569_000, 8_000_000, 1),
            map_item("map-b", 21_233_000, 8_001_000, 1),
        ],
        24_000_000,
        24_000_000,
    ));
    assert_eq!(edits[0].duration_us, 21_233_000 - 20_569_000);
    assert_eq!(edits[1].start_us, 21_233_000);
    assert_eq!(edits[1].start_us + edits[1].duration_us, 24_000_000);
}

#[test]
fn the_tail_map_never_outlives_the_plan() {
    let edits = resolve_map_overlaps(presentation_request(
        vec![map_item("map-a", 20_000_000, 8_000_000, 1)],
        24_000_000,
        24_000_000,
    ));
    assert_eq!(edits[0].duration_us, 4_000_000);
    assert_eq!(edits[0].end_ms, 24_000);
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
        v2: None,
    };
    let plan = compile_editorial_plan(request.clone());
    assert_eq!(
        plan.pre_budget_edits,
        trim_map_windows_before_budget(ResolveMapOverlapsRequest {
            fps_num: request.fps_num,
            fps_den: request.fps_den,
            scene_windows: request.scene_windows.clone(),
            items: request.items.clone(),
            map_presentation: None,
        })
    );
    assert_eq!(
        plan.timing_edits,
        resolve_map_overlaps(ResolveMapOverlapsRequest {
            fps_num: request.fps_num,
            fps_den: request.fps_den,
            scene_windows: request.scene_windows.clone(),
            items: request.items.clone(),
            map_presentation: None,
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
        v2: None,
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
