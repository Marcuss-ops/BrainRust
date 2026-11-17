//! Protocol contract for `lexicon_profile` (TICKET-18).
//!
//! The Go bootstrap consumes a generated snapshot, but the Rust worker must
//! still expose the same named-set profile so the contract is testable and a
//! future consumer never has to infer sets from the fused extractor pack.

use semantic_engine::protocol::handle_line;
use serde_json::{json, Value};

#[test]
fn returns_one_language_with_named_sets_and_policy() {
    let line = handle_line(r#"{"operation":"lexicon_profile","language":"it"}"#);
    let response: Value = serde_json::from_str(&line).expect("json response");
    assert_eq!(response["ok"], json!(true));
    assert!(response["schema_version"]
        .as_str()
        .unwrap_or_default()
        .starts_with("language_profiles.v1-"));
    let profile = &response["profile"];
    assert_eq!(profile["language"], json!("it"));
    assert_eq!(profile["phrase_policy"]["max_words"], json!(6));
    assert!(!profile["stopwords"].as_array().unwrap().is_empty());
    assert!(!profile["verb_suffixes"].as_array().unwrap().is_empty());
    assert!(profile["word_sets"]["pronouns"].is_array());
    assert!(profile["word_sets"]["subordinate_markers"].is_array());
}

#[test]
fn returns_every_profile_including_fallback() {
    let line = handle_line(r#"{"operation":"lexicon_profile"}"#);
    let response: Value = serde_json::from_str(&line).expect("json response");
    assert_eq!(response["ok"], json!(true));
    let profiles = response["profiles"].as_array().expect("profiles");
    assert!(profiles.iter().any(|p| p["language"] == json!("fallback")));
    assert!(profiles.iter().any(|p| p["language"] == json!("en")));
    assert_eq!(
        profiles
            .iter()
            .filter(|p| p["schema"] == json!("language_profile.v1"))
            .count(),
        profiles.len()
    );
}

#[test]
fn unknown_language_fails_closed() {
    let line = handle_line(r#"{"operation":"lexicon_profile","language":"zz"}"#);
    let response: Value = serde_json::from_str(&line).expect("json response");
    assert_eq!(response["ok"], json!(false));
    assert!(response["error"]
        .as_str()
        .unwrap_or_default()
        .contains("unknown language"));
}

#[test]
fn malformed_request_fails_closed() {
    let line = handle_line(r#"{"operation":"lexicon_profile","language":12}"#);
    let response: Value = serde_json::from_str(&line).expect("json response");
    assert_eq!(response["ok"], json!(false));
    assert!(response["error"]
        .as_str()
        .unwrap_or_default()
        .contains("invalid lexicon_profile request"));
}
