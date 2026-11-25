use super::*;

#[test]
fn italian_institutions_and_curated_given_names_beat_place_guessing() {
    assert_eq!(
        classify_value_type("Parlamento europeo", true, true, true, "it"),
        "ORG"
    );
    assert_eq!(
        classify_value_type("Regia Aeronautica", true, true, true, "it"),
        "ORG"
    );
    assert_eq!(
        classify_value_type("Teatro del 101", true, true, true, "it"),
        "ORG"
    );
    assert_eq!(
        classify_value_type(
            "Orchestra Sinfonica Nazionale della Rai",
            true,
            true,
            true,
            "it"
        ),
        "ORG"
    );
    assert_eq!(
        classify_value_type("Repubblica", true, true, true, "it"),
        "ORG"
    );
}

#[test]
fn media_outlets_are_typed_before_the_organisation_marker() {
    for (surface, language) in [
        ("Il Fatto Quotidiano", "it"),
        ("la Repubblica", "it"),
        ("ANSA", "it"),
        ("The Guardian", "en"),
        ("Le Monde", "fr"),
    ] {
        assert_eq!(
            classify_value_type(surface, true, true, true, language),
            "MEDIA_OUTLET",
            "{surface} ({language})"
        );
    }
    // Without proper-noun evidence the surface is prose, not a source.
    assert_ne!(
        classify_value_type("la repubblica", false, false, false, "it"),
        "MEDIA_OUTLET"
    );
}

#[test]
fn rejects_fused_subtitle_number_artifacts_without_blocking_common_codes() {
    assert!(has_suspicious_asr_digit_fusion("140ome"));
    assert!(has_suspicious_asr_digit_fusion(
        "The road is 100red miles long"
    ));
    for code in ["3D", "4K", "5G", "F16", "2025th"] {
        assert!(!has_suspicious_asr_digit_fusion(code), "{code}");
    }
}

#[test]
fn month_before_day_handles_multibyte_punctuation_before_date() {
    let source = "L’indagine proseguì fino a giugno 1988.";
    let number_start = source.find("1988").unwrap();
    let month_start = month_before_day(source, number_start).unwrap();
    assert_eq!(source.get(month_start..number_start), Some("giugno "));

    let candidates = value_candidates(source);
    assert!(candidates.iter().any(|candidate| {
        candidate.text == "giugno 1988"
            && source.get(candidate.start..candidate.end) == Some("giugno 1988")
    }));
}

