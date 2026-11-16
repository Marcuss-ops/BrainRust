//! Contract suite for the `editorial_plan.v2` envelope (TICKET-14 WP0/WP1).
//!
//! The fixtures under `tests/fixtures/overlay_plan/` are the checked-in
//! planning-input matrix: phrases only, map trim/overlap/resolution, entity
//! cards with and without verified media, editorial toggles with charts and a
//! chapter recap, and a localized run with a short-phrase scene plus an untimed
//! fixed-media scene. The plan files are produced by the real planner with
//! `cargo run -p overlay-compiler --example compile_fixture -- <request>`.
//!
//! A missing section, an unknown version, a duplicate identity, an unmaterialized
//! semantic placeholder or an item outside its scene must fail closed: the
//! compiler returns a typed error and never an empty plan.

use overlay_compiler::{
    compile_overlay_plan_v2, EditorialPlanError, EditorialPlanV2Compiled, EditorialPlanV2Request,
    PlanItemV2,
};

struct Fixture {
    name: &'static str,
    request: &'static str,
    plan: &'static str,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "minimal_phrase_only",
        request: include_str!("fixtures/overlay_plan/requests/minimal_phrase_only.request.json"),
        plan: include_str!("fixtures/overlay_plan/plans/minimal_phrase_only.plan.json"),
    },
    Fixture {
        name: "maps_trim_overlap",
        request: include_str!("fixtures/overlay_plan/requests/maps_trim_overlap.request.json"),
        plan: include_str!("fixtures/overlay_plan/plans/maps_trim_overlap.plan.json"),
    },
    Fixture {
        name: "entity_cards_verified_media",
        request: include_str!(
            "fixtures/overlay_plan/requests/entity_cards_verified_media.request.json"
        ),
        plan: include_str!("fixtures/overlay_plan/plans/entity_cards_verified_media.plan.json"),
    },
    Fixture {
        name: "editorial_toggles_charts_recap",
        request: include_str!(
            "fixtures/overlay_plan/requests/editorial_toggles_charts_recap.request.json"
        ),
        plan: include_str!("fixtures/overlay_plan/plans/editorial_toggles_charts_recap.plan.json"),
    },
    Fixture {
        name: "localized_short_phrase_empty",
        request: include_str!(
            "fixtures/overlay_plan/requests/localized_short_phrase_empty.request.json"
        ),
        plan: include_str!("fixtures/overlay_plan/plans/localized_short_phrase_empty.plan.json"),
    },
];

const UNKNOWN_SCHEMA: &str = include_str!("fixtures/overlay_plan/malformed/unknown_schema.json");
const DUPLICATE_SCENE: &str = include_str!("fixtures/overlay_plan/malformed/duplicate_scene.json");
const UNMATERIALIZED_ASSET: &str =
    include_str!("fixtures/overlay_plan/malformed/unmaterialized_asset.json");

fn fixture(name: &str) -> &'static Fixture {
    FIXTURES
        .iter()
        .find(|fixture| fixture.name == name)
        .expect("known fixture")
}

fn parse_request(fixture: &str) -> EditorialPlanV2Request {
    serde_json::from_str(fixture).expect("fixture is valid request JSON")
}

fn parse_plan(fixture: &str) -> EditorialPlanV2Compiled {
    serde_json::from_str(fixture).expect("fixture is valid plan JSON")
}

fn compile(name: &str) -> EditorialPlanV2Compiled {
    let entry = fixture(name);
    let request = parse_request(entry.request);
    compile_overlay_plan_v2(&request).unwrap_or_else(|error| panic!("{}: {error}", entry.name))
}

fn compile_json(name: &str) -> String {
    serde_json::to_string_pretty(&compile(name)).expect("encode plan")
}

#[test]
fn fixture_matrix_parses_validates_and_round_trips() {
    for entry in FIXTURES {
        let request = parse_request(entry.request);
        request
            .validate()
            .unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        request
            .validate_compilable()
            .unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        assert_eq!(
            request.schema_version, "editorial_plan.v2",
            "{}",
            entry.name
        );
        assert_eq!(
            request.operation, "compile_editorial_plan",
            "{}",
            entry.name
        );
        assert!(!request.scenes.is_empty(), "{}", entry.name);

        let encoded = serde_json::to_string(&request).expect("encode");
        let decoded: EditorialPlanV2Request =
            serde_json::from_str(&encoded).expect("decode canonical request");
        assert_eq!(
            decoded, request,
            "{} must round-trip without loss",
            entry.name
        );

        let again = serde_json::to_string(&decoded).expect("encode twice");
        assert_eq!(
            encoded, again,
            "{} must serialize deterministically",
            entry.name
        );
    }
}

