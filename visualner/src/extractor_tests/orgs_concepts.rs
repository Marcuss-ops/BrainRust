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
                  fascicolo. Davanti alla Corte Suprema si discute ancora. La \
                  Suprema Corte ha annullato la decisione. Il Giudice \
                  dell'Udienza Preliminare ha deciso, e la città di Pavia resta \
                  il luogo del fatto.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
    )
    .unwrap();
    let find = |name: &str| entities.iter().find(|entity| entity.text == name);
    for office in ["Cassazione", "Procura", "Corte Suprema", "Suprema Corte"] {
        let entity = find(office).unwrap_or_else(|| panic!("{office} missing: {entities:?}"));
        assert_eq!(entity.r#type, "ORG", "{office} must be an organization");
    }
    for surface in [
        "Cassazione",
        "Procura",
        "Corte Suprema",
        "Suprema Corte",
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
fn geo_name_alias_inside_a_curated_organization_is_not_a_place() {
    let entities = extract(
        "Nel 1994 fu eletto nelle liste del Partito Democratico della Sinistra.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
    )
    .unwrap();
    assert!(entities.iter().any(|entity| {
        entity.text == "Partito Democratico della Sinistra" && entity.r#type == "ORG"
    }));
    assert!(
        !entities
            .iter()
            .any(|entity| entity.text == "Sinistra" && entity.r#type == "LOCATION"),
        "{entities:?}"
    );
}

#[test]
fn italian_organization_complements_use_verb_dependencies_not_name_allowlists() {
    let source = "Corrado Augias entrò in Rai e collaborò con la Repubblica.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
    )
    .expect("italian extraction");
    for name in ["Rai", "Repubblica"] {
        assert!(
            entities
                .iter()
                .any(|entity| entity.text == name && entity.r#type == "ORG"),
            "organization complement {name} missing or mistyped: {entities:?}"
        );
    }
}

#[test]
fn acronym_compounds_are_not_people() {
    // "DNA di Chiara" is a trace mentioned in the text, not a person: the
    // acronym opens the surface and carries no name evidence.
    let entities = extract(
        "Il DNA di Chiara è stato analizzato durante le indagini.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(
        !entities
            .iter()
            .any(|entity| entity.r#type == "PERSON" && entity.text.contains("DNA")),
        "an acronym compound must not be a person: {entities:?}"
    );
}

#[test]
fn stress_entities_keep_curated_acronym_and_team_types() {
    let source = "La NASA e l’ESA collaborano con il CERN. L’ANSA racconta la missione dei Chicago Bulls. Nike ha creato Air Jordan.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "it".into(),
            entity_count: 20,
        },
    )
    .unwrap();
    for name in ["NASA", "ESA", "CERN", "ANSA", "Chicago Bulls"] {
        let entity = entities
            .iter()
            .find(|entity| entity.text == name)
            .unwrap_or_else(|| panic!("{name} missing: {entities:?}"));
        // MEDIA_OUTLET is the specific organization type for a curated
        // outlet (ANSA); every consumer treats it as an organization.
        assert!(
            matches!(
                entity.r#type.as_str(),
                "ORG" | "ORGANIZATION" | "MEDIA_OUTLET"
            ),
            "{name}: {entity:?}"
        );
    }
    assert_eq!(
        entities
            .iter()
            .find(|entity| entity.text == "Air Jordan")
            .map(|entity| entity.r#type.as_str()),
        Some("BRAND"),
        "{entities:?}"
    );
    assert_eq!(
        entities
            .iter()
            .filter(|entity| entity.text == "ANSA")
            .count(),
        1,
        "an elided article must not create a second organization identity: {entities:?}"
    );
}

#[test]
fn a_curated_outlet_has_one_span_and_one_specific_type() {
    // The gazetteer scan typed it ORG (no proper-noun evidence) while the
    // single-token scan typed it MEDIA_OUTLET; both survived and the manifest
    // order depended on hash iteration. One span, one specific type.
    for _ in 0..4 {
        let entities = extract(
            "ANSA",
            &ExtractOptions {
                language: "it".into(),
                entity_count: 20,
            },
        )
        .unwrap();
        let outlets: Vec<_> = entities
            .iter()
            .filter(|entity| entity.text == "ANSA")
            .collect();
        assert_eq!(outlets.len(), 1, "{entities:?}");
        assert_eq!(outlets[0].r#type, "MEDIA_OUTLET", "{entities:?}");
    }
}

#[test]
fn a_dropped_article_use_a_distinctive_outlet_name() {
    let entities = extract(
        "Fatto Quotidiano ha pubblicato l'inchiesta. Repubblica ha rilanciato la notizia.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Fatto Quotidiano" && entity.r#type == "MEDIA_OUTLET"),
        "the article-less distinctive name is a source: {entities:?}"
    );
    assert!(
        !entities
            .iter()
            .any(|entity| entity.text == "Repubblica" && entity.r#type == "MEDIA_OUTLET"),
        "a bare ambiguous head must not be guessed: {entities:?}"
    );
}

#[test]
fn curated_outlet_domains_are_sources_even_in_lowercase() {
    let source = "Secondo ansa.it, il debito è salito. Il sito notizie.it riporta la notizia. \
         Un lettore ha scritto a redazione@ansa.it e il dominio example.com non è una fonte.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
    )
    .unwrap();
    for domain in ["ansa.it", "notizie.it"] {
        let found: Vec<_> = entities
            .iter()
            .filter(|entity| entity.text.eq_ignore_ascii_case(domain))
            .collect();
        assert_eq!(found.len(), 1, "{domain} must be one mention: {entities:?}");
        assert_eq!(found[0].r#type, "MEDIA_OUTLET", "{domain}: {entities:?}");
        assert_eq!(&source[found[0].start..found[0].end], found[0].text);
    }
    assert!(
        !entities
            .iter()
            .any(|entity| entity.text.eq_ignore_ascii_case("example.com")),
        "an unlisted domain is not a source: {entities:?}"
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

#[test]
fn a_name_particle_fragment_is_not_a_visual_concept() {
    // The lowercase particle breaks the title-case run, so "Sanne" and    // "Vries" were emitted as standalone concepts next to the person. A
    // concept fully inside a longer person name is a fragment, not B-roll.
    let source = "In 2019 gaf de gemeente Utrecht de ingenieur Sanne de Vries de opdracht om de werkzaamheden te controleren.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "nl".to_string(),
            entity_count: 64,
        },
    )
    .expect("nl extraction");
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Sanne de Vries" && entity.r#type == "PERSON"),
        "the full person name is missing: {entities:?}"
    );
    for fragment in ["Sanne", "de", "Vries"] {
        assert!(
            !entities
                .iter()
                .any(|entity| entity.text == fragment && entity.r#type == "VISUAL_CONCEPT"),
            "name fragment {fragment:?} became a visual concept: {entities:?}"
        );
    }
}
