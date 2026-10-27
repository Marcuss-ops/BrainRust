//! Hyphenated compounds: POS tagging and typing of modifier+head surfaces.
//!
//! A hyphenated compound is one ICU token but several UDPipe words
//! ("water-level" -> "water" + "-" + "level"). These tests pin the two
//! general rules that keep such surfaces extractable: the joined-word POS
//! alignment (`pos::match_joined_compound`) and the hyphen-segment
//! capitalization evidence in `values::classify_type`. Neither rule mentions
//! any fixture name.

use super::super::*;
use crate::ExtractOptions;

#[test]
fn hyphenated_noun_phrase_keeps_its_tag_and_survives_extraction() {
    // Before the joined-word alignment the compound token stayed untagged and
    // the phrase failed the visual-concept gate: no entity at all.
    let entities = extract(
        "The technician inspected the water-level sensor and compared it.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    let entity = entities
        .iter()
        .find(|entity| entity.text == "water-level sensor")
        .unwrap_or_else(|| panic!("missing compound: {entities:?}"));
    assert!(
        matches!(entity.r#type.as_str(), "VISUAL_CONCEPT" | "OBJECT"),
        "{entity:?}"
    );
    // The span is verbatim source (NO EVIDENCE → NO ENTITY).
    assert_eq!(
        &"The technician inspected the water-level sensor and compared it."
            [entity.start..entity.end],
        "water-level sensor"
    );
}

#[test]
fn hyphenated_modifier_head_compound_survives_without_a_model() {
    // Model-free path: an untagged multi-word phrase must stay extractable.
    let entities = extract(
        "The high-level bridge is ready.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "high-level bridge"),
        "{entities:?}"
    );
}

#[test]
fn hyphenated_german_institution_compounds_are_organizations() {
    // German compounding joins the name modifier to the institutional head
    // ("Prisma-Labor" = "Prisma" + "Labor"). The head is the same shared
    // multilingual organization vocabulary the head-first pattern uses; only
    // the joining morphology differs.
    for (source, expected) in [
        (
            "Das Prisma-Labor hat einen Bericht veröffentlicht.",
            "Prisma-Labor",
        ),
        ("Der Stella-Rat hat entschieden.", "Stella-Rat"),
        ("Das Aurora-Institut hat teilgenommen.", "Aurora-Institut"),
        (
            "Die Lumen-Genossenschaft hat berichtet.",
            "Lumen-Genossenschaft",
        ),
        ("Die Horizont-Stiftung hat gefördert.", "Horizont-Stiftung"),
    ] {
        let entities = extract(
            source,
            &ExtractOptions {
                language: "de".to_string(),
                entity_count: 20,
            },
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        let entity = entities
            .iter()
            .find(|entity| entity.text == expected)
            .unwrap_or_else(|| panic!("{expected} missing in {source}: {entities:?}"));
        assert_eq!(entity.r#type, "ORG", "{source}: {entity:?}");
        assert_eq!(&source[entity.start..entity.end], expected);
    }
}