#[test]
fn fixture_matrix_carries_every_compile_input() {
    for entry in FIXTURES {
        let request = parse_request(entry.request);
        assert!(request.seal.is_some(), "{}: seal", entry.name);
        assert!(request.media.is_some(), "{}: media", entry.name);
        assert!(request.budgets.is_some(), "{}: budgets", entry.name);
        assert!(
            request.entity_timeline.is_some(),
            "{}: entity timeline",
            entry.name
        );
        assert!(
            request.editorial.is_some(),
            "{}: certified editorial manifest",
            entry.name
        );
        let scene = request.scenes.first().expect("scene");
        assert!(
            request.scene_windows.contains_key(&scene.id),
            "{}: canonical scene window",
            entry.name
        );
    }
}

#[test]
fn every_fixture_compiles_to_its_frozen_golden() {
    for entry in FIXTURES {
        assert_eq!(
            compile_json(entry.name).trim(),
            entry.plan.trim(),
            "{} drifted from its checked-in plan; regenerate the fixture \
             with examples/compile_fixture only for an intended contract change",
            entry.name
        );
    }
}

#[test]
fn compilation_is_deterministic() {
    for entry in FIXTURES {
        assert_eq!(
            compile_json(entry.name),
            compile_json(entry.name),
            "{} must compile byte-identically across calls",
            entry.name
        );
    }
}

#[test]
fn every_golden_validates_against_its_request() {
    for entry in FIXTURES {
        let request = parse_request(entry.request);
        let compiled = parse_plan(entry.plan);
        compiled
            .validate_for(&request)
            .unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        assert_eq!(
            compiled.plan.intake.planned,
            compiled.plan.items.len() as i64,
            "{}",
            entry.name
        );
    }
}

#[test]
fn editorial_projection_emits_grounded_title_and_bullet() {
    let plan = compile("minimal_phrase_only").plan;
    assert_eq!(plan.items.len(), 3);
    let roles: Vec<&str> = plan
        .items
        .iter()
        .filter_map(|item| {
            item.params
                .as_ref()
                .and_then(|params| params.get("editorial_role"))
                .and_then(|role| role.as_str())
        })
        .collect();
    assert!(roles.contains(&"scene_title"));
    assert!(roles.contains(&"scene_bullet"));
    assert!(plan.items.iter().any(|item| item.text == "Nike corre"));
}

#[test]
fn map_windows_are_trimmed_before_budget_and_resolved_after_layout() {
    let plan = compile("maps_trim_overlap").plan;
    assert_eq!(plan.items.len(), 3, "the map lane survives admission");
    for item in &plan.items {
        assert_eq!(item.kind, "map");
        assert!(item.map.is_some(), "a map item always carries its map");
    }
    let mut windows: Vec<(i64, i64)> = plan.items.iter().map(PlanItemV2::window_us).collect();
    windows.sort_by_key(|window| window.0);
    for pair in windows.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "resolved map windows must not overlap: {pair:?}"
        );
    }
    let first = windows.first().expect("first map");
    assert!(
        first.1 - first.0 < 1_500_000,
        "the earlier map must be trimmed to the next map's frame boundary"
    );
}

#[test]
fn a_grounded_place_renders_once_as_a_map_not_a_location_card() {
    let plan = compile("maps_trim_overlap").plan;
    assert!(
        plan.items.iter().all(|item| item.kind == "map"),
        "no location card may duplicate a map pin"
    );
    assert!(plan.items.iter().any(|item| item
        .map
        .as_ref()
        .and_then(|map| map.get("label"))
        .and_then(|label| label.as_str())
        == Some("Milano")));
}

#[test]
fn verified_entity_media_becomes_an_image_and_missing_media_stays_text_only() {
    let plan = compile("entity_cards_verified_media").plan;
    let verified = plan
        .items
        .iter()
        .find(|item| item.entity_id == "org:nike")
        .expect("verified entity card");
    assert_eq!(verified.kind, "entity_image");
    assert_eq!(verified.asset_refs.len(), 1);
    assert_eq!(
        verified.asset_refs[0].sha256,
        "b".repeat(64),
        "the verified content address survives unchanged"
    );
    assert_eq!(verified.entity_caption, "Nike");
    assert_eq!(
        verified.motion_id, "entity_soft_rise",
        "the run offset rotates the certified entity pool"
    );
    assert_eq!(
        verified.caption_motion_id, "caption_fade",
        "the caption lane rotates independently"
    );
    assert_eq!(
        verified.entity_style_id, "premium_below_random_v1",
        "a captioned card is pinned to the below-caption composition"
    );
    assert_eq!(verified.caption_layout, "below");
    assert_eq!(verified.caption_font_family, "playfair_display_italic");
    assert_eq!(verified.caption_color, "#000000");
    assert_eq!(verified.image_preset_id, "image_scale_in");
    assert_eq!(
        verified
            .motion_params
            .as_ref()
            .and_then(|params| params.get("enter_frames"))
            .and_then(|frames| frames.as_i64()),
        Some(8),
        "entity cards enter on the certified eight-frame cascade"
    );

    let text_only = plan
        .items
        .iter()
        .find(|item| item.entity_id == "org:ferrari")
        .expect("text-only entity card");
    assert_eq!(text_only.kind, "organization");
    assert!(
        text_only.asset_refs.is_empty(),
        "an unverified entity must never become a fake image"
    );
    assert_eq!(text_only.text, "Ferrari");
}

