//! Unit tests for extractor (extracted from extractor.rs to keep the
//! production file reviewable; compiled only under cfg(test)).

use super::*;

fn top3(text: &str) -> Vec<VisualEntity> {
    extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 3,
        },
    )
    .unwrap()
}

fn texts(entities: &[VisualEntity]) -> Vec<String> {
    entities.iter().map(|e| e.text.clone()).collect()
}

// TestEntitiesRequireSourceEvidence — the killer rule.
// "Imagine the" / "ready" must NEVER become entities because they are
// stopword-seeded and never produce candidates in the first place.
#[test]
fn imagine_the_and_ready_are_rejected() {
    let entities = top3("Imagine the vibrant world of Greek cuisine, ready to discover...");
    let surfaces: Vec<&str> = entities.iter().map(|e| e.text.as_str()).collect();
    assert!(
        !surfaces
            .iter()
            .any(|s| s.to_lowercase().contains("imagine")),
        "Imagine the must not become an entity: {:?}",
        surfaces
    );
    assert!(
        !surfaces.iter().any(|s| s.to_lowercase() == "ready"),
        "ready must not become an entity: {:?}",
        surfaces
    );
    assert!(
        !surfaces
            .iter()
            .any(|s| s.to_lowercase().contains("discover")),
        "discover must not become an entity: {:?}",
        surfaces
    );
}

// TestExactEntityLimit — requesting 3 returns exactly 3 when the text
// has ≥3 candidates.
#[test]
fn exactly_three_entities_returned() {
    let entities = top3(
        "Greek salad combines fresh tomatoes, cucumbers, olives, feta cheese and olive oil.",
    );
    assert_eq!(
        entities.len(),
        3,
        "expected exactly 3 entities, got {entities:?}"
    );
}

// Greek salad regression — the canonical golden-fixture input.
// Expected: feta cheese, tomatoes, olives (top 3 by visual score).
#[test]
fn greek_salad_returns_feta_tomatoes_olives() {
    let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
    let surfaces = texts(&entities);
    assert!(
        surfaces.iter().any(|s| s.to_lowercase() == "feta cheese"),
        "feta cheese missing: {surfaces:?}"
    );
    assert!(
        surfaces.iter().any(|s| s.to_lowercase() == "tomatoes"),
        "tomatoes missing: {surfaces:?}"
    );
    assert!(
        surfaces.iter().any(|s| s.to_lowercase() == "olives"),
        "olives missing: {surfaces:?}"
    );
    // All 3 must be source-grounded.
    for e in &entities {
        assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
        assert_eq!(e.evidence, e.text, "evidence must equal text verbatim");
    }
}

// Hummus regression — the second golden-fixture segment.
// Expected candidates include hummus, chickpeas, tahini, lemon juice, olive oil;
// top 3 must all be source-grounded.
#[test]
fn hummus_returns_source_grounded_entities() {
    let entities =
        top3("Hummus is traditionally made with chickpeas, tahini, lemon juice and olive oil.");
    // All returned entities must be source-grounded (NO EVIDENCE → NO ENTITY).
    for e in &entities {
        assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
        assert_eq!(
            e.evidence, e.text,
            "evidence must equal text verbatim for {}",
            e.text
        );
    }
    // The top 3 must come from the expected candidate set.
    let expected = ["hummus", "chickpeas", "tahini", "lemon juice", "olive oil"];
    for e in &entities {
        let lower = e.text.to_lowercase();
        assert!(
            expected.iter().any(|s| *s == lower),
            "unexpected entity {}: not in {expected:?}",
            e.text
        );
    }
}

