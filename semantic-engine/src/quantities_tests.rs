use super::*;
use serde_json::json;

fn fact(source: &str, surface: &str, kind: &str, value: Option<f64>) -> NumericFact {
    let start = source.find(surface).unwrap();
    NumericFact {
        surface: surface.to_string(),
        kind: kind.to_string(),
        value,
        start_byte: start,
        end_byte: start + surface.len(),
        evidence: surface.to_string(),
        form: None,
    }
}

#[test]
fn attaches_currency_scale_predicate_and_same_sentence_year() {
    let source = "Il fatturato è 3,2 miliardi di euro nel 2019.";
    let number = fact(source, "3,2 miliardi", "MONEY", Some(3_200_000_000.0));
    let date = semantic_core::temporal_facts(source, "it");
    let quantities = extract(source, "it", &[number], &date);
    assert_eq!(quantities.len(), 1);
    let quantity = &quantities[0];
    assert_eq!(quantity.dimension, "currency");
    assert_eq!(quantity.unit, "EUR");
    assert_eq!(quantity.scale, Some(1_000_000_000.0));
    assert_eq!(quantity.value, Some(3_200_000_000.0));
    assert_eq!(quantity.predicate.as_deref(), Some("revenue"));
    assert_eq!(
        quantity.period.as_ref().map(|period| period.year),
        Some(2019)
    );
    assert_eq!(
        &source[quantity.start_byte..quantity.end_byte],
        quantity.evidence
    );
}

#[test]
fn ignores_years_without_units_and_does_not_cross_sentence_boundaries() {
    let source = "Nel 2019 la popolazione è cresciuta. Il fatturato era 3 milioni di euro.";
    let number = fact(source, "3", "NUMBER", Some(3.0));
    let dates = semantic_core::temporal_facts(source, "it");
    let quantities = extract(source, "it", &[number], &dates);
    assert_eq!(quantities.len(), 1);
    assert_eq!(quantities[0].dimension, "currency");
    assert!(quantities[0].period.is_none());
}

#[test]
fn a_later_quantity_cannot_supply_the_previous_number_scale() {
    let source = "Il gruppo ha 2 euro contro 3 milioni di euro.";
    let facts = [
        fact(source, "2", "NUMBER", Some(2.0)),
        fact(source, "3", "NUMBER", Some(3.0)),
    ];
    let quantities = extract(source, "it", &facts, &[]);
    assert_eq!(quantities.len(), 2);
    assert_eq!(quantities[0].scale, None);
    assert_eq!(quantities[0].value, Some(2.0));
    assert_eq!(quantities[1].scale, Some(1_000_000.0));
    assert_eq!(quantities[1].value, Some(3_000_000.0));
}

#[test]
fn uses_the_language_unit_table_for_population_and_percent() {
    let population = "La città ha 3 milioni di abitanti.";
    let q = extract(
        population,
        "it",
        &[fact(population, "3", "NUMBER", Some(3.0))],
        &[],
    );
    assert_eq!(q[0].dimension, "population");
    assert_eq!(q[0].unit, "person");
    let percentage = "Revenue grew by 12%.";
    let q = extract(
        percentage,
        "en",
        &[fact(percentage, "12%", "PERCENT", Some(12.0))],
        &[],
    );
    assert_eq!(q[0].dimension, "percent");
}

#[test]
fn unknown_languages_do_not_borrow_another_language_lexicon() {
    let source = json!("The city has 3 million inhabitants.")
        .as_str()
        .unwrap()
        .to_string();
    let number = fact(&source, "3", "NUMBER", Some(3.0));
    assert!(extract(&source, "ru", &[number], &[]).is_empty());
}

#[test]
fn comparisons_require_the_same_dimension_and_currency_year() {
    let source = "Il gruppo A ha 42% contro il 18% del gruppo B.";
    let facts = [
        fact(source, "42%", "PERCENT", Some(42.0)),
        fact(source, "18%", "PERCENT", Some(18.0)),
    ];
    let quantities = extract(source, "it", &facts, &[]);
    let found_comparisons = comparisons(source, "it", &quantities);
    assert_eq!(found_comparisons.len(), 1);
    assert_eq!(found_comparisons[0].kind, "difference");
    assert_eq!(found_comparisons[0].difference, Some(24.0));

    let mixed = "Il gruppo A ha 42% contro 18 euro.";
    let facts = [
        fact(mixed, "42%", "PERCENT", Some(42.0)),
        fact(mixed, "18", "MONEY", Some(18.0)),
    ];
    let quantities = extract(mixed, "it", &facts, &[]);
    assert!(comparisons(mixed, "it", &quantities).is_empty());

    let cross_currency = "Il valore è 10 EUR contro 5 USD.";
    let facts = [
        fact(cross_currency, "10", "MONEY", Some(10.0)),
        fact(cross_currency, "5", "MONEY", Some(5.0)),
    ];
    let quantities = extract(cross_currency, "it", &facts, &[]);
    assert!(comparisons(cross_currency, "it", &quantities).is_empty());

    let ratio = "Il valore è 10% rispetto al doppio di 5%.";
    let facts = [
        fact(ratio, "10%", "PERCENT", Some(10.0)),
        fact(ratio, "5%", "PERCENT", Some(5.0)),
    ];
    let quantities = extract(ratio, "it", &facts, &[]);
    let found_comparisons = comparisons(ratio, "it", &quantities);
    assert_eq!(found_comparisons.len(), 1);
    assert_eq!(found_comparisons[0].kind, "ratio");
    assert_eq!(found_comparisons[0].ratio, Some(2.0));
}

