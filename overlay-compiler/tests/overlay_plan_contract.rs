//! Contract suite for the `editorial_plan.v2` envelope (TICKET-14 WP0/WP1).
//!
//! The fixtures under `tests/fixtures/overlay_plan/` are the checked-in
//! planning-input matrix. They cover phrases only, map trim/overlap, entity
//! cards with and without verified media, editorial toggles with charts and
//! recaps, and a localized run with a short-phrase scene plus an untimed fixed
//! media scene. The plan fixture pins the response shape `contract` (the full
//! Go-oracle goldens land with WP5).
//!
//! A missing section, an unknown version, a duplicate identity, an unmaterialized
//! semantic placeholder or an item outside its scene must fail closed: the
//! compiler returns a typed error and never an empty plan.

use overlay_compiler::{
    compile_overlay_plan_v2, EditorialPlanError, EditorialPlanV2Compiled, EditorialPlanV2Request,
    PlanItemV2,
};

const REQUESTS: &[(&str, &str)] = &[
    (
        "minimal_phrase_only",
        include_str!("fixtures/overlay_plan/requests/minimal_phrase_only.request.json"),
    ),
    (
        "maps_trim_overlap",
        include_str!("fixtures/overlay_plan/requests/maps_trim_overlap.request.json"),
    ),
    (
        "entity_cards_verified_media",
        include_str!("fixtures/overlay_plan/requests/entity_cards_verified_media.request.json"),
    ),
    (
        "editorial_toggles_charts_recap",
        include_str!("fixtures/overlay_plan/requests/editorial_toggles_charts_recap.request.json"),
    ),
    (
        "localized_short_phrase_empty",
        include_str!("fixtures/overlay_plan/requests/localized_short_phrase_empty.request.json"),
    ),
];

const MINIMAL_REQUEST: &str =
    include_str!("fixtures/overlay_plan/requests/minimal_phrase_only.request.json");
const MINIMAL_PLAN: &str =
    include_str!("fixtures/overlay_plan/plans/minimal_phrase_only.plan.json");
const UNKNOWN_SCHEMA: &str = include_str!("fixtures/overlay_plan/malformed/unknown_schema.json");
const DUPLICATE_SCENE: &str = include_str!("fixtures/overlay_plan/malformed/duplicate_scene.json");
const UNMATERIALIZED_ASSET: &str =
    include_str!("fixtures/overlay_plan/malformed/unmaterialized_asset.json");

fn parse_request(fixture: &str) -> EditorialPlanV2Request {
    serde_json::from_str(fixture).expect("fixture is valid request JSON")
}

fn parse_plan(fixture: &str) -> EditorialPlanV2Compiled {
    serde_json::from_str(fixture).expect("fixture is valid plan JSON")
}

#[test]
fn fixture_matrix_parses_validates_and_round_trips() {
    for (name, fixture) in REQUESTS {
        let request = parse_request(fixture);
        request
            .validate()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        request
            .validate_compilable()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(request.schema_version, "editorial_plan.v2", "{name}");
        assert_eq!(request.operation, "compile_editorial_plan", "{name}");
        assert!(!request.scenes.is_empty(), "{name}");

        let encoded = serde_json::to_string(&request).expect("encode");
        let decoded: EditorialPlanV2Request =
            serde_json::from_str(&encoded).expect("decode canonical request");
        assert_eq!(decoded, request, "{name} must round-trip without loss");

        let again = serde_json::to_string(&decoded).expect("encode twice");
        assert_eq!(encoded, again, "{name} must serialize deterministically");
    }
}

#[test]
fn fixture_matrix_carries_every_compile_input() {
    for (name, fixture) in REQUESTS {
        let request = parse_request(fixture);
        assert!(request.seal.is_some(), "{name}: seal");
        assert!(request.media.is_some(), "{name}: media");
        assert!(request.budgets.is_some(), "{name}: budgets");
        assert!(request.entity_timeline.is_some(), "{name}: entity timeline");
        let scene = request.scenes.first().expect("scene");
        assert!(
            request.scene_windows.contains_key(&scene.id),
            "{name}: canonical scene window"
        );
    }
}

#[test]
fn full_plan_fixture_validates_against_its_request() {
    let request = parse_request(MINIMAL_REQUEST);
    let compiled = parse_plan(MINIMAL_PLAN);
    compiled
        .validate_for(&request)
        .expect("the plan belongs to the request");
    assert_eq!(compiled.plan.items.len(), 3);
    assert_eq!(compiled.plan.intake.planned, 3);
    assert_eq!(
        compiled.plan.output_profile_id,
        request.seal.as_ref().expect("seal").output_profile_id
    );
}

#[test]
fn compile_returns_a_validated_full_plan() {
    let request = parse_request(MINIMAL_REQUEST);
    let compiled = compile_overlay_plan_v2(&request).expect("planner compiles the fixture");
    compiled
        .validate_for(&request)
        .expect("the compiled plan belongs to the request");
    assert_eq!(compiled.ok, true);
    assert_eq!(compiled.schema_version, "editorial_plan.v2");
    assert!(!compiled.plan.items.is_empty());
    assert_eq!(
        compiled.plan.intake.planned,
        compiled.plan.items.len() as i64
    );
    assert_eq!(
        compiled.plan.output_profile_id,
        request.seal.as_ref().expect("seal").output_profile_id
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
    let request = parse_request(MINIMAL_REQUEST);
    let mut compiled = parse_plan(MINIMAL_PLAN);
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
    let mut compiled = parse_plan(MINIMAL_PLAN);
    let duplicate: PlanItemV2 = compiled.plan.items[0].clone();
    compiled.plan.items.push(duplicate);
    compiled.plan.intake.planned = compiled.plan.items.len() as i64;
    let error = compiled.validate().expect_err("duplicate id");
    assert!(error.to_string().contains("duplicate item id"));
}

#[test]
fn a_plan_that_omits_items_fails_the_intake_contract() {
    let mut compiled = parse_plan(MINIMAL_PLAN);
    compiled.plan.items.clear();
    let error = compiled.validate().expect_err("missing items");
    assert!(error.to_string().contains("final items"));
}

#[test]
fn a_missing_compile_section_fails_closed() {
    let mut request = parse_request(MINIMAL_REQUEST);
    request.budgets = None;
    let error = compile_overlay_plan_v2(&request).expect_err("missing budgets");
    assert!(matches!(
        error,
        EditorialPlanError::MissingSection("budgets")
    ));
}

#[test]
fn a_scene_window_that_disagrees_with_the_scene_is_rejected() {
    let mut request = parse_request(MINIMAL_REQUEST);
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
    let mut request = parse_request(MINIMAL_REQUEST);
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
    let request = parse_request(MINIMAL_REQUEST);
    let mut compiled = parse_plan(MINIMAL_PLAN);
    compiled.plan.plan_id = "someone-elses-plan".to_string();
    let error = compiled
        .validate_for(&request)
        .expect_err("foreign plan id");
    assert!(error.to_string().contains("does not match the request"));
}
