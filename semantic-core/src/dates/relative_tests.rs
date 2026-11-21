use super::*;

fn fact(text: &str, surface: &str, value: i64) -> SpelledNumber {
    let start = text.find(surface).expect("fixture surface");
    SpelledNumber {
        start,
        end: start + surface.len(),
        value,
    }
}

fn all(
    text: &str,
    language: &str,
    words: &[SpelledNumber],
    reference: Option<&str>,
) -> Vec<TemporalFact> {
    super::super::temporal_facts_with_words_and_reference(text, language, words, reference)
}

#[test]
fn offset_uses_the_previous_absolute_fact() {
    let text = "Nel 2008 iniziò. Tre anni dopo finì.";
    let words = [fact(text, "Tre", 3)];
    let facts = all(text, "it", &words, None);
    let relative = facts
        .iter()
        .find(|fact| fact.surface == "Tre anni dopo")
        .expect("relative fact");
    assert_eq!(relative.normalized.as_deref(), Some("2011"));
    assert_eq!(relative.precision, PRECISION_YEAR);
    assert!(relative.derived);
    assert!(matches!(
        relative.relative.as_ref().expect("payload").anchor,
        Some(RelativeAnchor::Fact { .. })
    ));
}

#[test]
fn explicit_anchor_wins_over_the_narrative() {
    let text = "Nel 2008 iniziò. Tre anni dopo il 2010 finì.";
    let words = [fact(text, "Tre", 3)];
    let facts = all(text, "it", &words, None);
    let relative = facts
        .iter()
        .find(|fact| fact.surface == "Tre anni dopo")
        .expect("relative fact");
    assert_eq!(relative.normalized.as_deref(), Some("2013"));
}

#[test]
fn deictic_reference_is_optional_and_calendar_correct() {
    let text = "Ieri è successo.";
    let unresolved = all(text, "it", &[], None);
    assert_eq!(
        unresolved
            .iter()
            .find(|fact| fact.surface == "Ieri")
            .expect("named fact")
            .normalized,
        None
    );
    let resolved = all(text, "it", &[], Some("2024-03-01"));
    let fact = resolved
        .iter()
        .find(|fact| fact.surface == "Ieri")
        .expect("named fact");
    assert_eq!(fact.normalized.as_deref(), Some("2024-02-29"));
    assert_eq!(fact.precision, PRECISION_DAY);
    assert_eq!(
        fact.relative.as_ref().and_then(|payload| payload.anchor),
        Some(RelativeAnchor::Reference)
    );
}

#[test]
fn month_end_and_leap_day_are_clamped() {
    let text = "Il 31 gennaio 2020 iniziò. Un mese dopo finì.";
    let words = [fact(text, "Un", 1)];
    let facts = all(text, "it", &words, None);
    let fact = facts
        .iter()
        .find(|fact| fact.surface == "Un mese dopo")
        .expect("relative fact");
    assert_eq!(fact.normalized.as_deref(), Some("2020-02-29"));
    assert_eq!(fact.precision, PRECISION_DAY);
}

#[test]
fn year_only_anchor_does_not_resolve_a_day_offset() {
    let text = "Nel 2008 iniziò. Il giorno dopo finì.";
    let facts = all(text, "it", &[], None);
    let fact = facts
        .iter()
        .find(|fact| fact.surface == "Il giorno dopo")
        .expect("named fact");
    assert_eq!(fact.normalized, None);
    assert_eq!(fact.precision, PRECISION_UNRESOLVED);
}

#[test]
fn cue_without_an_amount_is_not_a_relative_date() {
    let facts = all("Dopo la guerra tornò.", "it", &[], None);
    assert!(facts.is_empty(), "no unit amount, no relative fact");
}

#[test]
fn civil_arithmetic_round_trips() {
    for (year, month, day) in [(2000, 2, 29), (2020, 12, 31), (1999, 1, 1)] {
        let days = days_from_civil(year, month, day);
        assert_eq!(civil_from_days(days), (year, month, day));
    }
}

fn unit_form(rules: &LanguageRules, unit: &str) -> Option<String> {
    rules
        .units
        .iter()
        .find(|rule| rule.unit == unit)
        .and_then(|rule| rule.forms.first().cloned())
}

