//! Organizations, institutions and concept-absorption guards.

use super::super::*;
use crate::ExtractOptions;
#[test]
fn company_suffixes_are_classified_as_organizations() {
    let source = "Acme Inc. reported revenue.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    let company = entities
        .iter()
        .find(|entity| entity.text == "Acme Inc")
        .unwrap_or_else(|| panic!("organization missing: {entities:?}"));
    assert_eq!(company.r#type, "ORGANIZATION");
    assert_eq!(&source[company.start..company.end], company.text);
}

#[test]
fn person_and_brand_mentions_are_not_absorbed_into_predicate_concepts() {
    let source = "Elon Musk discussed Tesla.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(entities
        .iter()
        .any(|entity| entity.text == "Elon Musk" && entity.r#type == "PERSON"));
    assert!(entities
        .iter()
        .any(|entity| entity.text == "Tesla" && entity.r#type == "BRAND"));
    assert!(!entities
        .iter()
        .any(|entity| { entity.text == "discussed Tesla" && entity.r#type == "VISUAL_CONCEPT" }));
    for entity in &entities {
        assert_eq!(&source[entity.start..entity.end], entity.text);
        assert_eq!(entity.evidence, entity.text);
    }
}

#[test]
fn dotted_initialism_fragments_are_not_emitted_as_entities() {
    let entities = extract(
        "U.S. sales increased.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(!entities
        .iter()
        .any(|entity| entity.text == "U" || entity.text == "S"));
}

#[test]
fn generic_company_nouns_are_not_organizations() {
    let entities = extract(
        "Apple Inc. employed 2,000 people.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Apple Inc" && entity.r#type == "ORGANIZATION"),
        "{entities:?}"
    );
    assert!(
        !entities
            .iter()
            .any(|entity| entity.text.eq_ignore_ascii_case("company")
                || entity.text.eq_ignore_ascii_case("corporation")),
        "{entities:?}"
    );
}

#[test]
fn institutions_are_orgs_and_never_places_or_people() {
    // A geographic cue in front of an office ("in Cassazione", "davanti alla
    // Corte Suprema") names the office, not a point on a map; the curated
    // legal cues decide the kind, and a common noun promoted by its position
    // ("Udienza Preliminare") is not a person either.
    let source = "In Cassazione si discute il ricorso. La Procura ha aperto un \
                  fascicolo. Davanti alla Corte Suprema si discute ancora. Il \
                  Giudice dell'Udienza Preliminare ha deciso, e la città di Pavia \
                  resta il luogo del fatto.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
    )
    .unwrap();
    let find = |name: &str| entities.iter().find(|entity| entity.text == name);
    for office in ["Cassazione", "Procura", "Corte Suprema"] {
        let entity = find(office).unwrap_or_else(|| panic!("{office} missing: {entities:?}"));
        assert_eq!(entity.r#type, "ORG", "{office} must be an organization");
    }
    for surface in [
        "Cassazione",
        "Procura",
        "Corte Suprema",
        "Udienza Preliminare",
    ] {
        assert!(
            !entities
                .iter()
                .any(|entity| entity.text == surface && entity.r#type == "LOCATION"),
            "{surface} must never be a LOCATION: {entities:?}"
        );
    }
    assert_eq!(
        find("Udienza Preliminare").map(|entity| entity.r#type.as_str()),
        Some("VISUAL_CONCEPT"),
        "a procedural act is not a person: {entities:?}"
    );
    assert_eq!(
        find("Pavia").map(|entity| entity.r#type.as_str()),
        Some("LOCATION"),
        "a real place after the same kind of cue stays a place: {entities:?}"
    );
}

#[test]
fn capitalized_multilingual_institution_heads_keep_the_full_name() {
    // The marker head is sometimes tagged as a common noun while its name is
    // tagged as a proper noun. The extractor must keep the full grounded span
    // and type the phrase as an organization in each language.
    for (language, source, expected) in [
        (
            "it",
            "Il Comitato Civico ha pubblicato una relazione.",
            "Comitato Civico",
        ),
        (
            "fr",
            "Institut Aurora publie un rapport.",
            "Institut Aurora",
        ),
        ("pl", "Rada Miejska opublikowała raport.", "Rada Miejska"),
        (
            "es",
            "El Consejo Regional publicó el informe.",
            "Consejo Regional",
        ),
    ] {
        let entities = extract(
            source,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 20,
            },
        )
        .unwrap_or_else(|error| panic!("{language}: {error}"));
        let entity = entities
            .iter()
            .find(|entity| entity.text == expected)
            .unwrap_or_else(|| panic!("{language}: {expected} missing: {entities:?}"));
        assert_eq!(entity.r#type, "ORG", "{language}: {entity:?}");
        assert_eq!(&source[entity.start..entity.end], expected);
    }
}