#[test]
fn entity_card_motions_rotate_with_the_run_offset_only() {
    let entry = fixture("entity_cards_verified_media");
    let request = parse_request(entry.request);
    let first = compile_overlay_plan_v2(&request).expect("first compile");
    let mut shifted = request.clone();
    shifted.canvas.image_motion_offset += 1;
    let second = compile_overlay_plan_v2(&shifted).expect("shifted compile");

    let motion = |compiled: &EditorialPlanV2Compiled| {
        compiled
            .plan
            .items
            .iter()
            .find(|item| item.kind == "entity_image")
            .expect("entity image")
            .motion_id
            .clone()
    };
    assert_eq!(motion(&first), "entity_soft_rise");
    assert_eq!(motion(&second), "entity_reveal_push");
    assert_ne!(motion(&first), motion(&second));
}

#[test]
fn an_admitted_entity_image_without_a_motion_pool_fails_closed() {
    let mut request = parse_request(fixture("entity_cards_verified_media").request);
    request.canvas.entity_image_motions.clear();
    let error = compile_overlay_plan_v2(&request).expect_err("missing entity motion pool");
    assert!(
        error.to_string().contains("entity_image_motions"),
        "{error}"
    );
}

#[test]
fn an_admitted_caption_without_a_motion_pool_fails_closed() {
    let mut request = parse_request(fixture("entity_cards_verified_media").request);
    request.canvas.entity_caption_motions.clear();
    let error = compile_overlay_plan_v2(&request).expect_err("missing caption motion pool");
    assert!(
        error.to_string().contains("entity_caption_motions"),
        "{error}"
    );
}

#[test]
fn an_uncertified_entity_style_selector_is_rejected() {
    let mut request = parse_request(fixture("entity_cards_verified_media").request);
    request.canvas.entity_style_id = "not_a_certified_selector".to_string();
    let error = request.validate().expect_err("uncertified selector");
    assert!(error.to_string().contains("entity_style_id"), "{error}");
}

#[test]
fn charts_and_chapter_recaps_reach_the_final_plan() {
    let plan = compile("editorial_toggles_charts_recap").plan;
    assert_eq!(plan.chart_cards, 1, "one authorized chart reaches the seal");
    assert!(
        plan.items.iter().any(|item| item
            .params
            .as_ref()
            .and_then(|params| params.get("chart"))
            .and_then(|chart| chart.as_bool())
            == Some(true)),
        "the chart card is marked in params"
    );
    assert!(
        plan.items.iter().any(|item| item
            .params
            .as_ref()
            .and_then(|params| params.get("editorial_role"))
            .and_then(|role| role.as_str())
            == Some("chapter_bullet")),
        "the chapter bullet owns the covered scene's bullet slot"
    );
    assert!(
        plan.items.iter().all(|item| item.text != "Nike corre"),
        "the canvas disabled the titles category"
    );
}

#[test]
fn a_localized_run_keeps_target_language_surfaces_and_skips_untimed_scenes() {
    let plan = compile("localized_short_phrase_empty").plan;
    let texts: Vec<&str> = plan
        .items
        .iter()
        .map(|item| item.text.as_str())
        .filter(|text| !text.is_empty())
        .collect();
    assert_eq!(texts, vec!["Nike runs", "north"]);
    assert!(
        plan.items.iter().all(|item| item.scene_id != "s3"),
        "the untimed fixed-media scene contributes no item"
    );
    assert!(
        plan.items
            .iter()
            .all(|item| item.text != "Nike corre al museo."),
        "no source-language surface may leak into a localized plan"
    );
}

#[test]
fn unknown_schema_versions_fail_closed() {
    let fixture = parse_request(UNKNOWN_SCHEMA);
    assert!(matches!(
        fixture.validate().expect_err("unknown version"),
        EditorialPlanError::UnsupportedSchemaVersion { .. }
    ));
    assert!(matches!(
        compile_overlay_plan_v2(&fixture).expect_err("unknown version"),
        EditorialPlanError::UnsupportedSchemaVersion { .. }
    ));
}