#[test]
fn explicit_change_cues_measure_new_value_minus_previous_value() {
    for (language, source, previous_value, current_value, expected) in [
        ("en", "5% rose to 10%.", 5.0, 10.0, 5.0),
        ("en", "10% fell to 5%.", 10.0, 5.0, -5.0),
        ("tr", "5% yükselerek 10% oldu.", 5.0, 10.0, 5.0),
    ] {
        let facts = [
            fact(
                source,
                if previous_value == 5.0 { "5%" } else { "10%" },
                "PERCENT",
                Some(previous_value),
            ),
            fact(
                source,
                if current_value == 5.0 { "5%" } else { "10%" },
                "PERCENT",
                Some(current_value),
            ),
        ];
        let quantities = extract(source, language, &facts, &[]);
        let found = comparisons(source, language, &quantities);
        assert_eq!(found.len(), 1, "{language}: {source}: {found:?}");
        assert_eq!(found[0].kind, "change", "{language}: {source}");
        assert_eq!(found[0].difference, Some(expected), "{language}: {source}");
    }
}

#[test]
fn localized_ratio_cues_are_data_driven_in_all_supported_languages() {
    let examples = [
        ("de", "10% im Vergleich zum Doppelten von 5%"),
        ("en", "10% compared with twice 5%"),
        ("es", "10% frente al doble de 5%"),
        ("fr", "10% par rapport au double de 5%"),
        ("id", "10% dibandingkan dengan dua kali 5%"),
        ("it", "10% rispetto al doppio di 5%"),
        ("nl", "10% tegenover het dubbele van 5%"),
        ("pl", "10% wobec dwukrotności 5%"),
        ("pt", "10% contra o dobro de 5%"),
        ("ru", "10% против удвоенных 5%"),
        ("tr", "10% karşısında iki kat 5%"),
    ];

    for (language, source) in examples {
        let facts = [
            fact(source, "10%", "PERCENT", Some(10.0)),
            fact(source, "5%", "PERCENT", Some(5.0)),
        ];
        let quantities = extract(source, language, &facts, &[]);
        let found = comparisons(source, language, &quantities);
        assert_eq!(found.len(), 1, "{language}: {source}; {found:?}");
        assert_eq!(found[0].kind, "ratio", "{language}: {source}");
        assert_eq!(found[0].ratio, Some(2.0), "{language}: {source}");
    }
}

#[test]
fn common_currency_units_are_attached_in_every_supported_language() {
    let currencies = [
        ("de", "euro"),
        ("en", "euro"),
        ("es", "euro"),
        ("fr", "euro"),
        ("id", "euro"),
        ("it", "euro"),
        ("nl", "euro"),
        ("pl", "euro"),
        ("pt", "euro"),
        ("ru", "евро"),
        ("tr", "euro"),
    ];
    for (language, surface) in currencies {
        let source = format!("5 {surface}");
        let number = fact(&source, "5", "MONEY", Some(5.0));
        let quantities = extract(&source, language, &[number], &[]);
        assert_eq!(
            quantities.first().map(|quantity| quantity.unit.as_str()),
            Some("EUR"),
            "language={language}, source={source}"
        );
    }
}

#[test]
fn compound_speed_units_win_over_the_length_prefix_in_every_language() {
    for language in [
        "de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr",
    ] {
        let unit = language_units(language)
            .unwrap()
            .units
            .iter()
            .find(|unit| unit.dimension == "speed")
            .unwrap();
        let surface = unit
            .surfaces
            .iter()
            .find(|surface| surface.contains('/'))
            .unwrap();
        let source = format!("5 {surface}");
        let quantities = extract(
            &source,
            language,
            &[fact(&source, "5", "NUMBER", Some(5.0))],
            &[],
        );
        assert_eq!(quantities.len(), 1, "language={language}, source={source}");
        assert_eq!(quantities[0].dimension, "speed", "{source}");
        assert_eq!(quantities[0].unit, "km/h", "{source}");
    }
}

