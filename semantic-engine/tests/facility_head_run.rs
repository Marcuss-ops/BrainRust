//! Head-first facility names: a lowercase modifier ends the name run.

mod common;

use common::*;

// Regression: "Thema" (a capitalized common noun) two words before the facility
// head "Stadion" was typed FACILITY because the head-first recognizer walked
// the prefix in the wrong order. The lowercase modifier "kommunales" must end
// the run, so "Thema" is never part of the facility name.
#[test]
fn head_first_facility_does_not_absorb_earlier_capitalized_word() {
    let text = "Das Projekt zum Thema kommunales Stadion wurde geprüft.";
    let value = manifest_value(vec![scene("de-stadion", text)]);
    let entities = value["scenes"][0]["entities"]
        .as_array()
        .expect("entities array");
    assert!(
        !entities
            .iter()
            .any(|e| e["type"] == "FACILITY" && e["text"].as_str().unwrap_or("").contains("Thema")),
        "Thema was absorbed into a facility: {entities:?}"
    );
}

// Regression: a sentence-initial determiner was absorbed when the same
// head-first facility had already been mentioned, producing "The Harbour
// Stadium" and a query with the article. The name keeps its grounded span
// without the determiner, and the fact-grounded query uses that name.
#[test]
fn head_first_facility_drops_a_leading_determiner() {
    let text =
        "They reviewed the renovation of the Harbour Stadium. The Harbour Stadium lies in Bristol.";
    let value = manifest_value(vec![scene("en-harbour", text)]);
    let scene = &value["scenes"][0];
    let facilities: Vec<&str> = scene["entities"]
        .as_array()
        .expect("entities array")
        .iter()
        .filter(|entity| entity["type"] == "FACILITY")
        .filter_map(|entity| entity["text"].as_str())
        .collect();
    assert!(
        facilities.contains(&"Harbour Stadium"),
        "facility name missing: {facilities:?}"
    );
    assert!(
        !facilities.iter().any(|name| name.starts_with("The ")),
        "determiner absorbed into the facility name: {facilities:?}"
    );
    let queries: Vec<&str> = scene["contextual_image_queries"]
        .as_array()
        .expect("queries array")
        .iter()
        .filter_map(|query| query["query"].as_str())
        .collect();
    assert_eq!(queries, vec!["Harbour Stadium Bristol"]);
}
