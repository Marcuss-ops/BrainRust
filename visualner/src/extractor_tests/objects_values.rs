//! Typed objects, numerals, spelled numbers, currency and date values.

use super::super::*;
use crate::ExtractOptions;
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
fn common_sports_objects_receive_the_typed_object_category() {
    for (language, source, objects) in [
        ("en", "Ball, trophy, jersey.", ["ball", "trophy", "jersey"]),
        (
            "it",
            "Pallone, trofeo, maglia.",
            ["pallone", "trofeo", "maglia"],
        ),
        (
            "es",
            "Balón, trofeo, camiseta.",
            ["balón", "trofeo", "camiseta"],
        ),
        (
            "fr",
            "Ballon, trophée, maillot.",
            ["ballon", "trophée", "maillot"],
        ),
        ("de", "Ball, Pokal, Trikot.", ["ball", "pokal", "trikot"]),
        (
            "pt",
            "Bola, troféu, camiseta.",
            ["bola", "troféu", "camiseta"],
        ),
        (
            "pl",
            "Piłka, trofeum, koszulka.",
            ["piłka", "trofeum", "koszulka"],
        ),
        (
            "ru",
            "Мяч, трофей, футболка.",
            ["мяч", "трофей", "футболка"],
        ),
        ("tr", "Top, kupa, forma.", ["top", "kupa", "forma"]),
        ("id", "Bola, piala, seragam.", ["bola", "piala", "seragam"]),
    ] {
        let entities = extract(
            source,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        for object in objects {
            let candidate = entities
                .iter()
                .find(|entity| entity.text.to_lowercase() == object)
                .unwrap_or_else(|| panic!("object {object:?} missing: {entities:?}"));
            assert_eq!(candidate.r#type, "OBJECT");
            assert_eq!(&source[candidate.start..candidate.end], candidate.text);
        }
    }
}

#[test]
fn everyday_technical_and_historical_objects_are_typed_from_the_shared_lexicon() {
    for (language, source, expected) in [
        ("it", "Il server è nella sala tecnica.", "server"),
        ("es", "El sensor está junto al río.", "sensor"),
        ("fr", "Le capteur mesure le niveau d'eau.", "capteur"),
        (
            "de",
            "Die elektrische Straßenbahn passiert das Museum.",
            "Straßenbahn",
        ),
        (
            "de",
            "Ein historisches Manuskript liegt im Archiv.",
            "Manuskript",
        ),
        ("pt", "O sensor mede a água do rio.", "sensor"),
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
            .find(|entity| entity.text.eq_ignore_ascii_case(expected))
            .unwrap_or_else(|| panic!("{language}: {expected} missing: {entities:?}"));
        assert_eq!(entity.r#type, "OBJECT", "{language}: {entity:?}");
        assert_eq!(&source[entity.start..entity.end], entity.text);
    }
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
fn spellout_composes_spelled_years_without_a_language_table() {
    for (language, source, value) in [
        ("it", "duemilasette", 2007),
        ("it", "duemilaventicinque", 2025),
        ("it", "due mila sette", 2007),
        ("en", "two thousand seven", 2007),
    ] {
        let candidates = crate::spellout::candidates(source, language).unwrap();
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.start == 0 && candidate.value == value as f64),
            "{source:?} must compose to {value}: {candidates:?}"
        );
    }
}

#[test]
fn spelled_italian_date_reaches_temporal_facts() {
    let analysis = crate::analyze(
        "Il tredici agosto duemilasette è una data che segna l'inizio.",
        &crate::ExtractOptions {
            language: "it".to_string(),
            entity_count: 20,
        },
    )
    .expect("analysis");
    assert!(
        analysis
            .temporal_facts
            .iter()
            .any(|fact| fact.normalized.as_deref() == Some("2007-08-13")),
        "spelled date missing: {:?}",
        analysis.temporal_facts
    );
}
