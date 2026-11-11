//! A caller that already owns a semantic manifest can reuse it both in-process
//! and over the worker boundary without a second analysis.

use imagecatalog::{
    plan, plan_from_manifest, CatalogIndex, CatalogSnapshot, ImageCatalogRequest, REQUEST_SCHEMA,
};
use semantic_core::{SceneInput, ScriptRequest};

#[test]
fn plan_from_manifest_reuses_the_manifest_and_reports_rejected_scenes() {
    let text = "Alberto Stasi entra nel tribunale.";
    let mut manifest = semantic_engine::analyze_script(&ScriptRequest {
        language: "it".to_string(),
        scenes: vec![SceneInput {
            scene_id: "s1".to_string(),
            text: text.to_string(),
        }],
        editorial: None,
        entity_safety_cap: 0,
    })
    .expect("semantic manifest");
    manifest.scenes[0].status = "rejected".to_string();

    let catalog = CatalogSnapshot::default();
    let plan = plan_from_manifest(
        &manifest,
        &CatalogIndex::new(&catalog).expect("catalog snapshot"),
        0,
    );

    assert_eq!(plan.fingerprint, manifest.fingerprint);
    assert_eq!(plan.evaluated_at, 0);
    assert_eq!(plan.counts.scenes, 1);
    assert_eq!(plan.counts.rejected_scenes, 1);
    assert_eq!(plan.scenes.len(), 1);
    assert!(plan.scenes[0].queries.is_empty());
}

fn analysed_manifest(text: &str) -> semantic_core::SemanticManifest {
    semantic_engine::analyze_script(&ScriptRequest {
        language: "it".to_string(),
        scenes: vec![SceneInput {
            scene_id: "s1".to_string(),
            text: text.to_string(),
        }],
        editorial: None,
        entity_safety_cap: 0,
    })
    .expect("semantic manifest")
}

fn wire_request(manifest: semantic_core::SemanticManifest) -> ImageCatalogRequest {
    serde_json::from_value(serde_json::json!({
        "version": REQUEST_SCHEMA,
        "operation": "plan",
        "language": "it",
        "scenes": [{"scene_id": "s1", "text": "Alberto Stasi entra nel tribunale."}],
        "manifest": manifest,
        "catalog": CatalogSnapshot::default(),
        "now": "2026-11-10T00:00:00Z"
    }))
    .expect("wire request with a semantic manifest")
}

#[test]
fn wire_manifest_is_reused_without_a_second_analysis() {
    let mut manifest = analysed_manifest("Alberto Stasi entra nel tribunale.");
    // The fingerprint covers the request, not the extractor's scene status. A
    // changed status therefore proves the plan used the supplied manifest: a
    // second analysis would produce `valid` and a query.
    manifest.scenes[0].status = "rejected".to_string();
    manifest.scenes[0].reason = Some("reviewed rejection".to_string());

    let plan = plan(&wire_request(manifest)).expect("manifest is reusable");
    assert_eq!(plan.counts.rejected_scenes, 1);
    assert!(plan.scenes[0].queries.is_empty());
}

#[test]
fn wire_manifest_with_the_wrong_schema_is_rejected() {
    let mut manifest = analysed_manifest("Alberto Stasi entra nel tribunale.");
    manifest.schema_version = "semantic_manifest.v1".to_string();

    let error = plan(&wire_request(manifest)).expect_err("schema must fail closed");
    assert!(error.contains("schema_version"), "{error}");
}

#[test]
fn wire_manifest_with_a_foreign_fingerprint_is_rejected() {
    let mut manifest = analysed_manifest("Alberto Stasi entra nel tribunale.");
    manifest.fingerprint = "0000000000000000".to_string();

    let error = plan(&wire_request(manifest)).expect_err("fingerprint must fail closed");
    assert!(error.contains("fingerprint"), "{error}");
}

#[test]
fn wire_manifest_with_a_foreign_language_is_rejected() {
    let mut manifest = analysed_manifest("Alberto Stasi entra nel tribunale.");
    manifest.language = "en".to_string();

    let error = plan(&wire_request(manifest)).expect_err("language must fail closed");
    assert!(error.contains("language"), "{error}");
}

#[test]
fn wire_manifest_with_a_different_scene_count_is_rejected() {
    let mut manifest = analysed_manifest("Alberto Stasi entra nel tribunale.");
    manifest.scenes.clear();

    let error = plan(&wire_request(manifest)).expect_err("scene count must fail closed");
    assert!(error.contains("scenes"), "{error}");
}

#[test]
fn wire_manifest_with_a_different_scene_id_is_rejected() {
    let mut manifest = analysed_manifest("Alberto Stasi entra nel tribunale.");
    manifest.scenes[0].scene_id = "other".to_string();

    let error = plan(&wire_request(manifest)).expect_err("scene id must fail closed");
    assert!(error.contains("scene 0 id"), "{error}");
}
