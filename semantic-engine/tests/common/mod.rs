//! Shared request/manifest helpers for the semantic-engine integration tests.

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};
use serde_json::Value;

pub fn scene(id: &str, text: &str) -> SceneInput {
    SceneInput {
        scene_id: id.to_string(),
        text: text.to_string(),
    }
}

pub fn request(scenes: Vec<SceneInput>) -> ScriptRequest {
    ScriptRequest {
        language: "it".to_string(),
        scenes,
        editorial: None,
        entity_safety_cap: 0,
    }
}

pub fn manifest_value(scenes: Vec<SceneInput>) -> Value {
    let manifest = analyze_script(&request(scenes)).expect("analysis must succeed");
    serde_json::to_value(manifest).expect("manifest serializes")
}

// Shared across topic test binaries; not every binary uses every helper.
#[allow(dead_code)]
pub fn proposals_of<'a>(scene: &'a Value, kind: &str) -> Vec<&'a Value> {
    scene["visual_proposals"]
        .as_array()
        .expect("visual_proposals array")
        .iter()
        .filter(|c| c["proposal_type"] == kind)
        .collect()
}
