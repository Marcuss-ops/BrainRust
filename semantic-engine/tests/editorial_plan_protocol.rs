//! Protocol routing for `compile_editorial_plan` (TICKET-14 WP1).
//!
//! The sparse legacy batch and the complete `editorial_plan.v2` request share
//! one wire operation name. The worker must route on the declared version
//! before deserializing: a v2 request interpreted as the legacy batch would
//! default every section and silently return an empty plan.

use std::io::Write;
use std::process::{Command, Stdio};

use semantic_engine::protocol::handle_line;

const MINIMAL_V2: &str = include_str!(
    "../../overlay-compiler/tests/fixtures/overlay_plan/requests/minimal_phrase_only.request.json"
);

fn handle(request: &str) -> serde_json::Value {
    let line = handle_line(request);
    serde_json::from_str(line.trim()).expect("worker returns one JSON object")
}

#[test]
fn v2_requests_are_routed_to_the_full_plan_compiler() {
    let response = handle(MINIMAL_V2);
    assert_eq!(response["ok"], true, "the full-plan compiler must run");
    assert_eq!(response["schema_version"], "editorial_plan.v2");
    assert_eq!(response["plan"]["schema_version"], "editorial_plan.v2");
    assert_eq!(response["plan"]["plan_id"], "min-fixture");
    let items = response["plan"]["items"].as_array().expect("items");
    assert_eq!(2, items.len(), "title filter leaves bullet + entity image");
    assert_eq!(items[0]["kind"], "text_phrase");
    assert_eq!(items[1]["kind"], "entity_image");
    assert_eq!(items[1]["template_id"], "image_popup");
    assert!(
        response.get("plan").is_some(),
        "a v2 request must never degrade to the legacy sparse plan"
    );
}

#[test]
fn an_unknown_schema_version_is_rejected_before_deserializing() {
    let mut request: serde_json::Value = serde_json::from_str(MINIMAL_V2).expect("fixture");
    request["schema_version"] = serde_json::json!("editorial_plan.v9");
    let response = handle(&request.to_string());
    assert_eq!(response["ok"], false);
    let error = response["error"].as_str().expect("typed error");
    assert!(error.contains("unsupported"), "got {error:?}");
    assert!(error.contains("editorial_plan.v9"), "got {error:?}");
    assert!(response.get("plan").is_none());
}

#[test]
fn a_malformed_v2_request_fails_closed() {
    let mut request: serde_json::Value = serde_json::from_str(MINIMAL_V2).expect("fixture");
    let scene = request["scenes"][0].clone();
    request["scenes"]
        .as_array_mut()
        .expect("scenes")
        .push(scene);
    let response = handle(&request.to_string());
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .expect("typed error")
        .contains("duplicate scene id"));
}

#[test]
fn the_sparse_legacy_batch_still_compiles() {
    let legacy = r#"{
        "operation": "compile_editorial_plan",
        "fps_num": 24,
        "fps_den": 1,
        "scene_windows": {"s1": [0, 3000000]},
        "items": [],
        "entity_scenes": [],
        "entrance_items": []
    }"#;
    let response = handle(legacy);
    assert_eq!(response["ok"], true);
    assert_eq!(response["plan"]["pre_budget_edits"], serde_json::json!([]));
    assert_eq!(response["plan"]["timing_edits"], serde_json::json!([]));
    assert_eq!(response["plan"]["ranked_entities"], serde_json::json!([]));
    assert_eq!(response["plan"]["entrance_edits"], serde_json::json!([]));
}

#[test]
fn a_v2_request_never_returns_an_empty_plan() {
    let response = handle(MINIMAL_V2);
    let items = response["plan"]["items"].as_array().expect("items");
    assert!(!items.is_empty(), "the fixture compiles admitted items");
    assert!(
        response["plan"].get("pre_budget_edits").is_none(),
        "a v2 request must not degrade to the legacy sparse plan"
    );
}

#[test]
fn the_real_worker_answers_exactly_one_v2_compile_request() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_semantic-engine"))
        .env("SEMANTIC_ENGINE_WORKERS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn semantic-engine");
    {
        let request: serde_json::Value =
            serde_json::from_str(MINIMAL_V2).expect("v2 fixture parses");
        let mut stdin = child.stdin.take().expect("worker stdin");
        stdin
            .write_all(request.to_string().as_bytes())
            .expect("write request");
        stdin.write_all(b"\n").expect("terminate request line");
    }
    let output = child.wait_with_output().expect("worker exits on EOF");
    assert!(
        output.status.success(),
        "worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 worker output");
    let responses: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        responses.len(),
        1,
        "one compile_editorial_plan call must produce exactly one response: {stdout}"
    );
    let response: serde_json::Value =
        serde_json::from_str(responses[0]).expect("worker returns one JSON object");
    assert_eq!(response["ok"], true, "worker rejected request: {response}");
    assert_eq!(response["schema_version"], "editorial_plan.v2");
    assert_eq!(response["plan"]["plan_id"], "min-fixture");
    let items = response["plan"]["items"].as_array().expect("items");
    assert_eq!(2, items.len(), "title filter leaves bullet + entity image");
    assert_eq!(items[0]["kind"], "text_phrase");
    assert_eq!(items[1]["kind"], "entity_image");
}
