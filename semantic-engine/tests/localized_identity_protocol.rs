//! Protocol contract for `localize_identities` (TICKET-20).
//!
//! The Go seam sends the source identities and the translated text; Rust owns
//! exact matching, pl/de/tr inflection and the ru/tr/pl ordered fallback. The
//! response carries source indices and rune spans, never a re-minted id.

use semantic_engine::protocol::handle_line;
use serde_json::{json, Value};

#[test]
fn match_returns_the_polish_inflected_surface() {
    let request = json!({
        "operation": "localize_identities",
        "language": "pl",
        "text": "To opowieść o Mike’a Tysona.",
        "sources": [
            {"kind": "PERSON", "canonical_name": "Mike Tyson", "text": "Mike Tyson"}
        ]
    });
    let response: Value =
        serde_json::from_str(&handle_line(&request.to_string())).expect("json response");
    assert_eq!(response["ok"], json!(true));
    let matches = response["matches"].as_array().expect("matches");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["source_index"], json!(0));
    assert_eq!(matches[0]["surface"], json!("Mike’a Tysona"));
}

#[test]
fn complete_orders_russian_person_candidates() {
    let request = json!({
        "operation": "localize_identities",
        "mode": "complete",
        "language": "ru",
        "text": "Майк Тайсон встретил Ивана Петрова.",
        "sources": [
            {"kind": "PERSON", "canonical_name": "Mike Tyson", "text": "Mike Tyson"},
            {"kind": "PERSON", "canonical_name": "Ivan Petrov", "text": "Ivan Petrov"}
        ],
        "visual": [
            {"text": "Майк Тайсон"},
            {"text": "Ивана Петрова"}
        ]
    });
    let response: Value =
        serde_json::from_str(&handle_line(&request.to_string())).expect("json response");
    assert_eq!(response["ok"], json!(true));
    let matches = response["matches"].as_array().expect("matches");
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["surface"], json!("Майк Тайсон"));
    assert_eq!(matches[1]["surface"], json!("Ивана Петрова"));
    assert_eq!(matches[0]["source_index"], json!(0));
    assert_eq!(matches[1]["source_index"], json!(1));
}

#[test]
fn unknown_mode_fails_closed() {
    let request = json!({
        "operation": "localize_identities",
        "mode": "rewrite",
        "language": "it",
        "text": "Mike Tyson"
    });
    let response: Value =
        serde_json::from_str(&handle_line(&request.to_string())).expect("json response");
    assert_eq!(response["ok"], json!(false));
    assert!(response["error"]
        .as_str()
        .unwrap_or_default()
        .contains("unsupported mode"));
}

#[test]
fn missing_language_fails_closed() {
    let response: Value = serde_json::from_str(&handle_line(
        r#"{"operation":"localize_identities","text":"Mike Tyson"}"#,
    ))
    .expect("json response");
    assert_eq!(response["ok"], json!(false));
    assert!(response["error"]
        .as_str()
        .unwrap_or_default()
        .contains("language is required"));
}

#[test]
fn malformed_request_fails_closed() {
    let response: Value = serde_json::from_str(&handle_line(
        r#"{"operation":"localize_identities","language":12}"#,
    ))
    .expect("json response");
    assert_eq!(response["ok"], json!(false));
    assert!(response["error"]
        .as_str()
        .unwrap_or_default()
        .contains("invalid localize_identities request"));
}