#[test]
fn years_and_frequency_counts_do_not_become_typed_quantities() {
    let year = "Nel 2019 la popolazione è cresciuta.";
    let year_fact = fact(year, "2019", "NUMBER", Some(2019.0));
    assert!(extract(year, "it", &[year_fact], &[]).is_empty());

    // TICKET-43 promotes the bare multiplier ("3 volte") to a ratio, but
    // the frequency idiom with a period cue stays untyped.
    let frequency = "La squadra si allena 3 volte al giorno.";
    let count = fact(frequency, "3", "NUMBER", Some(3.0));
    assert!(extract(frequency, "it", &[count], &[]).is_empty());
    let multiplier = "La squadra ha vinto 3 volte.";
    let count = fact(multiplier, "3", "NUMBER", Some(3.0));
    let quantities = extract(multiplier, "it", &[count], &[]);
    assert_eq!(quantities.len(), 1);
    assert_eq!(quantities[0].dimension, "ratio");
}

#[test]
fn a_range_can_share_one_unit_but_a_comparison_cannot_steal_the_later_unit() {
    let source = "Il percorso misura tra 3 e 5 km.";
    let facts = [
        fact(source, "3", "NUMBER", Some(3.0)),
        fact(source, "5", "NUMBER", Some(5.0)),
    ];
    let quantities = extract(source, "it", &facts, &[]);
    assert_eq!(quantities.len(), 2);
    assert_eq!(quantities[0].unit, "km");
    assert_eq!(quantities[1].unit, "km");
    assert_eq!(quantities[0].qualifier, "range_start");
    assert_eq!(quantities[1].qualifier, "range_end");
    assert_eq!(quantities[0].range_id, quantities[1].range_id);

    let scaled_range =
            "Il costo previsto è compreso tra 10 e 15 milioni di euro, rispetto agli 8 milioni di budget.";
    let facts = [
        fact(scaled_range, "10", "NUMBER", Some(10.0)),
        fact(scaled_range, "15 milioni", "NUMBER", Some(15_000_000.0)),
        fact(scaled_range, "8 milioni", "NUMBER", Some(8_000_000.0)),
    ];
    let quantities = extract(scaled_range, "it", &facts, &[]);
    assert_eq!(quantities.len(), 2, "quantities: {quantities:#?}");
    assert_eq!(quantities[0].qualifier, "range_start");
    assert_eq!(quantities[0].scale, Some(1_000_000.0));
    assert_eq!(quantities[0].value, Some(10_000_000.0));
    assert_eq!(quantities[1].value, Some(15_000_000.0));

    let comparison = "Il gruppo ha 3 contro 5 km.";
    let facts = [
        fact(comparison, "3", "NUMBER", Some(3.0)),
        fact(comparison, "5", "NUMBER", Some(5.0)),
    ];
    let quantities = extract(comparison, "it", &facts, &[]);
    assert_eq!(
        quantities.len(),
        1,
        "the first value must not steal the later unit"
    );
    assert_eq!(quantities[0].evidence, "5");
}

#[test]
fn ratio_dimension_is_grounded_across_eleven_languages() {
    let cases = [
        ("it", "Il rapporto è 3 su 4.", "3", "4"),
        ("en", "The ratio is 3 out of 4.", "3", "4"),
        ("de", "Das Verhältnis ist 3 von 4.", "3", "4"),
        ("es", "La proporción es 3 de cada 4.", "3", "4"),
        ("fr", "Le ratio est 3 sur 4.", "3", "4"),
        ("nl", "De verhouding is 3 van 4.", "3", "4"),
        ("pl", "Stosunek wynosi 3 na 4.", "3", "4"),
        ("pt", "A proporção é 3 em cada 4.", "3", "4"),
        ("ru", "Пропорция 3 из 4.", "3", "4"),
        ("id", "Rasionya 3 dari 4.", "3", "4"),
        ("tr", "Oran 3 kat 4.", "3", "4"),
    ];
    for (language, source, first, second) in cases {
        let facts = [
            fact(source, first, "NUMBER", Some(3.0)),
            fact(source, second, "NUMBER", Some(4.0)),
        ];
        let quantities = extract(source, language, &facts, &[]);
        assert!(
            quantities
                .iter()
                .any(|quantity| quantity.dimension == "ratio"
                    && quantity.start_byte == source.find(first).unwrap()),
            "{language}: {source}: {quantities:#?}"
        );
    }
}

