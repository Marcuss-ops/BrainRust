//! Replay tests over the real catalog seed and its visual-review overlay.
//!
//! The seed is a slice of `entity_image_catalog_*` rows copied read-only from
//! the existing PipelineGen database. It is a fixture, not a second catalog:
//! every candidate and materialization keeps its durable id and SHA-256.

use imagecatalog::{plan_text, CatalogSnapshot, ImageDecision, ImagePlan, ImageQueryPlan};
use semantic_core::SceneInput;

const SEED: &str = include_str!("../fixtures/catalog_seed_20261110.json");

fn catalog() -> CatalogSnapshot {
    serde_json::from_str(SEED).expect("the live catalog seed must match imagecatalog.v1")
}

fn scene(id: &str, text: &str) -> SceneInput {
    SceneInput {
        scene_id: id.to_string(),
        text: text.to_string(),
    }
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
fn a_solo_chiara_query_prefers_the_reviewed_portrait_over_composites() {
    let plan = plan_text(
        "it",
        vec![scene("s1", "Chiara Poggi entra nel tribunale.")],
        &catalog(),
        Some("2026-11-11T00:00:00Z"),
    )
    .expect("planning succeeds");

    let query = entity_query(&plan, "person:chiara-poggi");
    assert_eq!(query.decision, ImageDecision::Selected);
    let selection = query.selection.as_ref().expect("selection");
    assert_eq!(
        selection.candidate_id, 49960,
        "a reviewed single portrait must beat composite collages"
    );
    assert!(selection.materialized);
    assert_eq!(
        selection.identity_status.as_deref(),
        Some("verified_visual")
    );
    assert_eq!(selection.depiction.as_deref(), Some("photograph"));
    assert!(selection
        .subjects
        .contains(&"person:chiara-poggi".to_string()));
    assert!(selection
        .description
        .as_deref()
        .is_some_and(|description| description.contains("Ritratto di Chiara Poggi")));

    let mismatch = query
        .candidates
        .iter()
        .find(|candidate| candidate.candidate_id == 23268)
        .expect("the reviewed mismatch must remain auditable");
    assert!(!mismatch.eligible);
    assert!(
        mismatch.reason.contains("mismatch"),
        "the wrong catalog label must be explained: {mismatch:?}"
    );
}

#[test]
fn a_scene_with_both_people_promotes_the_triptych_by_subject_overlap() {
    let plan = plan_text(
        "it",
        vec![scene(
            "s1",
            "Chiara Poggi e Alberto Stasi arrivano in tribunale.",
        )],
        &catalog(),
        Some("2026-11-11T00:00:00Z"),
    )
    .expect("planning succeeds");

    let chiara = entity_query(&plan, "person:chiara-poggi");
    assert_eq!(chiara.decision, ImageDecision::Selected);
    let selection = chiara.selection.as_ref().expect("selection");
    assert_eq!(
        selection.candidate_id, 34357,
        "the triptych lists both scene identities and should win the context bonus"
    );
    assert_eq!(selection.depiction.as_deref(), Some("composite"));
    for subject in [
        "person:andrea-sempio",
        "person:chiara-poggi",
        "person:alberto-stasi",
    ] {
        assert!(
            selection.subjects.contains(&subject.to_string()),
            "missing {subject} in {selection:?}"
        );
    }

    let stasi = entity_query(&plan, "person:alberto-stasi");
    assert_eq!(stasi.decision, ImageDecision::Selected);
    assert_eq!(
        stasi
            .selection
            .as_ref()
            .expect("Stasi selection")
            .candidate_id,
        23462,
        "the solo Stasi portrait remains the best unused candidate"
    );
}

#[test]
fn the_live_fixture_replays_byte_identically() {
    let build = || {
        plan_text(
            "it",
            vec![scene("s1", "Chiara Poggi entra nel tribunale.")],
            &catalog(),
            Some("2026-11-11T00:00:00Z"),
        )
        .expect("planning succeeds")
    };
    let first = serde_json::to_vec(&build()).expect("serialize");
    let second = serde_json::to_vec(&build()).expect("serialize");
    assert_eq!(first, second);
}
