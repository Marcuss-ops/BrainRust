use super::*;

const SOURCE: &str = "Sales rose 25% in Lisbon. Fado singers perform in Alfama.";

fn options() -> ExtractOptions {
    ExtractOptions {
        language: "en".to_string(),
        entity_count: 10,
    }
}

#[test]
fn every_derived_span_is_verbatim_source() {
    let analysis = analyze(SOURCE, &options()).unwrap();
    for lt in &analysis.lower_thirds {
        assert_eq!(&SOURCE[lt.start..lt.end], lt.evidence);
    }
    for bullet in &analysis.bullets {
        assert_eq!(&SOURCE[bullet.start..bullet.end], bullet.text);
    }
}

#[test]
fn numeric_facts_survive_a_low_entity_budget() {
    // The plan's second killer scenario: more numbers than the visual
    // top-N budget. NumericFacts must carry every numeric occurrence.
    let text = "Il team contava 15 persone, il budget copriva il 42%, i costi passarono da 3 ad 8, e il 2024 chiuse con 1,2 milioni di euro.";
    let analysis = analyze(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    for fact in &analysis.numeric_facts {
        assert_eq!(
            text.get(fact.start_byte..fact.end_byte),
            Some(fact.surface.as_str()),
            "surface must be verbatim at its offsets"
        );
        assert_eq!(fact.evidence, fact.surface);
        assert!(matches!(fact.kind.as_str(), "NUMBER" | "PERCENT" | "MONEY"));
    }
    let surfaces: Vec<&str> = analysis
        .numeric_facts
        .iter()
        .map(|fact| fact.surface.as_str())
        .collect();
    assert!(surfaces.contains(&"42%"), "facts: {surfaces:?}");
    assert!(surfaces.contains(&"15"), "facts: {surfaces:?}");
    // A 4-digit year classifies as DATE, not NUMBER: the temporal
    // projection owns it, so the numeric projection must not duplicate
    // it (one fact, one projection).
    assert!(!surfaces.contains(&"2024"), "facts: {surfaces:?}");
}

#[test]
fn spelled_quantities_are_numeric_facts_but_not_chart_points() {
    let text = "Stasi è stato condannato a sedici anni di reclusione; il deposito di cinque consulenze difensive.";
    let analysis = analyze(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    let facts: Vec<(&str, Option<f64>, &str)> = analysis
        .numeric_facts
        .iter()
        .map(|fact| (fact.surface.as_str(), fact.value, fact.kind.as_str()))
        .collect();
    assert!(
        facts.contains(&("sedici", Some(16.0), "NUMBER")),
        "facts: {facts:?}"
    );
    assert!(
        facts.contains(&("cinque", Some(5.0), "NUMBER")),
        "facts: {facts:?}"
    );
    // The same values must not become bars: a spelled duration/tally has
    // no written unit that makes it a comparable chart value.
    assert!(
        analysis.chart_points.is_empty(),
        "{:?}",
        analysis.chart_points
    );

    let en = analyze(
        "sixteen years and five reports.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    let en_facts: Vec<(&str, Option<f64>)> = en
        .numeric_facts
        .iter()
        .map(|fact| (fact.surface.as_str(), fact.value))
        .collect();
    assert!(
        en_facts.contains(&("sixteen years", Some(16.0))),
        "{en_facts:?}"
    );
    assert!(en_facts.contains(&("five", Some(5.0))), "{en_facts:?}");
}

#[test]
fn spelled_scale_words_multiply_the_parsed_prefix() {
    // CLDR parses "tre milioni" as the prefix 3; the consumed scale noun
    // must multiply it, otherwise a headline becomes a chart bar of 3.
    let it = analyze(
        "Tre milioni di euro sono stati spesi.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    let fact = it
        .numeric_facts
        .iter()
        .find(|fact| fact.surface.eq_ignore_ascii_case("tre milioni"))
        .expect("tre milioni must be one fact");
    assert_eq!(fact.value, Some(3_000_000.0));

    let en = analyze(
        "Three million dollars were spent.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    let fact = en
        .numeric_facts
        .iter()
        .find(|fact| fact.surface.eq_ignore_ascii_case("three million"))
        .expect("three million must be one fact");
    assert_eq!(fact.value, Some(3_000_000.0));
    assert_eq!(fact.kind, "MONEY");
}

#[test]
fn spelled_conjunctions_fractions_and_idioms_are_typed() {
    let it = analyze(
            "Mille e duecento persone sono arrivate. Un terzo degli elettori. Due milioni e mezzo di euro. Mille grazie.",
            &ExtractOptions {
                language: "it".to_string(),
                entity_count: 3,
            },
        )
        .unwrap();
    let approx = analyze(
        "Circa cinquemila persone sono arrivate.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    assert!(
        approx
            .numeric_facts
            .iter()
            .any(|fact| { fact.value == Some(5000.0) && fact.surface.starts_with("Circa") }),
        "approximator span: {:?}",
        approx.numeric_facts
    );
    let values: Vec<(&str, Option<f64>, &str)> = it
        .numeric_facts
        .iter()
        .map(|fact| (fact.surface.as_str(), fact.value, fact.kind.as_str()))
        .collect();
    assert!(
        values
            .iter()
            .any(|(_, value, kind)| *value == Some(1200.0) && *kind == "NUMBER"),
        "values: {values:?}"
    );
    assert!(
        values
            .iter()
            .any(|(_, value, kind)| *value == Some(1.0 / 3.0) && *kind == "FRACTION"),
        "values: {values:?}"
    );
    assert!(
        values
            .iter()
            .any(|(_, value, _)| *value == Some(2_500_000.0)),
        "values: {values:?}"
    );
    // "Mille grazie" is an idiom, not a quantity.
    assert!(!values
        .iter()
        .any(|(surface, _, _)| surface.contains("grazie")));

    let es = analyze(
        "Mil doscientos personas llegaron.",
        &ExtractOptions {
            language: "es".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    assert!(
        es.numeric_facts
            .iter()
            .any(|fact| fact.value == Some(1200.0)),
        "es: {:?}",
        es.numeric_facts
    );

    let tr = analyze(
        "Bin iki yüz kişi geldi.",
        &ExtractOptions {
            language: "tr".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    assert!(
        tr.numeric_facts
            .iter()
            .any(|fact| fact.value == Some(1200.0)),
        "tr: {:?}",
        tr.numeric_facts
    );

    for (language, text) in [
        ("en", "Two and a half million people arrived."),
        ("fr", "Deux millions et demi de personnes sont arrivées."),
        ("pl", "Dwa i pół miliona osób przyjechało."),
        ("ru", "Два с половиной миллиона человек приехали."),
        ("id", "Dua setengah juta orang datang."),
        ("pt", "Dois milhões e meio de pessoas chegaram."),
        ("de", "Zweieinhalb Millionen Menschen kamen."),
        ("nl", "Tweeënhalf miljoen mensen kwamen."),
        ("tr", "iki buçuk milyon kişi geldi."),
    ] {
        let analysis = analyze(
            text,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 3,
            },
        )
        .unwrap();
        assert!(
            analysis
                .numeric_facts
                .iter()
                .any(|fact| fact.value == Some(2_500_000.0)),
            "{language} additive half: {:?}",
            analysis.numeric_facts
        );
    }

    for (language, text) in [
        ("en", "One third of the voters agreed."),
        ("es", "Un tercio de los votantes estuvo de acuerdo."),
        ("fr", "Un tiers des électeurs était d'accord."),
        ("de", "Ein Drittel der Wähler war einverstanden."),
        ("pl", "Jedna trzecia wyborców się zgodziła."),
        ("ru", "Одна треть избирателей согласилась."),
    ] {
        let analysis = analyze(
            text,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 3,
            },
        )
        .unwrap();
        assert!(
            analysis
                .numeric_facts
                .iter()
                .any(|fact| fact.kind == "FRACTION"
                    && (fact.value.unwrap_or_default() - 1.0 / 3.0).abs() < 1e-9),
            "{language} fraction: {:?}",
            analysis.numeric_facts
        );
    }
}

#[test]
fn mixed_digit_and_word_scales_are_one_fact() {
    let it = analyze(
        "Sono stati spesi 3 milioni e mezzo di euro. Il fatturato è cresciuto di 3,2 miliardi.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    let facts: Vec<(&str, Option<f64>, Option<&str>)> = it
        .numeric_facts
        .iter()
        .map(|fact| (fact.surface.as_str(), fact.value, fact.form.as_deref()))
        .collect();
    assert!(
        facts.iter().any(|(surface, value, form)| surface
            .eq_ignore_ascii_case("3 milioni e mezzo")
            && *value == Some(3_500_000.0)
            && *form == Some("mixed")),
        "facts: {facts:?}"
    );
    assert!(
        facts.iter().any(
            |(surface, value, _)| surface.eq_ignore_ascii_case("3,2 miliardi")
                && *value == Some(3_200_000_000.0)
        ),
        "facts: {facts:?}"
    );
    assert_eq!(
        facts
            .iter()
            .filter(|(surface, _, _)| surface.eq_ignore_ascii_case("3"))
            .count(),
        0,
        "the bare 3 must be absorbed: {facts:?}"
    );
}

#[test]
fn spelled_date_parts_do_not_become_numeric_facts() {
    let text = "Il ventiquattro maggio duemilaventicinque si chiude la fase.";
    let analysis = analyze(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    assert!(
        analysis
            .temporal_facts
            .iter()
            .any(|fact| fact.normalized.as_deref() == Some("2025-05-24")),
        "temporal: {:?}",
        analysis.temporal_facts
    );
    assert!(
        analysis
            .numeric_facts
            .iter()
            .all(|fact| !matches!(fact.value, Some(24.0 | 2025.0))),
        "numeric: {:?}",
        analysis.numeric_facts
    );
}

#[test]
fn numeric_facts_parse_units_by_language_convention() {
    let en = analyze(
        "Revenue hit $1,200 and 25% growth.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    let money = en
        .numeric_facts
        .iter()
        .find(|fact| fact.kind == "MONEY")
        .expect("$1,200 must produce a MONEY fact");
    assert_eq!(money.value, Some(1200.0));
    let percent = en
        .numeric_facts
        .iter()
        .find(|fact| fact.kind == "PERCENT" && fact.value == Some(25.0));
    assert!(percent.is_some(), "facts: {:?}", en.numeric_facts);

    // Italian decimal comma: 12,5% parses to 12.5.
    let it = analyze(
        "Le vendite crebbero del 12,5%.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    let percent = it
        .numeric_facts
        .iter()
        .find(|fact| fact.kind == "PERCENT")
        .expect("12,5% must produce a PERCENT fact");
    assert_eq!(percent.value, Some(12.5));
}

#[test]
fn arabic_numbers_with_written_percent_units_keep_percent_type_multilingually() {
    assert!(semantic_core::lexicon::lexicon()
        .scale_words
        .unit_words
        .iter()
        .any(|unit| unit == "per cento"));
    assert_eq!(
        crate::extractor::numeric_value_spans("42 per cento")
            .into_iter()
            .map(|candidate| (candidate.start, candidate.end, candidate.entity_type))
            .collect::<Vec<_>>(),
        vec![(0, "42 per cento".len(), "PERCENT".to_string())]
    );
    let cases = [
        (
            "it",
            "Aurora è al 42 per cento contro Lumen al 18 per cento.",
            [42.0, 18.0],
        ),
        (
            "en",
            "Aurora is at 42 percent versus Lumen at 18 per cent.",
            [42.0, 18.0],
        ),
        (
            "es",
            "Aurora llega al 42 por ciento frente a Lumen con 18 por ciento.",
            [42.0, 18.0],
        ),
        (
            "pt",
            "Aurora chega a 42 por cento contra 18 por cento de Lumen.",
            [42.0, 18.0],
        ),
        (
            "fr",
            "Aurora atteint 42 pour cent contre 18 pour cent pour Lumen.",
            [42.0, 18.0],
        ),
        (
            "de",
            "Aurora erreicht 42 Prozent gegenüber 18 Prozent bei Lumen.",
            [42.0, 18.0],
        ),
        (
            "nl",
            "Aurora haalt 42 procent tegenover 18 procent voor Lumen.",
            [42.0, 18.0],
        ),
        (
            "pl",
            "Aurora osiąga 42 procent wobec 18 procent Lumen.",
            [42.0, 18.0],
        ),
        (
            "ru",
            "Аврора достигает 42 процента против 18 процентов у Люмена.",
            [42.0, 18.0],
        ),
        (
            "id",
            "Aurora mencapai 42 persen dibandingkan Lumen 18 persen.",
            [42.0, 18.0],
        ),
    ];
    for (language, text, expected) in cases {
        let analysis = analyze(
            text,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 16,
            },
        )
        .unwrap();
        let mut percentages: Vec<_> = analysis
            .numeric_facts
            .iter()
            .filter(|fact| fact.kind == "PERCENT")
            .filter_map(|fact| fact.value)
            .collect();
        percentages.sort_by(f64::total_cmp);
        let mut expected = expected;
        expected.sort_by(f64::total_cmp);
        assert_eq!(
            percentages, expected,
            "{language}: {:?}",
            analysis.numeric_facts
        );
    }
}

#[test]
fn relations_stay_inside_one_sentence() {
    let analysis = analyze(SOURCE, &options()).unwrap();
    for rel in &analysis.relations {
        let a = &analysis.entities[rel.source];
        let b = &analysis.entities[rel.target];
        assert!(rel.evidence.contains(&a.text));
        assert!(rel.evidence.contains(&b.text));
    }
}

#[test]
fn one_lower_third_per_entity_and_chart_value_is_plain() {
    let analysis = analyze(SOURCE, &options()).unwrap();
    assert_eq!(analysis.lower_thirds.len(), analysis.entities.len());
    let percent = analysis
        .chart_points
        .iter()
        .find(|point| point.kind == "PERCENT")
        .expect("25% must be a chart point");
    assert_eq!(percent.value, 25.0);
}

#[test]
fn parses_only_plain_numeric_surfaces() {
    assert_eq!(parse_unambiguous_value("25%", "en"), Some(25.0));
    assert_eq!(parse_unambiguous_value("$1,200", "en"), Some(1200.0));
    assert_eq!(parse_unambiguous_value("3.5", "en"), Some(3.5));
    assert_eq!(parse_unambiguous_value("25 percent", "en"), Some(25.0));
    assert_eq!(parse_unambiguous_value("1.5 million", "en"), None);
    assert_eq!(parse_unambiguous_value("March 2025", "en"), None);
}

#[test]
fn decimal_comma_languages_use_dot_thousands() {
    assert_eq!(parse_unambiguous_value("1.200 euro", "it"), Some(1200.0));
    assert_eq!(
        parse_unambiguous_value("1.200.000", "de"),
        Some(1_200_000.0)
    );
    assert_eq!(parse_unambiguous_value("12,5%", "it"), Some(12.5));
    // A lone comma followed by three digits is ambiguous in Italian: refuse.
    assert_eq!(parse_unambiguous_value("1,200", "it"), None);
    assert_eq!(parse_unambiguous_value("$1,200", "it"), None);
    assert_eq!(parse_unambiguous_value("1,25", "it"), Some(1.25));
    // Not a valid grouping in either convention: refuse rather than guess.
    assert_eq!(parse_unambiguous_value("3.5", "it"), None);
    assert_eq!(parse_unambiguous_value("1.2.3", "it"), None);
    assert_eq!(parse_unambiguous_value("1,2,3", "it"), None);
}

#[test]
fn overlapping_numeric_spans_yield_one_chart_point() {
    let source = "Costs rose 25% to $1,200 in 2025.";
    let entities = vec![
        VisualEntity {
            text: "25".into(),
            r#type: "NUMBER".into(),
            start: 12,
            end: 14,
            score: 1.0,
            evidence: String::new(),
        },
        VisualEntity {
            text: "25%".into(),
            r#type: "PERCENT".into(),
            start: 12,
            end: 15,
            score: 1.0,
            evidence: String::new(),
        },
        VisualEntity {
            text: "$1,200".into(),
            r#type: "MONEY".into(),
            start: 19,
            end: 25,
            score: 1.0,
            evidence: String::new(),
        },
        VisualEntity {
            text: "1,200".into(),
            r#type: "NUMBER".into(),
            start: 20,
            end: 25,
            score: 1.0,
            evidence: String::new(),
        },
    ];
    let sentences = split_sentences(source, "en", None);
    let facts = numeric_facts(source, "en", &[], &[]);
    let points = chart_points(source, "en", &facts, &sentences, &entities);
    let values: Vec<f64> = points.iter().map(|p| p.value).collect();
    assert_eq!(values, vec![25.0, 1200.0]);
}

#[test]
fn inner_number_survives_when_outer_span_has_no_value() {
    let source = "Units: 3.25 units sold.";
    let entities = vec![
        VisualEntity {
            text: "3.25 units".into(),
            r#type: "NUMBER".into(),
            start: 7,
            end: 17,
            score: 1.0,
            evidence: String::new(),
        },
        VisualEntity {
            text: "3.25".into(),
            r#type: "NUMBER".into(),
            start: 7,
            end: 11,
            score: 1.0,
            evidence: String::new(),
        },
    ];
    let sentences = split_sentences(source, "en", None);
    let facts = numeric_facts(source, "en", &[], &[]);
    let points = chart_points(source, "en", &facts, &sentences, &entities);
    let values: Vec<f64> = points.iter().map(|p| p.value).collect();
    assert_eq!(values, vec![3.25]);
}

#[test]
fn decimal_point_does_not_split_sentences() {
    let sentences = split_sentences("Pi is 3.14. Done", "en", None);
    assert_eq!(sentences.len(), 2);
    assert_eq!(
        &"Pi is 3.14. Done"[sentences[0].start_byte..sentences[0].end_byte],
        "Pi is 3.14."
    );
}

#[test]
fn co_mentions_stay_bounded_on_one_huge_sentence() {
    use crate::types::VisualEntity;

    let sentence = "word ".repeat(4000);
    let sentences = vec![SentenceSpan {
        start_byte: 0,
        end_byte: sentence.len(),
    }];
    let mut entities = Vec::new();
    for index in 0..400 {
        let text = format!("Persona{index:03} Cognome");
        entities.push(VisualEntity {
            text: text.clone(),
            r#type: "PERSON".to_string(),
            score: 1.0,
            start: index * 10,
            end: index * 10 + text.len(),
            evidence: text,
        });
    }
    let member_of: Vec<Option<usize>> = (0..entities.len()).map(|_| Some(0)).collect();
    let pairs = relations(&sentence, &entities, &sentences, &member_of);
    assert!(
        pairs.len()
            <= MAX_CO_MENTION_SUBJECTS_PER_SENTENCE * (MAX_CO_MENTION_SUBJECTS_PER_SENTENCE - 1)
                / 2,
        "one sentence produced {} relations",
        pairs.len()
    );
    // A generic phrase is not a co-mention partner.
    let mut generic = entities.clone();
    generic[0].r#type = "VISUAL_CONCEPT".to_string();
    let pairs = relations(&sentence, &generic, &sentences, &member_of);
    assert!(
        pairs
            .iter()
            .all(|relation| relation.source != 0 && relation.target != 0),
        "a VISUAL_CONCEPT entered a co-mention"
    );
}