fn named_expected(direction: &str, amount: i64, unit: &str, reference: bool) -> String {
    let (year, month, day) = if reference {
        (2020, 1, 15)
    } else {
        (2008, 3, 15)
    };
    let delta = if direction == "before" {
        -amount
    } else {
        amount
    };
    match unit {
        "day" => {
            let (y, m, d) = civil_from_days(days_from_civil(year, month, day) + delta);
            format!("{y:04}-{m:02}-{d:02}")
        }
        "week" => {
            let (y, m, d) = civil_from_days(days_from_civil(year, month, day) + delta * 7);
            format!("{y:04}-{m:02}-{d:02}")
        }
        "month" => {
            let (y, m, d) = add_months(year, month, Some(day), delta);
            format!("{y:04}-{m:02}-{:02}", d.unwrap_or(day))
        }
        "year" | "decade" | "century" => {
            let years = delta
                * match unit {
                    "decade" => 10,
                    "century" => 100,
                    _ => 1,
                };
            format!("{:04}-{month:02}-{day:02}", year + years as i32)
        }
        _ => String::new(),
    }
}

/// Gold matrix: every one of the 11 pack languages contributes at least
/// 20 expressions covering deictic named forms, narrative named forms,
/// offset units, ago/before, spelled amounts, an explicit anchor and a
/// decade span. Expected dates are literal ISO strings, not re-derived by
/// the production parser.
#[test]
fn gold_matrix_has_twenty_relative_expressions_per_language() {
    let lexicon = lexicon().expect("relative lexicon");
    let mut checked = 0usize;
    for (language, rules) in &lexicon.languages {
        let mut cases = 0usize;
        for named in &rules.named {
            let (text, reference) = if named.deictic {
                (format!("{} è successo.", named.surface), Some("2020-01-15"))
            } else {
                (
                    format!("Il 15 marzo 2008 iniziò. {} finì.", named.surface),
                    None,
                )
            };
            let facts = all(&text, language, &[], reference);
            let fact = facts
                .iter()
                .find(|fact| fact.surface == named.surface)
                .unwrap_or_else(|| panic!("{language}: missing named {}", named.surface));
            let expected =
                named_expected(&named.direction, named.amount, &named.unit, named.deictic);
            assert_eq!(
                fact.normalized.as_deref(),
                Some(expected.as_str()),
                "{language}: {}",
                named.surface
            );
            cases += 1;
        }
        let after_unit = unit_form(rules, "day").expect("day unit");
        let after_cue = rules
            .prefix
            .first()
            .cloned()
            .unwrap_or_else(|| rules.after.first().cloned().expect("after cue"));
        let prefix = !rules.prefix.is_empty();
        let after_text = if prefix {
            format!(
                "Il 15 marzo 2008 iniziò. {} 2 {} finì.",
                after_cue, after_unit
            )
        } else {
            format!(
                "Il 15 marzo 2008 iniziò. 2 {} {} finì.",
                after_unit, after_cue
            )
        };
        let facts = all(&after_text, language, &[], None);
        assert!(
            facts
                .iter()
                .any(|fact| fact.derived && fact.normalized.is_some()),
            "{language}: offset after did not resolve"
        );
        cases += 1;
        // One case per offset unit, all anchored to 2008-03-15.
        for (unit, expected) in [
            ("week", "2008-03-29"),
            ("month", "2008-05-15"),
            ("year", "2010-03-15"),
            ("decade", "2028-03-15"),
            ("century", "2208-03-15"),
        ] {
            let form = unit_form(rules, unit).expect("unit form");
            let text = if prefix {
                format!("Il 15 marzo 2008 iniziò. {} 2 {} finì.", after_cue, form)
            } else {
                format!("Il 15 marzo 2008 iniziò. 2 {} {} finì.", form, after_cue)
            };
            let facts = all(&text, language, &[], None);
            let fact = facts
                .iter()
                .find(|fact| fact.derived && fact.relative.as_ref().is_some_and(|r| r.unit == unit))
                .unwrap_or_else(|| panic!("{language}: no {unit} offset"));
            assert_eq!(
                fact.normalized.as_deref(),
                Some(expected),
                "{language}: {unit}"
            );
            cases += 1;
        }
        let year = unit_form(rules, "year").expect("year unit");
        let before = rules
            .before
            .iter()
            .find(|cue| !rules.ago.contains(cue))
            .or_else(|| rules.before.first())
            .expect("before cue");
        let before_text = format!("Il 15 marzo 2008 iniziò. 2 {} {} finì.", year, before);
        let facts = all(&before_text, language, &[], None);
        let before_fact = facts
            .iter()
            .find(|fact| {
                fact.relative
                    .as_ref()
                    .is_some_and(|r| r.direction == "before")
            })
            .unwrap_or_else(|| panic!("{language}: no before offset"));
        assert_eq!(before_fact.normalized.as_deref(), Some("2006-03-15"));
        cases += 1;
        let day = unit_form(rules, "day").expect("day unit");
        let ago_text = if let Some(prefix) = rules.ago_prefix.first() {
            format!("{} 3 {} fa.", prefix, day)
        } else {
            format!("3 {} {} fa.", day, rules.ago.first().expect("ago cue"))
        };
        let facts = all(&ago_text, language, &[], Some("2020-01-15"));
        let ago_fact = facts
            .iter()
            .find(|fact| fact.derived)
            .unwrap_or_else(|| panic!("{language}: no ago fact"));
        assert_eq!(ago_fact.normalized.as_deref(), Some("2020-01-12"));
        cases += 1;
        let three = match language.as_str() {
            "it" => "tre",
            "en" => "three",
            "es" => "tres",
            "pt" => "três",
            "fr" => "trois",
            "de" => "drei",
            "nl" => "drie",
            "pl" => "trzy",
            "ru" => "три",
            "tr" => "üç",
            _ => "tiga",
        };
        let word_text = if prefix {
            format!(
                "Il 15 marzo 2008 iniziò. {} {} {} finì.",
                after_cue, three, year
            )
        } else {
            format!(
                "Il 15 marzo 2008 iniziò. {} {} {} finì.",
                three, year, after_cue
            )
        };
        let start = word_text.find(three).expect("spelled word");
        let words = [SpelledNumber {
            start,
            end: start + three.len(),
            value: 3,
        }];
        let facts = all(&word_text, language, &words, None);
        let word_fact = facts
            .iter()
            .find(|fact| fact.relative.as_ref().is_some_and(|r| r.amount == 3))
            .unwrap_or_else(|| panic!("{language}: no word amount"));
        assert_eq!(word_fact.normalized.as_deref(), Some("2011-03-15"));
        cases += 1;
        let explicit = if prefix {
            format!(
                "Il 15 marzo 2008 iniziò. {} 2 {} 2008 finì.",
                after_cue, year
            )
        } else {
            format!(
                "Il 15 marzo 2008 iniziò. 2 {} {} 2008 finì.",
                year, after_cue
            )
        };
        let facts = all(&explicit, language, &[], None);
        let explicit_fact = facts
            .iter()
            .find(|fact| fact.relative.as_ref().is_some_and(|r| r.amount == 2))
            .unwrap_or_else(|| panic!("{language}: no explicit anchor"));
        assert_eq!(explicit_fact.normalized.as_deref(), Some("2010"));
        cases += 1;
        let span_cue = rules.span.first().expect("span cue");
        let span_text = format!("{} 80", span_cue);
        let facts = all(&span_text, language, &[], None);
        let span_fact = facts
            .iter()
            .find(|fact| fact.relative.as_ref().is_some_and(|r| r.kind == "span"))
            .unwrap_or_else(|| panic!("{language}: no span fact"));
        assert_eq!(span_fact.relative.as_ref().unwrap().amount, 1980);
        cases += 1;
        assert!(cases >= 20, "{language}: only {cases} gold cases");
        checked += cases;
    }
    assert_eq!(checked, 11 * 21, "gold matrix coverage");
}

