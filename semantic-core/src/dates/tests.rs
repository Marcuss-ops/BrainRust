use super::*;

fn facts(text: &str, language: &str) -> Vec<TemporalFact> {
    temporal_facts(text, language)
}

fn assert_grounded(text: &str, all: &[TemporalFact]) {
    for fact in all {
        assert_eq!(
            text.get(fact.start_byte..fact.end_byte)
                .unwrap_or("OUT-OF-RANGE"),
            fact.surface,
            "surface must be verbatim at its offsets"
        );
        assert_eq!(fact.evidence, fact.surface);
        assert_eq!(fact.kind, "DATE");
    }
    let mut previous: Option<&TemporalFact> = None;
    for fact in all {
        if let Some(last) = previous {
            assert!(
                fact.start_byte > last.start_byte
                    || (fact.start_byte == last.start_byte && fact.end_byte >= last.end_byte),
                "facts must be ordered by offset"
            );
        }
        previous = Some(fact);
    }
}

fn spelled(text: &str, surface: &str, value: i64) -> SpelledNumber {
    let start = text.find(surface).expect("fixture surface");
    SpelledNumber {
        start,
        end: start + surface.len(),
        value,
    }
}

#[test]
fn spelled_italian_day_month_year_normalizes_to_iso() {
    let text = "Il tredici agosto duemilasette è il punto di partenza.";
    let words = [
        spelled(text, "tredici", 13),
        spelled(text, "duemilasette", 2007),
    ];
    let all = temporal_facts_with_words(text, "it", &words);
    assert_grounded(text, &all);
    let fact = all
        .iter()
        .find(|fact| fact.normalized.as_deref() == Some("2007-08-13"))
        .unwrap_or_else(|| panic!("spelled date must normalize: {all:?}"));
    assert_eq!(fact.precision, "day");
    assert_eq!(fact.surface, "tredici agosto duemilasette");
}