#[test]
fn trump_is_retained_as_a_person_in_a_dense_scene() {
    let text = "Donald Trump's trajectory offers a profound case study into the interwoven nature of business success, intense media visibility, and political leadership within American public life. His career has consistently demonstrated how these three elements feed one another, creating a defining narrative arc for Donald Trump. Understanding Donald Trump's influence requires recognizing his public impact.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    let trump = entities.iter().find(|entity| {
        entity.r#type == "PERSON" && entity.text.to_lowercase().contains("trump")
    });
    assert!(
        trump.is_some(),
        "Donald Trump must survive the top-N visual entity bound: {entities:?}"
    );
    let trump = trump.unwrap();
    assert_eq!(trump.text, "Donald Trump");
    assert_eq!(trump.evidence, trump.text);
}

#[test]
fn person_runs_are_not_prefixed_or_suffixed_by_sentence_text() {
    let text = "Michael Jordan transformed basketball. Phil Jackson designed the offense. Scottie Pippen supplied versatile defense.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    let names: Vec<&str> = entities
        .iter()
        .filter(|entity| entity.r#type == "PERSON")
        .map(|entity| entity.text.as_str())
        .collect();
    assert!(
        names.contains(&"Michael Jordan"),
        "Michael Jordan missing: {entities:?}"
    );
    assert!(
        names.contains(&"Phil Jackson"),
        "Phil Jackson missing: {entities:?}"
    );
    assert!(
        names.contains(&"Scottie Pippen"),
        "Scottie Pippen missing: {entities:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.contains("designed") || name.contains("supplied")),
        "predicate leaked into person: {names:?}"
    );
}

#[test]
fn stopword_leads_and_known_locations_do_not_consume_person_slots() {
    let text = "Michael Jordan studied at the University of North Carolina with Dean Smith. When Jordan entered the game, Scottie Pippen and Phil Jackson supported him.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    let persons: Vec<&str> = entities
        .iter()
        .filter(|entity| entity.r#type == "PERSON")
        .map(|entity| entity.text.as_str())
        .collect();
    assert_eq!(
        persons,
        vec![
            "Michael Jordan",
            "Dean Smith",
            "Scottie Pippen",
            "Phil Jackson"
        ]
    );
    assert!(
        !entities.iter().any(|entity| entity.text == "When Jordan"),
        "stopword-prefixed false person must be rejected: {entities:?}"
    );
    assert!(
        !persons.contains(&"North Carolina"),
        "known location must not be typed as PERSON: {entities:?}"
    );
}

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
    assert!(!entities.iter().any(|entity| {
        entity.text == "discussed Tesla" && entity.r#type == "VISUAL_CONCEPT"
    }));
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
fn dotted_person_initials_keep_the_full_grounded_name() {
    let source = "J. D. Vance visited Berlin.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    let vance = entities.iter().find(|entity| entity.text == "J. D. Vance");
    assert_eq!(
        vance.map(|entity| entity.r#type.as_str()),
        Some("PERSON"),
        "{entities:?}"
    );
    let vance = vance.unwrap();
    assert_eq!(
        source.get(vance.start..vance.end),
        Some(vance.text.as_str())
    );
    assert!(!entities
        .iter()
        .any(|entity| entity.text == "J" || entity.text == "D" || entity.text == "Vance"));
}

#[test]
fn multilingual_scene_function_words_and_named_places_are_typed_safely() {
    for (language, source, expected_place) in [
        (
            "en",
            "The president Emmanuel Macron visited Berlin.",
            "Berlin",
        ),
        ("en", "J. D. Vance visited Berlin.", "Berlin"),
        (
            "it",
            "Il presidente Emmanuel Macron ha visitato Parigi.",
            "Parigi",
        ),
        ("es", "El presidente Emmanuel Macron visitó París.", "París"),
        (
            "pt",
            "O presidente Emmanuel Macron visitou Lisboa.",
            "Lisboa",
        ),
        (
            "fr",
            "Le président Emmanuel Macron a visité Paris.",
            "Paris",
        ),
        (
            "de",
            "Der Präsident Emmanuel Macron besuchte Berlin.",
            "Berlin",
        ),
    ] {
        let entities = extract(
            source,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        assert!(
            entities
                .iter()
                .any(|entity| (entity.text == "Emmanuel Macron"
                    || entity.text == "J. D. Vance")
                    && entity.r#type == "PERSON"),
            "{language}: {entities:?}"
        );
        assert!(
            entities
                .iter()
                .any(|entity| entity.text == expected_place && entity.r#type == "LOCATION"),
            "{language}: {entities:?}"
        );
        assert!(
            !entities.iter().any(|entity| matches!(
                entity.text.to_lowercase().as_str(),
                "il" | "el"
                    | "o"
                    | "le"
                    | "der"
                    | "presidente"
                    | "président"
                    | "visited"
                    | "visitó"
                    | "visitato"
                    | "visitou"
                    | "visité"
                    | "besuchte"
            )),
            "function word leaked for {language}: {entities:?}"
        );
        for entity in &entities {
            assert_eq!(
                source.get(entity.start..entity.end),
                Some(entity.text.as_str())
            );
            assert_eq!(entity.evidence, entity.text);
        }
    }
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
fn entity_candidates_remain_verbatim_spans_and_keep_numeric_values() {
    let source = "Tesla reported $2.5 billion in revenue and employed 5,000 workers.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(entities.iter().any(|entity| entity.text == "Tesla"));
    assert!(entities
        .iter()
        .any(|entity| entity.r#type == "MONEY" && entity.text == "$2.5 billion"));
    assert!(entities
        .iter()
        .any(|entity| entity.r#type == "NUMBER" && entity.text == "5,000"));
    for entity in &entities {
        assert_eq!(&source[entity.start..entity.end], entity.text);
        assert_eq!(entity.evidence, entity.text);
    }
}

#[test]
fn typed_entities_use_shared_vocabulary() {
    let entities = extract(
        "Gerard Butler spoke at an event in London. OpenAI released an iPhone.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    let find = |name: &str| entities.iter().find(|entity| entity.text == name);
    assert_eq!(
        find("Gerard Butler").map(|entity| entity.r#type.as_str()),
        Some("PERSON")
    );
    assert_eq!(
        find("London").map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
    assert_eq!(
        find("OpenAI").map(|entity| entity.r#type.as_str()),
        Some("BRAND")
    );
    assert_eq!(
        find("iPhone").map(|entity| entity.r#type.as_str()),
        Some("PRODUCT")
    );
}

#[test]
fn extracts_typed_brands_metrics_money_and_dates_with_exact_spans() {
    let source = "OpenAI reported 25% growth, $2.5 million in revenue, 42 orders on March 5, 2026, November 22, 1986 and 03/06/2026.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 30,
        },
    )
    .unwrap();
    let find = |kind: &str, text: &str| {
        entities
            .iter()
            .find(|entity| entity.r#type == kind && entity.text == text)
    };
    assert!(
        find("BRAND", "OpenAI").is_some(),
        "brand missing: {entities:?}"
    );
    assert!(
        find("PERCENT", "25%").is_some(),
        "percentage missing: {entities:?}"
    );
    assert!(
        find("MONEY", "$2.5 million in revenue").is_none(),
        "money span should stop at the currency unit: {entities:?}"
    );
    assert!(
        find("MONEY", "$2.5 million").is_some(),
        "money missing: {entities:?}"
    );
    assert!(
        find("NUMBER", "42 orders").is_some(),
        "metric missing: {entities:?}"
    );
    assert!(
        find("DATE", "March 5, 2026").is_some(),
        "month date missing: {entities:?}"
    );
    assert!(
        find("DATE", "03/06/2026").is_some(),
        "numeric date missing: {entities:?}"
    );
    assert!(
        find("DATE", "November 22, 1986").is_some(),
        "month date with multiple spaces must retain the exact year: {entities:?}"
    );
    for entity in entities.iter().filter(|entity| {
        matches!(
            entity.r#type.as_str(),
            "BRAND" | "PERCENT" | "MONEY" | "NUMBER" | "DATE"
        )
    }) {
        assert_eq!(&source[entity.start..entity.end], entity.text);
        assert_eq!(entity.evidence, entity.text);
    }
}

#[test]
fn spelled_numbers_use_locale_rules_units_and_exact_utf8_evidence() {
    for (language, source, surface, kind) in [
        (
            "en",
            "Growth reached twenty-five percent.",
            "twenty-five percent",
            "PERCENT",
        ),
        (
            "it",
            "Vendite pari a venticinque percento.",
            "venticinque percento",
            "PERCENT",
        ),
        (
            "fr",
            "La croissance atteint vingt-cinq pour cent.",
            "vingt-cinq pour cent",
            "PERCENT",
        ),
        (
            "ru",
            "Показатель вырос на двадцать пять процентов.",
            "двадцать пять процентов",
            "PERCENT",
        ),
        (
            "de",
            "Die Firma meldete dreißig Bestellungen.",
            "dreißig Bestellungen",
            "NUMBER",
        ),
    ] {
        let entities = extract(
            source,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 30,
            },
        )
        .unwrap();
        let entity = entities
            .iter()
            .find(|entity| entity.r#type == kind && entity.text == surface);
        assert!(
            entity.is_some(),
            "{language} did not extract {surface:?}: {entities:?}"
        );
        let entity = entity.unwrap();
        assert_eq!(source.get(entity.start..entity.end), Some(surface));
        assert_eq!(entity.evidence, surface);
    }
}

#[test]
fn spelled_number_parser_rejects_ambiguous_words_and_partial_matches() {
    let source =
        "One day later, a runner reached the finish, while twenty-five percent of sales rose.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 30,
        },
    )
    .unwrap();
    assert!(
        !entities.iter().any(
            |entity| matches!(entity.r#type.as_str(), "NUMBER" | "ORDINAL")
                && matches!(entity.text.as_str(), "One" | "one" | "a" | "One day")
        ),
        "ambiguous small words and duration phrases must not be typed as numbers: {entities:?}"
    );
    assert!(
        !entities.iter().any(|entity| entity.text == "twenty-five"),
        "partial parse before a unit must not leak: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .any(|entity| entity.r#type == "PERCENT" && entity.text == "twenty-five percent"),
        "quantity surface missing: {entities:?}"
    );
}

#[test]
fn spelled_number_spans_do_not_absorb_leading_prose() {
    for (language, source, number) in [
        (
            "it",
            "Vendite pari a venticinque percento oggi.",
            "venticinque percento",
        ),
        (
            "en",
            "Revenue reached twenty-five percent today.",
            "twenty-five percent",
        ),
    ] {
        let candidates = crate::spellout::candidates(source, language).unwrap();
        assert_eq!(
            candidates.len(),
            1,
            "unexpected spellout candidates for {language}: {candidates:?}"
        );
        let candidate = &candidates[0];
        assert_eq!(source.get(candidate.start..candidate.end), Some(number));
        assert_eq!(candidate.entity_type, "PERCENT");
    }
}

#[test]
fn language_is_required_and_unsupported_rules_fail_closed() {
    assert!(extract(
        "twenty-five orders",
        &ExtractOptions {
            language: String::new(),
            entity_count: 3
        }
    )
    .is_err());
    let unsupported = extract(
        "Some text.",
        &ExtractOptions {
            language: "zz-ZZ".to_string(),
            entity_count: 3,
        },
    );
    assert!(
        unsupported.is_err(),
        "invalid language tags should fail explicitly"
    );
}

#[test]
fn euro_prefix_keeps_utf8_boundaries_and_june_year_is_a_date() {
    let source = "Revenue was €80,000 in June 1988.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.r#type == "MONEY" && entity.text == "€80,000"),
        "money span missing: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .any(|entity| entity.r#type == "DATE" && entity.text == "June 1988"),
        "month-year date missing: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .all(|entity| source.get(entity.start..entity.end) == Some(entity.text.as_str())),
        "invalid UTF-8 evidence span: {entities:?}"
    );
}

#[test]
fn las_vegas_is_a_location_not_a_person() {
    let entities = top3("Mike Tyson fought in Las Vegas.");
    let las_vegas = entities.iter().find(|entity| entity.text == "Las Vegas");
    assert_eq!(
        las_vegas.map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
}

#[test]
fn named_location_survives_a_dense_top_five() {
    let text = "On November 22, 1986, in Las Vegas, he faced Trevor Berbick for the WBC heavyweight title. Tyson won by second-round technical knockout and became the youngest heavyweight world champion.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Las Vegas" && entity.r#type == "LOCATION"),
        "a named location must not be crowded out by visual-concept phrases: {entities:?}"
    );
}

#[test]
fn atlantic_city_is_a_location_not_a_person() {
    let entities = top3("In June 1988, Tyson met Michael Spinks in Atlantic City.");
    let atlantic_city = entities
        .iter()
        .find(|entity| entity.text == "Atlantic City");
    assert_eq!(
        atlantic_city.map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
}

#[test]
fn portuguese_geographic_cues_extract_city_and_neighborhood() {
    let text = "Isabelle Caracristi morreu em Fortaleza, no bairro Aldeota.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "pt".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    for place in ["Fortaleza", "Aldeota"] {
        let found = entities
            .iter()
            .find(|entity| entity.text == place)
            .unwrap_or_else(|| panic!("missing {place}: {entities:?}"));
        assert_eq!(found.r#type, "LOCATION", "{found:?}");
        assert_eq!(text.get(found.start..found.end), Some(place));
    }
}

#[test]
fn month_after_location_preposition_is_not_a_place() {
    let entities = extract(
        "The meeting was in May.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    assert!(!entities
        .iter()
        .any(|entity| entity.text == "May" && entity.r#type == "LOCATION"));
}

#[test]
fn dolly_places_and_song_titles_do_not_consume_person_slots() {
    let text = "Dolly Parton was born in Sevier County, Tennessee, in the Great Smoky Mountains. Porter Wagoner worked with Dolly Parton. I Will Always Love You became a song.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 10,
        },
    )
    .unwrap();
    let find = |name: &str| entities.iter().find(|entity| entity.text == name);
    assert_eq!(
        find("Dolly Parton").map(|entity| entity.r#type.as_str()),
        Some("PERSON")
    );
    assert_eq!(
        find("Porter Wagoner").map(|entity| entity.r#type.as_str()),
        Some("PERSON")
    );
    assert_eq!(
        find("Sevier County").map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
    assert_eq!(
        find("Great Smoky Mountains").map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
    assert_eq!(
        find("Will Always Love You").map(|entity| entity.r#type.as_str()),
        Some("WORK")
    );
    assert!(!entities.iter().any(|entity| {
        entity.r#type == "PERSON"
            && matches!(
                entity.text.as_str(),
                "Sevier County"
                    | "Great Smoky Mountains"
                    | "Will Always Love"
                    | "Will Always Love You"
            )
    }));
}

// TestEntitiesRequireSourceEvidence — explicit: an entity not in the
// source text must never be returned. Synthetic check.
#[test]
fn no_entity_without_evidence() {
    let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
    for e in &entities {
        // evidence must be a non-empty verbatim slice of the source.
        assert!(e.start < e.end, "empty span for {}", e.text);
        assert!(!e.evidence.is_empty(), "empty evidence for {}", e.text);
        assert_eq!(
            e.evidence, e.text,
            "evidence must equal text verbatim for {}",
            e.text
        );
    }
}

// Determinism: 100 runs must produce the same winner set.
#[test]
fn deterministic_across_runs() {
    let text = "Greek salad contains tomatoes, feta cheese and olives.";
    let first = top3(text);
    for _ in 0..99 {
        let again = top3(text);
        assert_eq!(again, first, "non-deterministic extraction result");
    }
}