#[test]
fn physical_measurements_are_typed_as_measurement_not_number() {
    let cases = [
        ("Il ponte misura 12 km di lunghezza.", "12"),
        ("La nave pesa 3,5 tonnellate.", "3,5"),
        ("Il forno raggiunge 180 °C.", "180"),
        ("The tank holds 40 liters.", "40"),
        ("The parcel weighs 2 kg.", "2"),
        ("El puente mide 12 km de largo.", "12"),
        ("Die Brücke ist 12 km lang.", "12"),
        ("Le pont mesure 12 km de long.", "12"),
        ("Ponte ma 12 km długości.", "12"),
        ("Мост имеет длину 12 км.", "12"),
    ];
    for (source, surface) in cases {
        let candidates = value_candidates(source);
        assert!(
            candidates.iter().any(|candidate| {
                candidate.entity_type.as_deref() == Some("MEASUREMENT")
                    && source.get(candidate.start..candidate.end) == Some(surface)
            }),
            "{source}: {:?}",
            candidates
                .iter()
                .map(|candidate| (&candidate.text, &candidate.entity_type))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn measurement_units_do_not_steal_money_percent_or_dates() {
    let money = value_candidates("Costs rose to $1,200.");
    assert!(money
        .iter()
        .any(|c| c.entity_type.as_deref() == Some("MONEY")));
    let percent = value_candidates("Sales grew 25% this year.");
    assert!(percent
        .iter()
        .any(|c| c.entity_type.as_deref() == Some("PERCENT")));
    let date = value_candidates("Il 12 giugno 1988 la nave partì.");
    assert!(date
        .iter()
        .any(|c| c.entity_type.as_deref() == Some("DATE")));
    assert!(!date
        .iter()
        .any(|c| c.entity_type.as_deref() == Some("MEASUREMENT")));
}

#[test]
fn a_bare_lowercase_institution_word_is_a_common_noun_not_an_organization() {
    // The cue head names the office only when the surface keeps its
    // capital: "tribunale" is the common noun, "Tribunale di Torino" and
    // "La Procura" are the offices.
    assert_eq!(
        classify_type("tribunale", true, false, true, "it"),
        "VISUAL_CONCEPT"
    );
    assert_eq!(classify_type("Tribunale", true, false, true, "it"), "ORG");
    assert_eq!(
        classify_type("Tribunale di Torino", true, false, true, "it"),
        "ORG"
    );
    assert_eq!(classify_type("Procura", true, false, true, "it"), "ORG");
}

#[test]
fn inverted_institutions_are_orgs_and_acronym_compounds_are_not_people() {
    // Italian inverts the office: "Suprema Corte" is the same institution
    // as "Corte Suprema" and must not fall through to the person rule.
    assert_eq!(
        classify_type("Suprema Corte", true, false, true, "it"),
        "ORG"
    );
    // An acronym-led compound is evidence, not a name: "DNA di Chiara"
    // must not become a PERSON.
    assert_eq!(
        classify_type("DNA di Chiara", true, false, true, "it"),
        "VISUAL_CONCEPT"
    );
    // A real name keeps its person type.
    assert_eq!(
        classify_type("Alberto Stasi", true, false, true, "it"),
        "PERSON"
    );
}

#[test]
fn organization_heads_and_tails_type_multilingual_names_without_person_fallbacks() {
    let samples = [
        ("it", "Fondazione Prisma"),
        ("en", "Prism Laboratory"),
        ("es", "Laboratorio Prisma"),
        ("pt", "Conselho Estrela"),
        ("fr", "Laboratoire Prisma"),
        ("de", "Prisma Stiftung"),
        ("nl", "Prisma Laboratorium"),
        ("pl", "Laboratorium Prisma"),
        ("ru", "Лаборатория Призма"),
        ("id", "Laboratorium Prisma"),
    ];
    for (language, name) in samples {
        assert_eq!(
            classify_type(name, true, false, true, language),
            "ORG",
            "institution marker should prevent PERSON fallback for {language}: {name}"
        );
    }
    assert_eq!(
        classify_type("Fondazione", true, false, true, "it"),
        "VISUAL_CONCEPT",
        "a bare generic noun does not identify an organization"
    );
}

#[test]
fn a_curated_commercial_head_closes_an_organization_name() {
    // Two capitalized tokens with no legal suffix would satisfy the person
    // rule; a curated commercial head makes the same shape an
    // organization, while a real name keeps its type.
    for name in ["Aurora Sport", "Lumen Energia", "Nordwasser Energie"] {
        assert_eq!(
            classify_type(name, true, false, true, "it"),
            "ORGANIZATION",
            "commercial head should type {name} as an organization"
        );
    }
    assert_eq!(
        classify_type("Elena Conti", true, false, true, "it"),
        "PERSON"
    );
    // The head is matched as a whole word: a longer word that merely ends
    // with one of the commercial heads stays a person name.
    assert_eq!(
        classify_type("Elena Sportiva", true, false, true, "it"),
        "PERSON"
    );
}

#[test]
fn capitalization_is_name_evidence_only_where_the_script_makes_it_so() {
    // Two capitalized words are a person in a cased script that keeps
    // common nouns lowercase.
    assert_eq!(
        classify_type("Alice Johnson", true, false, true, "it"),
        "PERSON"
    );
    // Not in German: every noun is capitalized, so "Neue Halle" proves
    // nothing.
    assert_eq!(
        classify_type("Neue Halle", true, false, true, "de"),
        "VISUAL_CONCEPT"
    );
    // The tagger's PROPN restores person recall where case cannot, but a
    // single PROPN is not enough: "Deutschland" is a country.
    assert_eq!(
        classify_type("Alice Johnson", true, true, true, "de"),
        "PERSON"
    );
    assert_eq!(
        classify_type("Deutschland", true, true, true, "de"),
        "VISUAL_CONCEPT"
    );
    // A caseless script never produces the capitalization signal, and a
    // bare PROPN is not evidence of PERSON there (a country is a PROPN).
    assert_eq!(
        classify_type("北京 大学", true, false, true, "zh"),
        "VISUAL_CONCEPT"
    );
    assert_eq!(
        classify_type("日本", true, true, true, "zh"),
        "VISUAL_CONCEPT"
    );
}

#[test]
fn event_heads_pageants_and_lowercase_city_homographs_stay_non_person_non_gpe() {
    assert_eq!(
        classify_type("Black Expo", true, false, true, "it"),
        "EVENT"
    );
    assert_eq!(
        classify_type("Indiana Black Expo", true, false, true, "it"),
        "EVENT"
    );
    assert_eq!(
        classify_type("Miss Black America", true, false, true, "it"),
        "EVENT"
    );
    assert_ne!(
        classify_type("potenza", true, false, true, "it"),
        "LOCATION"
    );
    assert_eq!(
        classify_type("Potenza", true, false, true, "it"),
        "LOCATION"
    );
}

#[test]
fn four_digit_numbers_become_years_only_with_temporal_context() {
    let year_and_quantity = [
        ("it", "Nel 2015 ha venduto 1500 dischi."),
        ("en", "In 2015 Aurora sold 1500 units."),
        ("de", "Im Jahr 2015 verkaufte Aurora 1500 Stück."),
        ("tr", "2015'te Aurora 1500 adet sattı."),
        ("pl", "W 2015 roku Aurora sprzedała 1500 sztuk."),
        ("ru", "В 2015 году Aurora продала 1500 штук."),
    ];
    for (language, source) in year_and_quantity {
        let candidates = value_candidates(source);
        let year = candidates
            .iter()
            .find(|candidate| candidate.text.contains("2015"))
            .unwrap_or_else(|| panic!("{language}: no year candidate: {candidates:?}"));
        assert_eq!(
            year.entity_type.as_deref(),
            Some("DATE"),
            "{language}: temporal context must type 2015 as DATE: {candidates:?}"
        );
        let quantity = candidates
            .iter()
            .find(|candidate| candidate.text.contains("1500"))
            .unwrap_or_else(|| panic!("{language}: no quantity candidate: {candidates:?}"));
        assert_eq!(
            quantity.entity_type.as_deref(),
            Some("NUMBER"),
            "{language}: 1500 is a quantity, not a year: {candidates:?}"
        );
    }

    for source in [
        "Mario 1500, Luca 1800, Marco 1200.",
        "Il codice è 1500, la password è 1800.",
        "1500 dischi sono stati venduti.",
        "Nel 2015 1.500 dischi, nel 2016 1200.",
    ] {
        let candidates = value_candidates(source);
        assert!(
            candidates.iter().any(|candidate| {
                matches!(candidate.entity_type.as_deref(), Some("NUMBER"))
                    && (candidate.text.contains("1500")
                        || candidate.text.contains("1.500")
                        || candidate.text.contains("1800"))
            }),
            "a four-digit quantity must survive year detection: {source}: {candidates:?}"
        );
    }
}