#[test]
fn ratio_frequency_idioms_are_not_ratios() {
    for (language, source, surface) in [
        ("it", "Lo prende 3 volte al giorno.", "3"),
        ("en", "He takes it 3 times a day.", "3"),
        ("de", "Er nimmt es 3 mal pro Tag.", "3"),
        ("es", "Lo toma 3 veces al día.", "3"),
        ("fr", "Il le prend 3 fois par jour.", "3"),
        ("nl", "Hij neemt het 3 keer per dag.", "3"),
        ("pl", "Bierze to 3 razy dziennie.", "3"),
        ("pt", "Ele toma 3 vezes por dia.", "3"),
        ("ru", "Он принимает 3 раза в день.", "3"),
        ("id", "Dia meminumnya 3 kali per hari.", "3"),
        ("tr", "Günde 3 kez alıyor.", "3"),
    ] {
        let facts = [fact(source, surface, "NUMBER", Some(3.0))];
        let quantities = extract(source, language, &facts, &[]);
        assert!(
            quantities
                .iter()
                .all(|quantity| quantity.dimension != "ratio"),
            "{language}: {source}: {quantities:#?}"
        );
    }
}

#[test]
fn growth_rate_dimension_is_grounded_across_eleven_languages() {
    let cases = [
        ("en", "Revenue grew 12% YoY.", "12%"),
        ("de", "Der Umsatz wuchs 12% jährlich.", "12%"),
        ("it", "Il fatturato è cresciuto del 12% annuo.", "12%"),
        ("es", "Los ingresos crecieron un 12% interanual.", "12%"),
        ("fr", "Le chiffre a augmenté de 12% annuel.", "12%"),
        ("nl", "De omzet groeide 12% jaarlijks.", "12%"),
        ("pl", "Przychody wzrosły o 12% rocznie.", "12%"),
        ("pt", "A receita cresceu 12% anual.", "12%"),
        ("ru", "Выручка выросла на 12% ежегодно.", "12%"),
        ("id", "Pendapatan tumbuh 12% tahunan.", "12%"),
        ("tr", "Gelir 12% yıllık arttı.", "12%"),
    ];
    for (language, source, surface) in cases {
        let facts = [fact(source, surface, "PERCENT", Some(12.0))];
        let quantities = extract(source, language, &facts, &[]);
        assert!(
            quantities
                .iter()
                .any(|quantity| quantity.dimension == "growth_rate"),
            "{language}: {source}: {quantities:#?}"
        );
    }
}

#[test]
fn a_plain_percentage_is_not_a_growth_rate() {
    for (language, source) in [
        ("en", "12% of the people agreed."),
        ("it", "Il 12% delle persone è d'accordo."),
        ("de", "12% der Menschen stimmten zu."),
    ] {
        let facts = [fact(source, "12%", "PERCENT", Some(12.0))];
        let quantities = extract(source, language, &facts, &[]);
        assert!(
            quantities
                .iter()
                .all(|quantity| quantity.dimension != "growth_rate"),
            "{language}: {source}: {quantities:#?}"
        );
        assert!(quantities
            .iter()
            .any(|quantity| quantity.dimension == "percent"));
    }
}

#[test]
fn rank_dimension_is_grounded_across_eleven_languages() {
    let cases = [
        ("en", "She finished in 3rd place.", "3rd"),
        ("it", "È arrivata al 3° posto.", "3"),
        ("de", "Sie belegte den 3 Platz.", "3"),
        ("es", "Quedó en el 3 puesto.", "3"),
        ("fr", "Elle a fini à la 3 place.", "3"),
        ("nl", "Zij eindigde op de 3 plaats.", "3"),
        ("pl", "Zajęła 3 miejsce.", "3"),
        ("pt", "Ela terminou em 3 lugar.", "3"),
        ("ru", "Она заняла 3 место.", "3"),
        ("id", "Dia finis di 3 tempat.", "3"),
        ("tr", "Yarışı 3 sıra bitirdi.", "3"),
    ];
    for (language, source, surface) in cases {
        let facts = [fact(source, surface, "NUMBER", Some(3.0))];
        let quantities = extract(source, language, &facts, &[]);
        assert!(
            quantities
                .iter()
                .any(|quantity| quantity.dimension == "rank"),
            "{language}: {source}: {quantities:#?}"
        );
    }
}

#[test]
fn plain_counts_are_not_ranks() {
    for (language, source) in [
        ("en", "She bought 3 apples."),
        ("it", "Ha comprato 3 mele."),
        ("de", "Sie kaufte 3 Äpfel."),
    ] {
        let facts = [fact(source, "3", "NUMBER", Some(3.0))];
        let quantities = extract(source, language, &facts, &[]);
        assert!(
            quantities
                .iter()
                .all(|quantity| quantity.dimension != "rank"),
            "{language}: {source}: {quantities:#?}"
        );
    }
}
