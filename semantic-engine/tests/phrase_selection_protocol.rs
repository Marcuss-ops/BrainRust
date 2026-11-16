//! Protocol contract for `select_phrases` (TICKET-15 WP1).
//!
//! The fixtures are the ones the Go selector's suite pins in
//! `internal/capabilities/scripts/phrases/selection_test.go`: same text, same
//! profile, same blocked rune spans, same expected order. Rust is the owner of
//! the decision, so the wire must reproduce the accepted Go surfaces before any
//! caller is switched over.

use std::io::Write;
use std::process::{Command, Stdio};

use semantic_engine::protocol::handle_line;

fn handle(request: serde_json::Value) -> serde_json::Value {
    let line = handle_line(&request.to_string());
    serde_json::from_str(line.trim()).expect("worker returns one JSON object")
}

fn profile(
    stop_words: &[&str],
    function_words: &[&str],
    visual_verbs: &[&str],
    min_words: usize,
    max_words: usize,
) -> serde_json::Value {
    serde_json::json!({
        "stop_words": stop_words,
        "function_words": function_words,
        "visual_verbs": visual_verbs,
        "min_words": min_words,
        "max_words": max_words,
        "max_results": 5,
        "reject_verbs_when_all": true,
    })
}

fn request(
    text: &str,
    blocked: &[[usize; 2]],
    limit: usize,
    profile: Option<serde_json::Value>,
    corpus: &[&str],
) -> serde_json::Value {
    let mut request = serde_json::json!({
        "operation": "select_phrases",
        "text": text,
        "blocked_rune_spans": blocked,
        "limit": limit,
        "corpus": corpus,
    });
    if let Some(profile) = profile {
        request["profile"] = profile;
    }
    request
}

fn texts(response: &serde_json::Value) -> Vec<String> {
    response["phrases"]
        .as_array()
        .expect("phrases is an array")
        .iter()
        .map(|phrase| phrase["text"].as_str().expect("phrase text").to_owned())
        .collect()
}

#[test]
fn ranks_configured_actions_and_skips_grounded_names() {
    let response = handle(request(
        "Ada Lovelace built props. Later, they fix costumes.",
        &[[0, 12]],
        5,
        Some(profile(
            &["they"],
            &["they", "later"],
            &["built", "fix"],
            2,
            16,
        )),
        &[],
    ));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(
        texts(&response),
        vec!["built props".to_string(), "fix costumes".to_string()]
    );
}

#[test]
fn the_limit_keeps_the_strongest_phrase_in_order() {
    let response = handle(request(
        "Ada Lovelace built props. Later, they fix costumes.",
        &[[0, 12]],
        1,
        Some(profile(
            &["they"],
            &["they", "later"],
            &["built", "fix"],
            2,
            16,
        )),
        &[],
    ));
    assert_eq!(texts(&response), vec!["built props".to_string()]);
}

#[test]
fn blocked_spans_are_exclusive_and_rune_based() {
    let text = "Ada Lovelace built props.";
    let ungrounded = handle(request(text, &[], 5, None, &[]));
    assert_eq!(
        texts(&ungrounded),
        vec!["Lovelace built props".to_string()],
        "without grounding the longer surface is selectable"
    );

    let name_blocked = handle(request(text, &[[0, 12]], 5, None, &[]));
    assert_eq!(texts(&name_blocked), vec!["built props".to_string()]);

    let action_blocked = handle(request(text, &[[13, 24]], 5, None, &[]));
    assert!(texts(&action_blocked).is_empty());
}

#[test]
fn corpus_recurrence_boosts_an_equal_candidate_and_an_empty_corpus_does_not() {
    let text = "Gamma delta. Alpha beta.";
    let plain = handle(request(text, &[], 5, None, &[]));
    assert_eq!(
        texts(&plain),
        vec!["Gamma delta.".to_string(), "Alpha beta.".to_string()],
        "source order without a corpus"
    );

    let boosted = handle(request(text, &[], 5, None, &[text, "Alpha beta."]));
    assert_eq!(
        texts(&boosted),
        vec!["Alpha beta.".to_string(), "Gamma delta.".to_string()],
        "document recurrence must outrank source order"
    );
}

#[test]
fn keeps_a_natural_six_word_phrase_with_internal_function_words() {
    let phrase = "O maior arrependimento da minha vida";
    let response = handle(request(
        phrase,
        &[],
        5,
        Some(profile(&["o", "da"], &["o", "da"], &[], 2, 6)),
        &[],
    ));
    assert_eq!(texts(&response), vec![phrase.to_string()]);
}

