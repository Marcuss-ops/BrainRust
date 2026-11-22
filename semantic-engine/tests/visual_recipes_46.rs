//! TICKET-46: credit quotes and highlight cited outlets from data the manifest
//! already extracted. Both recipes must fail closed: an unattributed quote and
//! an outlet without an attribution cue produce nothing.

mod common;

use common::{manifest_value_for, scene};
use serde_json::Value;

fn candidates(language: &str, text: &str) -> Vec<Value> {
    let value = manifest_value_for(language, vec![scene("recipe", text)]);
    value["visual_consolidation"]["production_candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn recipe<'a>(candidates: &'a [Value], name: &str) -> Vec<&'a Value> {
    candidates
        .iter()
        .filter(|candidate| candidate["recipe"] == name)
        .collect()
}

#[test]
fn a_delimited_quote_is_credited_to_the_same_sentence_speaker() {
    let source = "«Non molliamo», ha detto Mario Rossi.";
    let candidates = candidates("it", source);
    let quotes = recipe(&candidates, "quote_attribution");
    assert_eq!(quotes.len(), 1, "{candidates:?}");
    let payload = &quotes[0]["visual_payload"];
    assert_eq!(payload["quote_text"], "«Non molliamo»");
    assert_eq!(payload["author_label"], "Mario Rossi");
    assert_eq!(payload["author_type"], "PERSON");
    let start = quotes[0]["start_byte"].as_u64().unwrap() as usize;
    let end = quotes[0]["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], "«Non molliamo»");
}

#[test]
fn an_attributed_quote_carries_the_speaker_role_when_the_manifest_has_it() {
    let source = "«Il piano è pronto», ha detto il sindaco Mario Rossi.";
    let candidates = candidates("it", source);
    let quotes = recipe(&candidates, "quote_attribution");
    assert_eq!(quotes.len(), 1, "{candidates:?}");
    assert_eq!(
        quotes[0]["visual_payload"]["role_surface"], "sindaco",
        "{quotes:?}"
    );
}

#[test]
fn a_quote_without_a_named_speaker_stays_a_plain_quote() {
    for source in ["«Non molliamo», hanno detto ieri.", "«Non molliamo»."] {
        let candidates = candidates("it", source);
        assert!(
            recipe(&candidates, "quote_attribution").is_empty(),
            "{source}: {candidates:?}"
        );
    }
}

#[test]
fn a_cited_outlet_highlights_the_attributed_sentence() {
    let source = "Secondo ANSA, il debito è salito del 12%.";
    let candidates = candidates("it", source);
    let highlights = recipe(&candidates, "source_highlight");
    assert_eq!(highlights.len(), 1, "{candidates:?}");
    let payload = &highlights[0]["visual_payload"];
    assert_eq!(payload["source_label"], "ANSA");
    assert_eq!(payload["highlight_kind"], "phrase");
    let highlight = payload["highlight_text"].as_str().unwrap();
    assert!(highlight.contains("debito"), "{payload:?}");
    let start = highlights[0]["start_byte"].as_u64().unwrap() as usize;
    let end = highlights[0]["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], highlight);
}

#[test]
fn an_outlet_without_an_attribution_cue_emits_no_highlight() {
    let source = "ANSA ha pubblicato il rapporto.";
    let candidates = candidates("it", source);
    assert!(
        recipe(&candidates, "source_highlight").is_empty(),
        "{candidates:?}"
    );
}

#[test]
fn an_ordinary_organisation_is_not_a_citable_source() {
    let source = "Secondo la società, il debito è salito del 12%.";
    let candidates = candidates("it", source);
    assert!(
        recipe(&candidates, "source_highlight").is_empty(),
        "{candidates:?}"
    );
}