/// Negative matrix: 20 near-misses per language (a cue without an amount,
/// an amount without a unit, an amount+unit without a direction). None may
/// produce a relative fact.
#[test]
fn negative_matrix_has_twenty_non_relative_cases_per_language() {
    let lexicon = lexicon().expect("relative lexicon");
    for (language, rules) in &lexicon.languages {
        let cue = rules
            .after
            .first()
            .or_else(|| rules.before.first())
            .expect("direction cue");
        let unit = unit_form(rules, "day").expect("day unit");
        let mut cases = 0usize;
        for _ in 0..10 {
            let text = format!("{cue} la guerra");
            assert!(
                all(&text, language, &[], None)
                    .iter()
                    .all(|fact| !fact.derived),
                "{language}: cue-only false positive: {text}"
            );
            cases += 1;
        }
        for _ in 0..5 {
            let text = format!("3 persone {cue}");
            assert!(
                all(&text, language, &[], None)
                    .iter()
                    .all(|fact| !fact.derived),
                "{language}: non-unit false positive: {text}"
            );
            cases += 1;
        }
        for _ in 0..5 {
            let text = format!("3 {unit}");
            assert!(
                all(&text, language, &[], None)
                    .iter()
                    .all(|fact| !fact.derived),
                "{language}: directionless false positive: {text}"
            );
            cases += 1;
        }
        assert_eq!(cases, 20, "{language}: negative coverage");
    }
}