#[test]
fn malformed_fixtures_fail_closed() {
    let duplicate = parse_request(DUPLICATE_SCENE);
    assert!(duplicate
        .validate()
        .expect_err("duplicate scene")
        .to_string()
        .contains("duplicate scene id"));

    let placeholder = parse_request(UNMATERIALIZED_ASSET);
    assert!(placeholder
        .validate()
        .expect_err("placeholder asset")
        .to_string()
        .contains("unmaterialized semantic placeholder"));
}

#[test]
fn a_legacy_sparse_batch_is_not_a_v2_request() {
    let legacy = r#"{
        "operation": "compile_editorial_plan",
        "fps_num": 24,
        "fps_den": 1,
        "scene_windows": {"s1": [0, 3000000]},
        "items": [],
        "entity_scenes": [],
        "entrance_items": []
    }"#;
    assert!(
        serde_json::from_str::<EditorialPlanV2Request>(legacy).is_err(),
        "the sparse batch must never deserialize into the full-plan envelope"
    );
}

#[test]
fn an_item_outside_its_scene_is_rejected() {
    let entry = fixture("minimal_phrase_only");
    let request = parse_request(entry.request);
    let mut compiled = parse_plan(entry.plan);
    let item = &mut compiled.plan.items[0];
    item.start_us = 3_500_000;
    item.duration_us = 500_000;
    let error = compiled
        .validate_for(&request)
        .expect_err("item escapes the scene");
    assert!(error.to_string().contains("escapes scene"));
}

#[test]
fn duplicate_item_identity_is_rejected() {
    let mut compiled = parse_plan(fixture("minimal_phrase_only").plan);
    let duplicate: PlanItemV2 = compiled.plan.items[0].clone();
    compiled.plan.items.push(duplicate);
    compiled.plan.intake.planned = compiled.plan.items.len() as i64;
    let error = compiled.validate().expect_err("duplicate id");
    assert!(error.to_string().contains("duplicate item id"));
}

#[test]
fn a_plan_that_omits_items_fails_the_intake_contract() {
    let mut compiled = parse_plan(fixture("minimal_phrase_only").plan);
    compiled.plan.items.clear();
    let error = compiled.validate().expect_err("missing items");
    assert!(error.to_string().contains("final items"));
}

#[test]
fn a_missing_compile_section_fails_closed() {
    let mut request = parse_request(fixture("minimal_phrase_only").request);
    request.budgets = None;
    let error = compile_overlay_plan_v2(&request).expect_err("missing budgets");
    assert!(matches!(
        error,
        EditorialPlanError::MissingSection("budgets")
    ));
}

#[test]
fn a_missing_editorial_manifest_fails_closed() {
    let mut request = parse_request(fixture("minimal_phrase_only").request);
    request.editorial = None;
    let error = compile_overlay_plan_v2(&request).expect_err("missing editorial");
    assert!(matches!(
        error,
        EditorialPlanError::MissingSection("editorial")
    ));
}

#[test]
fn a_scene_window_that_disagrees_with_the_scene_is_rejected() {
    let mut request = parse_request(fixture("minimal_phrase_only").request);
    request
        .scene_windows
        .insert("s1".to_string(), [0, 1_000_000]);
    let error = request.validate_compilable().expect_err("window mismatch");
    assert!(error
        .to_string()
        .contains("does not match the scene duration"));
}

#[test]
fn an_annotation_outside_its_scene_is_rejected() {
    let mut request = parse_request(fixture("minimal_phrase_only").request);
    let span = request.scenes[0]
        .annotations
        .as_mut()
        .expect("annotations")
        .phrases
        .first_mut()
        .expect("phrase");
    span.start_us = 2_900_000;
    span.duration_us = 900_000;
    let error = request.validate().expect_err("annotation escapes scene");
    assert!(error
        .to_string()
        .contains("escapes the owning scene window"));
}

#[test]
fn a_plan_with_a_foreign_identity_is_rejected() {
    let entry = fixture("minimal_phrase_only");
    let request = parse_request(entry.request);
    let mut compiled = parse_plan(entry.plan);
    compiled.plan.plan_id = "someone-elses-plan".to_string();
    let error = compiled
        .validate_for(&request)
        .expect_err("foreign plan id");
    assert!(error.to_string().contains("does not match the request"));
}

#[test]
fn a_plan_with_a_foreign_output_profile_is_rejected() {
    let entry = fixture("minimal_phrase_only");
    let request = parse_request(entry.request);
    let mut compiled = parse_plan(entry.plan);
    compiled.plan.output_profile_id = "someone-elses-profile".to_string();
    let error = compiled
        .validate_for(&request)
        .expect_err("foreign output profile");
    assert!(error.to_string().contains("output_profile_id"));
}