#[test]
fn rejects_function_word_density_above_one_third() {
    let response = handle(request(
        "greatest of my in entire life",
        &[],
        5,
        Some(profile(&["of", "my", "in"], &["of", "my", "in"], &[], 6, 6)),
        &[],
    ));
    assert_eq!(response["ok"], true, "{response}");
    assert!(texts(&response).is_empty());
}

#[test]
fn hard_caps_fragments_at_sixteen_words_even_when_the_policy_asks_for_more() {
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";
    let response = handle(request(
        text,
        &[],
        5,
        Some(profile(&[], &[], &[], 2, 24)),
        &[],
    ));
    let selected = texts(&response);
    assert!(!selected.is_empty());
    for phrase in &selected {
        assert!(
            phrase.split_whitespace().count() <= 16,
            "fragment over the hard ceiling: {phrase:?}"
        );
    }
}

#[test]
fn keeps_a_long_complete_sentence_whole_to_the_terminator() {
    let sentence = "Nella provincia di Pavia, la giovane vittima viene ritrovata senza vita nella sua abitazione il tredici agosto del duemilasette in una vicenda che segna la memoria pubblica italiana.";
    let response = handle(request(
        sentence,
        &[],
        5,
        Some(profile(&[], &[], &[], 2, 6)),
        &[],
    ));
    assert_eq!(
        texts(&response).first().map(String::as_str),
        Some(sentence),
        "the sentence path must ignore the fragment cap"
    );
}

#[test]
fn includes_complete_sentence_and_two_sentence_card_without_overlap() {
    let text = "We built a garden for the neighbors. The garden brought people together. Everyone found a place.";
    let response = handle(request(
        text,
        &[],
        5,
        Some(profile(&[], &[], &[], 2, 6)),
        &[],
    ));
    assert_eq!(
        texts(&response),
        vec![
            "We built a garden for the neighbors. The garden brought people together.".to_string(),
            "Everyone found a place.".to_string(),
        ]
    );
}

/// Byte and rune coordinates are both on the wire: the Go caller projects byte
/// spans, the selector decides in runes. Every reported phrase must slice the
/// original text and its rune offsets must agree with that slice.
#[test]
fn reported_spans_slice_the_source_text_in_bytes_and_runes() {
    let text = "Perché Ada costruì le macchine. Poi le riparò con cura.";
    let response = handle(request(text, &[], 5, None, &[]));
    let phrases = response["phrases"].as_array().expect("phrases");
    assert!(!phrases.is_empty(), "{response}");
    for phrase in phrases {
        let surface = phrase["text"].as_str().expect("text");
        let start_byte = phrase["start_byte"].as_u64().expect("start_byte") as usize;
        let end_byte = phrase["end_byte"].as_u64().expect("end_byte") as usize;
        let start_rune = phrase["start_rune"].as_u64().expect("start_rune") as usize;
        let end_rune = phrase["end_rune"].as_u64().expect("end_rune") as usize;
        assert_eq!(&text[start_byte..end_byte], surface);
        assert_eq!(text[..start_byte].chars().count(), start_rune);
        assert_eq!(text[..end_byte].chars().count(), end_rune);
        assert_eq!(
            surface.chars().count(),
            end_rune - start_rune,
            "rune width must match the surface"
        );
    }
}

#[test]
fn an_empty_text_fails_closed_with_a_typed_error() {
    let response = handle(request("   ", &[], 5, None, &[]));
    assert_eq!(response["ok"], false);
    assert!(response
        .get("phrases")
        .is_none_or(serde_json::Value::is_null));
    assert!(response["error"]
        .as_str()
        .expect("typed error")
        .contains("text is required"));
}

#[test]
fn a_malformed_request_fails_closed_with_a_typed_error() {
    let line = handle_line(r#"{"operation":"select_phrases","text":12}"#);
    let response: serde_json::Value = serde_json::from_str(line.trim()).expect("one JSON object");
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .expect("typed error")
        .contains("invalid select_phrases request"));
}

#[test]
fn the_real_worker_answers_one_select_phrases_request() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_semantic-engine"))
        .env("SEMANTIC_ENGINE_WORKERS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn semantic-engine");
    {
        let request = request("Ada Lovelace built props.", &[[0, 12]], 5, None, &[]);
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
    assert_eq!(responses.len(), 1, "one request, one response: {stdout}");
    let response: serde_json::Value =
        serde_json::from_str(responses[0]).expect("worker returns one JSON object");
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(texts(&response), vec!["built props".to_string()]);
}
