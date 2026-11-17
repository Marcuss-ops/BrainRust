use semantic_engine::protocol::Worker;
use serde_json::{json, Value};

fn gate_payload(text: &str, word_count: i32) -> Value {
    json!({
        "operation": "quality_gate",
        "item": { "id": "item-1" },
        "plan": {
            "language": "en",
            "source_text": "The quick brown fox jumps over the lazy dog in Italy.",
            "target_words": 10
        },
        "result": {
            "output": {
                "text": text,
                "word_count": word_count
            }
        }
    })
}

#[test]
fn quality_gate_passes_a_clean_generation() {
    let worker = Worker::default();
    let response: Value = serde_json::from_str(&worker.handle_line(
        &gate_payload("The quick brown fox jumps over the lazy dog in Italy.", 10).to_string(),
    ))
    .expect("json response");

    assert_eq!(response["ok"], true);
    assert_eq!(response["quality"]["passed"], true);
    assert_eq!(response["quality"]["language_detected"], "en");
    assert_eq!(response["quality"]["actual_words"], 10);
    assert_eq!(response["quality"]["target_words"], 10);
    assert_eq!(response["reasons"], json!([]));
}

#[test]
fn quality_gate_returns_failure_reasons_without_protocol_error() {
    let worker = Worker::default();
    let response: Value =
        serde_json::from_str(&worker.handle_line(&gate_payload("   ", 0).to_string()))
            .expect("json response");

    assert_eq!(response["ok"], true);
    assert_eq!(response["quality"]["passed"], false);
    assert_eq!(response["reasons"][0], "generated text is empty");
}

#[test]
fn quality_gate_rejects_malformed_requests() {
    let worker = Worker::default();

    // Invalid JSON is rejected by the worker envelope before dispatch.
    let response: Value =
        serde_json::from_str(&worker.handle_line(r#"{"operation":"quality_gate","plan":"#))
            .expect("json response");
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .is_some_and(|error| error.starts_with("invalid request:")));

    // A valid envelope with a wrong nested shape reaches the quality-gate
    // handler and gets the operation-specific error.
    let response: Value = serde_json::from_str(
        &worker.handle_line(r#"{"operation":"quality_gate","plan":"not-an-object"}"#),
    )
    .expect("json response");
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .is_some_and(|error| error.starts_with("invalid quality_gate request:")));
}