#[test]
fn spelled_italian_year_only_and_spaced_year() {
    let year_text = "Nel duemilaventicinque il nome torna.";
    let year = temporal_facts_with_words(
        year_text,
        "it",
        &[spelled(year_text, "duemilaventicinque", 2025)],
    );
    assert!(year
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2025")));
    let spaced_text = "La svolta arrivò nel due mila sette.";
    let spaced = temporal_facts_with_words(
        spaced_text,
        "it",
        &[spelled(spaced_text, "due mila sette", 2007)],
    );
    assert!(spaced
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2007")));
    let spring_text = "Il ventiquattro maggio del duemilaventicinque si chiude.";
    let spring = temporal_facts_with_words(
        spring_text,
        "it",
        &[
            spelled(spring_text, "ventiquattro", 24),
            spelled(spring_text, "duemilaventicinque", 2025),
        ],
    );
    assert!(spring
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2025-05-24")));
}

#[test]
fn a_spelled_date_does_not_emit_a_nested_year_fact() {
    let text = "Il tredici agosto duemilasette è il punto di partenza.";
    let words = [
        spelled(text, "tredici", 13),
        spelled(text, "duemilasette", 2007),
    ];
    let all = temporal_facts_with_words(text, "it", &words);
    assert_eq!(all.len(), 1, "one date, one fact: {all:#?}");
    assert_eq!(all[0].normalized.as_deref(), Some("2007-08-13"));
    assert_eq!(all[0].precision, "day");
}

#[test]
fn coordinated_days_share_the_month_and_year() {
    let text = "Gli atti riportano i giorni 19, 20 e 26 settembre 2007.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    let dates: Vec<&str> = all
        .iter()
        .filter_map(|fact| fact.normalized.as_deref())
        .collect();
    assert_eq!(dates, vec!["2007-09-19", "2007-09-20", "2007-09-26"]);
}

#[test]
fn day_month_year_italian_normalizes_to_iso() {
    let text = "La svolta arrivò il 15 marzo 2008.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    let fact = all
        .iter()
        .find(|fact| fact.normalized.as_deref() == Some("2008-03-15"))
        .expect("15 marzo 2008 must normalize to 2008-03-15");
    assert_eq!(fact.precision, "day");
    assert!(fact.surface.contains("15 marzo 2008"));
    assert!(!fact.ambiguous);
}

#[test]
fn turkish_month_suffix_keeps_the_long_name_over_the_english_prefix() {
    // The ASCII-only boundary check lets "may" match inside "mayıs";
    // the generated tie-break order must keep the long name winning.
    let text = "5 mayıs 2020 tarihinde başlandı.";
    let all = facts(text, "tr");
    assert_grounded(text, &all);
    let fact = all
        .iter()
        .find(|fact| fact.normalized.as_deref() == Some("2020-05-05"))
        .expect("5 mayıs 2020 must normalize to 2020-05-05");
    assert!(
        fact.surface.contains("mayıs"),
        "the long Turkish name must win the tie: {}",
        fact.surface
    );
}

#[test]
fn english_may_still_names_a_month() {
    let text = "The launch was May 5, 2020.";
    let all = facts(text, "en");
    assert_grounded(text, &all);
    assert!(
        all.iter()
            .any(|fact| fact.normalized.as_deref() == Some("2020-05-05")),
        "May 5, 2020 must normalize to 2020-05-05: {all:?}"
    );
}

#[test]
fn cldr_generated_names_cover_accented_and_new_month_forms() {
    for (text, language, iso) in [
        ("12 mayo 2020.", "es", "2020-05-12"),
        ("15 août 2020.", "fr", "2020-08-15"),
        ("15 aout 2020.", "fr", "2020-08-15"),
        ("3 maret 2020.", "id", "2020-03-03"),
    ] {
        let all = facts(text, language);
        assert!(
            all.iter()
                .any(|fact| fact.normalized.as_deref() == Some(iso)),
            "{text} must normalize to {iso}: {all:?}"
        );
    }
}

#[test]
fn multibyte_month_names_never_bisect_a_code_point() {
    // The month scan used to advance one BYTE, so a Cyrillic month name
    // ("марта") put the cursor inside 'м' and the worker panicked on the
    // next slice. Two Cyrillic months and a repeated one must produce
    // grounded facts instead of a crash.
    let text = "Выручка выросла 12 марта 2007 года, а отчёт вышел 3 апреля 2008 года.";
    let all = facts(text, "ru");
    assert_grounded(text, &all);
    assert!(
        all.iter()
            .any(|fact| fact.normalized.as_deref() == Some("2007-03-12")),
        "12 марта 2007 must normalize: {all:?}"
    );
    assert!(
        all.iter()
            .any(|fact| fact.normalized.as_deref() == Some("2008-04-03")),
        "3 апреля 2008 must normalize: {all:?}"
    );
}

#[test]
fn unbounded_month_inside_a_word_never_bisects_the_following_code_point() {
    // Regression: two "février" occurrences inside longer words made the
    // scan advance by the width read at the wrong base offset, landing the
    // next slice inside the 'é' of the second occurrence.
    let text = "xfévrierxéfévrierx";
    let all = facts(text, "fr");
    assert_grounded(text, &all);
}

#[test]
fn repeated_month_name_after_an_earlier_month_terminates_and_keeps_dates() {
    // A repeated month word after a different month used to move the
    // month-search cursor backwards and spin forever in long scripts.
    let text = "Il 23 febbraio iniziò l'udienza, il 9 aprile partì il processo. Il 30 aprile furono disposti altri esami.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    assert!(all.iter().any(|fact| fact.surface == "23 febbraio"));
    assert!(all.iter().any(|fact| fact.surface == "9 aprile"));
    assert!(all.iter().any(|fact| fact.surface == "30 aprile"));
}

#[test]
fn six_dates_survive_with_top_n_three() {
    // The plan's killer scenario: 6 dates in one text with a top-N=3
    // visual budget. TemporalFacts must preserve them all.
    let text = "Nel 1998 avvenne il primo evento, il 12 marzo 2001 arrivò la \
svolta, il 25 luglio 2005 tutto cambiò, il 2 giugno 2010 si consolidò, \
e il 7 settembre 2019 completò la storia.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    let resolved: Vec<&str> = all
        .iter()
        .filter(|fact| fact.precision != "unresolved")
        .map(|fact| fact.normalized.as_deref().unwrap_or(""))
        .collect();
    assert!(resolved.contains(&"1998"), "bare year must survive");
    assert!(resolved.contains(&"2001-03-12"));
    assert!(resolved.contains(&"2005-07-25"));
    assert!(resolved.contains(&"2010-06-02"));
    assert!(resolved.contains(&"2019-09-07"), "resolved: {resolved:?}");
    assert!(all.len() >= 5, "expected at least five facts, got: {all:?}");
}

#[test]
fn english_month_day_comma_year() {
    let text = "On March 12, 2001 the deal closed.";
    let all = facts(text, "en");
    assert_grounded(text, &all);
    assert!(all
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2001-03-12")));
}

#[test]
fn unlisted_language_uses_the_default_date_order() {
    // The day/month order is read from the profile's `default` entry for
    // a language without its own: day-first, never English month-first.
    let all = facts("Il 03/04/2020 cambiò tutto.", "xx");
    let fact = all
        .iter()
        .find(|fact| fact.ambiguous)
        .expect("03/04/2020 must be flagged ambiguous");
    assert_eq!(fact.normalized.as_deref(), Some("2020-04-03"));
}

#[test]
fn ambiguous_numeric_uses_language_convention_and_flags() {
    // Italian day-first: 3 April 2020.
    let it = facts("Il 03/04/2020 cambiò tutto.", "it");
    let it_fact = it
        .iter()
        .find(|fact| fact.ambiguous)
        .expect("03/04/2020 must be flagged ambiguous");
    assert_eq!(it_fact.normalized.as_deref(), Some("2020-04-03"));
    assert_eq!(it_fact.precision, "day");

    // English month-first: March 4, 2020.
    let en = facts("On 03/04/2020 everything changed.", "en");
    let en_fact = en
        .iter()
        .find(|fact| fact.ambiguous)
        .expect("03/04/2020 must be flagged ambiguous");
    assert_eq!(en_fact.normalized.as_deref(), Some("2020-03-04"));
}

#[test]
fn impossible_month_forces_day_first_unambiguously() {
    let en = facts("On 13/04/2020 the app launched.", "en");
    let fact = en
        .iter()
        .find(|fact| fact.normalized.as_deref() == Some("2020-04-13"))
        .expect("13 cannot be a month; day-first must win");
    assert_eq!(fact.precision, "day");
    assert!(!fact.ambiguous, "13/04/2020 is not order-ambiguous");
}

#[test]
fn impossible_calendar_dates_stay_presentable_but_unresolved() {
    let all = facts("Il 31/02/2020 era una data impossibile.", "it");
    let fact = all
        .iter()
        .find(|fact| fact.surface.contains("31/02/2020"))
        .expect("the impossible surface must still be reported");
    assert_eq!(fact.precision, "unresolved");
    assert_eq!(fact.normalized, None, "no invented ISO value");
}

#[test]
fn year_only_never_invents_day_or_month() {
    let all = facts("Nel 2001 tutto cambiò.", "it");
    let fact = all
        .iter()
        .find(|fact| fact.precision == "year")
        .expect("spoken year must be a year fact");
    assert_eq!(fact.normalized.as_deref(), Some("2001"));
    assert!(!fact.surface.contains("gennaio"), "no invented month");
}

#[test]
fn iso_forms_with_dash_and_slash_delimiters() {
    let all = facts("Verified on 2020-02-29 by the archive.", "it");
    assert!(all
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2020-02-29")));
    let slash = facts("Signed 2020/11/30 in the contract.", "it");
    assert!(slash
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2020-11-30")));
}

#[test]
fn month_year_without_day() {
    let text = "L'indagine proseguì fino a giugno 1988.";
    let all = facts(text, "it");
    let fact = all
        .iter()
        .find(|fact| fact.precision == "month")
        .expect("month+year must be a month fact");
    assert_eq!(fact.normalized.as_deref(), Some("1988-06"));
    assert!(fact.surface.contains("giugno 1988"));
    assert_grounded(text, &all);
}

#[test]
fn era_years_are_classified() {
    let all = facts("Cadde nel 476 d.C. e nel 1453 d.C.", "it");
    // "d.C." is CE ("dopo Cristo"): no BCE era flag.
    assert!(
        all.iter()
            .filter(|fact| fact.surface.contains("1453"))
            .all(|fact| fact.era.is_none()),
        "d.C. years are CE: {:?}",
        all
    );
    let bc = facts("The archive dates to 44 BC.", "en");
    let fact = bc
        .iter()
        .find(|fact| fact.era == Some("BCE"))
        .expect("44 BC must be a BCE year fact");
    assert_eq!(fact.precision, "year");
}

#[test]
fn day_month_without_year_is_unresolved_no_iso_invented() {
    let all = facts("La consegna fu il 12 marzo.", "it");
    let fact = all
        .iter()
        .find(|fact| fact.surface.contains("12 marzo"))
        .expect("day+month form must still be reported");
    assert_eq!(fact.normalized, None, "no invented year");
    assert_eq!(fact.precision, "unresolved");
}

#[test]
fn portuguese_connector_form() {
    let text = "Nasceu em 12 de março de 2001.";
    let all = facts(text, "pt");
    assert_grounded(text, &all);
    assert!(all
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2001-03-12")));
}

#[test]
fn date_connectors_come_from_the_language_pack_not_a_table() {
    // "del" is a closed-class Italian word from the pack; the same text
    // under English must not treat it as a connector.
    let text = "Il fatto avvenne il 12 marzo del 2001.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    assert!(all
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2001-03-12")));
}

#[test]
fn months_resolve_without_any_spelled_day_table() {
    // Turkish and Russian have CLDR months but no curated spellout table:
    // the numeric day and the pack-only connector path must still work.
    for (language, text, expected) in [
        ("tr", "12 Mart 2001 tarihinde oldu.", "2001-03-12"),
        ("ru", "12 марта 2001 года.", "2001-03-12"),
        ("de", "Am 12. März 2001 geschah es.", "2001-03-12"),
    ] {
        let all = facts(text, language);
        assert_grounded(text, &all);
        assert!(
            all.iter()
                .any(|fact| fact.normalized.as_deref() == Some(expected)),
            "{language}: {text:?} did not resolve to {expected}: {all:?}"
        );
    }
}

#[test]
fn surfaces_come_from_the_original_not_the_folded_view() {
    let text = "Il 12 Marzo 2008 è la data ufficiale.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    assert!(all
        .iter()
        .any(|fact| fact.normalized.as_deref() == Some("2008-03-12")));
}

#[test]
fn date_clusters_from_two_to_seven_all_survive() {
    // The Fase 2 acceptance ladder: 2..=7 nearby dates in one text, all
    // independent of the visual top-N budget.
    let years = ["1998", "2001", "2005", "2010", "2019", "2050", "2088"];
    for count in 2..=7 {
        let mut text = String::from("Inizio: ");
        for (i, year) in years.iter().take(count).enumerate() {
            if i > 0 {
                text.push_str(", poi il ");
            }
            text.push_str(&format!("{} marzo", year));
            if count % 2 == 0 {
                text.push_str(" 2008");
            }
        }
        text.push('.');
        let all = facts(&text, "it");
        assert_grounded(&text, &all);
        let resolved: Vec<Vec<String>> = all
            .iter()
            .map(|fact| {
                fact.normalized
                    .as_deref()
                    .unwrap_or_default()
                    .split('-')
                    .map(|part| part.to_string())
                    .collect()
            })
            .collect();
        // The first `count` bare years must each survive as own facts.
        let year_values: Vec<&str> = all
            .iter()
            .filter_map(|fact| {
                let normalized = fact.normalized.as_deref().unwrap_or("");
                let head = normalized.split('-').next().unwrap_or("");
                (years.contains(&head)).then_some(head)
            })
            .collect();
        for year in &years[..count] {
            assert!(
                year_values.contains(year),
                "count={count}: {year} lost, facts: {all:?}"
            );
        }
        assert!(
            all.len() >= count,
            "count={count}: expected at least {count} facts, got {all:?} resolved: {resolved:?}"
        );
    }
}

#[test]
fn historic_years_normalize_without_era_confusion() {
    let text =
        "L'unificazione progredì nel 1848, la scoperta fu del 1492, la guerra finì nel 1943.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    let years: Vec<&str> = all
        .iter()
        .filter(|fact| fact.precision == "year")
        .filter_map(|fact| fact.normalized.as_deref())
        .collect();
    assert!(years.contains(&"1848"), "years: {years:?}");
    assert!(years.contains(&"1492"), "years: {years:?}");
    assert!(years.contains(&"1943"), "years: {years:?}");
    for fact in &all {
        assert!(fact.era.is_none(), "plain CE years carry no era: {fact:?}");
    }
}

#[test]
fn narration_order_follows_offsets_not_chronology() {
    // «Nel 2005, ripensando al 1998...»: 2005 is spoken first even though
    // it is chronologically after 1998. Facts must follow offset order.
    let text = "Nel 2005, ripensando al 1998, e infine al 2010.";
    let all = facts(text, "it");
    assert_grounded(text, &all);
    let surfaces: Vec<&str> = all.iter().map(|f| f.surface.as_str()).collect();
    assert!(
        surfaces.len() >= 3
            && text.find(surfaces[0]).unwrap_or(usize::MAX)
                < text.find(surfaces[1]).unwrap_or(usize::MAX)
            && text.find(surfaces[1]).unwrap_or(usize::MAX)
                < text.find(surfaces[2]).unwrap_or(usize::MAX),
        "narration order must equal source order; surfaces: {surfaces:?}"
    );
    let first = all
        .iter()
        .find(|f| f.normalized.as_deref() == Some("2005"))
        .expect("2005 must be a fact");
    let second = all
        .iter()
        .find(|f| f.normalized.as_deref() == Some("1998"))
        .expect("1998 must be a fact");
    assert!(
        first.start_byte < second.start_byte,
        "spoken 2005 must come before remembered 1998"
    );
    for fact in &all {
        assert_eq!(
            text.get(fact.start_byte..fact.end_byte),
            Some(fact.surface.as_str()),
            "byte offsets must point at the verbatim surface"
        );
    }
}

#[test]
fn facts_are_deterministic() {
    let text = "Primo il 12 marzo 2001, poi il 25 luglio 2005, infine l'8 febbraio 2019.";
    assert_eq!(facts(text, "it"), facts(text, "it"));
}
