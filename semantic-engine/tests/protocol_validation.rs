//! NDJSON protocol surface and request validation.

mod common;

use common::*;
use semantic_engine::{analyze_script, protocol::handle_line};
use serde_json::{json, Value};
#[test]
fn replay_of_the_same_script_gives_identical_manifest_and_fingerprint() {
    let scenes = vec![
        scene(
            "a",
            "Alberto Stasi e Andrea Sempio sono due procedimenti distinti.",
        ),
        scene("b", "Il 42% contro il 18% è nel fascicolo."),
    ];
    let first = serde_json::to_value(analyze_script(&request(scenes.clone())).unwrap()).unwrap();
    let second = serde_json::to_value(analyze_script(&request(scenes)).unwrap()).unwrap();

    assert_eq!(first, second);
    assert_eq!(first["schema_version"], "semantic_manifest.v2");
    assert_eq!(first["fingerprint"].as_str().unwrap().len(), 16);
    assert_eq!(first["analyzers"]["visualner"], "visualner");
}

#[test]
fn empty_scene_is_valid_and_empty() {
    let value = manifest_value(vec![scene("vuota", "")]);
    let scene = &value["scenes"][0];
    assert_eq!(scene["status"], "valid");
    assert!(scene["visual_proposals"].as_array().unwrap().is_empty());
}

#[test]
fn missing_language_and_duplicate_scene_ids_are_rejected() {
    let mut no_language = request(vec![scene("a", "Testo.")]);
    no_language.language = "  ".to_string();
    assert!(analyze_script(&no_language).is_err());

    let duplicate = request(vec![scene("a", "Testo."), scene("a", "Altro testo.")]);
    assert!(analyze_script(&duplicate)
        .unwrap_err()
        .contains("duplicate scene_id"));
}

#[test]
fn ndjson_protocol_serves_health_analysis_and_errors() {
    let health: Value = serde_json::from_str(&handle_line(r#"{"operation":"health"}"#)).unwrap();
    assert_eq!(health["ok"], true);

    let request = json!({
        "operation": "analyze_script",
        "language": "it",
        "scenes": [{"scene_id": "s1", "text": "Il 42% contro il 18%."}]
    });
    let response: Value = serde_json::from_str(&handle_line(&request.to_string())).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["manifest"]["scenes"][0]["scene_id"], "s1");

    let unknown: Value = serde_json::from_str(&handle_line(r#"{"operation":"render"}"#)).unwrap();
    assert_eq!(unknown["ok"], false);

    let broken: Value = serde_json::from_str(&handle_line("not json")).unwrap();
    assert_eq!(broken["ok"], false);
}
