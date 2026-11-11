//! End-to-end image planning over a script and a catalog snapshot.

use imagecatalog::{
    plan_text, CatalogSnapshot, ImageDecision, ImagePlan, ImageQueryPlan, PLAN_SCHEMA,
};
use semantic_core::SceneInput;

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn scene(id: &str, text: &str) -> SceneInput {
    SceneInput {
        scene_id: id.to_string(),
        text: text.to_string(),
    }
}

fn catalog() -> CatalogSnapshot {
    serde_json::from_value(serde_json::json!({
        "entities": [
            {
                "canonical_entity_id": "person:alberto-stasi",
                "entity_type": "PERSON",
                "canonical_name": "Alberto Stasi"
            },
            {
                "canonical_entity_id": "person:andrea-sempio",
                "entity_type": "PERSON",
                "canonical_name": "Andrea Sempio"
            }
        ],
        "candidates": [
            {
                "candidate_id": 1,
                "canonical_entity_id": "person:alberto-stasi",
                "provider": "duckduckgo",
                "rank": 1,
                "source_url": "https://example.com/alberto-1.jpg",
                "width": 1200,
                "height": 800,
                "status": "fresh",
                "semantic_status": "accepted",
                "semantic_score": 0.92,
                "technical_score": 0.88,
                "last_seen_at": "2026-11-09T00:00:00Z"
            },
            {
                "candidate_id": 2,
                "canonical_entity_id": "person:alberto-stasi",
                "provider": "duckduckgo",
                "rank": 2,
                "source_url": "https://example.com/alberto-2.jpg",
                "width": 1200,
                "height": 800,
                "status": "fresh",
                "semantic_status": "accepted",
                "semantic_score": 0.90,
                "technical_score": 0.86,
                "last_seen_at": "2026-11-09T00:00:00Z"
            }
        ],
        "materializations": [
            {
                "candidate_id": 1,
                "asset_id": "asset-alberto-1",
                "content_sha256": SHA,
                "drive_link": "https://drive.example/alberto-1",
                "status": "materialized"
            }
        ]
    }))
    .expect("catalog fixture must match the existing schema")
}

fn entity_query<'a>(plan: &'a ImagePlan, canonical_id: &str) -> &'a ImageQueryPlan {
    plan.scenes
        .iter()
        .flat_map(|scene| &scene.queries)
        .find(|query| {
            query.query_kind == "entity_search"
                && query.canonical_entity_id.as_deref() == Some(canonical_id)
        })
        .unwrap_or_else(|| panic!("missing entity query for {canonical_id}: {plan:?}"))
}

#[test]
fn grounded_person_query_selects_the_existing_catalog_asset() {
    let text = "Alberto Stasi entra nel tribunale. Il dibattimento è iniziato.";
    let plan = plan_text(
        "it",
        vec![scene("s1", text)],
        &catalog(),
        Some("2026-11-10T00:00:00Z"),
    )
    .expect("planning succeeds");

    assert_eq!(plan.schema_version, PLAN_SCHEMA);
    let query = entity_query(&plan, "person:alberto-stasi");
    assert_eq!(query.decision, ImageDecision::Selected);
    assert!(
        text.contains(&query.query),
        "entity query must remain a grounded source slice: {query:?}"
    );
    let selection = query.selection.as_ref().expect("selection");
    assert_eq!(selection.candidate_id, 1);
    assert_eq!(selection.asset_id.as_deref(), Some("asset-alberto-1"));
    assert_eq!(selection.content_sha256.as_deref(), Some(SHA));
    assert!(selection.materialized);
    assert!(!selection.reused);
    assert_eq!(plan.counts.selected, 1);
}

#[test]
fn same_person_in_a_second_scene_prefers_an_unused_candidate() {
    let text = "Alberto Stasi entra nel tribunale.";
    let plan = plan_text(
        "it",
        vec![scene("s1", text), scene("s2", text)],
        &catalog(),
        Some("2026-11-10T00:00:00Z"),
    )
    .expect("planning succeeds");

    let first = entity_query(&plan, "person:alberto-stasi");
    let second_scene = &plan.scenes[1];
    let second = second_scene
        .queries
        .iter()
        .find(|query| query.canonical_entity_id.as_deref() == Some("person:alberto-stasi"))
        .expect("second scene query");
    let first_id = first
        .selection
        .as_ref()
        .expect("first selection")
        .candidate_id;
    let second_selection = second.selection.as_ref().expect("second selection");
    assert_ne!(
        first_id, second_selection.candidate_id,
        "a fresh unused candidate must be preferred"
    );
    assert_eq!(second_selection.candidate_id, 2);
    assert_eq!(plan.counts.reused_assets, 0);
}

#[test]
fn unknown_person_is_an_explicit_catalog_miss() {
    let plan = plan_text(
        "it",
        vec![scene("s1", "Alberto Stasi entra nel tribunale.")],
        &CatalogSnapshot::default(),
        Some("2026-11-10T00:00:00Z"),
    )
    .expect("an empty catalog is valid");
    let query = entity_query(&plan, "person:alberto-stasi");
    assert_eq!(query.decision, ImageDecision::CatalogMiss);
    assert!(query.reason.contains("no catalog entity"));
    assert!(query.selection.is_none());
    assert_eq!(plan.counts.catalog_misses, 1);
}

#[test]
fn non_person_and_contextual_queries_are_deferred_to_the_existing_path() {
    let plan = plan_text(
        "it",
        vec![scene(
            "s1",
            "Lo Stadio Aurora è un impianto sportivo di Torino.",
        )],
        &catalog(),
        Some("2026-11-10T00:00:00Z"),
    )
    .expect("planning succeeds");

    let contextual = plan
        .scenes
        .iter()
        .flat_map(|scene| &scene.queries)
        .find(|query| query.query_kind == "contextual_image_query")
        .expect("the grounded subject-place query must be present");
    assert_eq!(contextual.decision, ImageDecision::NotCatalogEligible);
    assert!(contextual.reason.contains("PERSON identity images only"));

    assert!(plan
        .scenes
        .iter()
        .flat_map(|scene| &scene.queries)
        .filter(|query| query.query_kind == "entity_search")
        .all(|query| query.decision == ImageDecision::NotCatalogEligible
            || query.decision == ImageDecision::CatalogMiss));
}

#[test]
fn replay_returns_the_same_plan_bytes() {
    let text = "Alberto Stasi entra nel tribunale.";
    let build = || {
        plan_text(
            "it",
            vec![scene("s1", text)],
            &catalog(),
            Some("2026-11-10T00:00:00Z"),
        )
        .expect("planning succeeds")
    };
    let first = serde_json::to_vec(&build()).expect("serialize");
    let second = serde_json::to_vec(&build()).expect("serialize");
    assert_eq!(first, second, "the image plan must replay byte-identically");
}
