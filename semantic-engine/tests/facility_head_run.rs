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
