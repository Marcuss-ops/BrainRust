//! TICKET-52 item 3: a cross-sentence series is one subject's metric.

mod common;

use common::{manifest_value_for, scene};
use serde_json::Value;

fn charts(language: &str, text: &str) -> Vec<Value> {
    let manifest = manifest_value_for(language, vec![scene("charts", text)]);
    manifest["scenes"][0]["visual_proposals"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|proposal| proposal["proposal_type"] == "chart")
        .collect()
}

#[test]
fn two_named_subjects_never_form_one_time_series() {
    let source =
        "Rick Ross nel 2011 ha venduto 25.000 copie. Eminem nel 2014 ne ha vendute 34.999.";
    let charts = charts("it", source);
    assert!(
        charts.is_empty(),
        "different subjects must not become one series: {charts:?}"
    );
}

#[test]
fn one_subject_with_an_elided_second_sentence_still_forms_a_series() {
    let source = "Nel 2015 Rick Ross ha venduto 1500 dischi. Nel 2016 ne ha venduti 1200.";
    let charts = charts("it", source);
    assert_eq!(charts.len(), 1, "{charts:?}");
    let items = charts[0]["chart_items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{charts:?}");
    assert_eq!(items[0]["value"], 1500.0);
    assert_eq!(items[1]["value"], 1200.0);
}

/// One subject across two sentences, in every supported language. The subject
/// is named once and then elided or pronominal, which must continue the same
/// series.
const CONTINUED_SERIES: &[(&str, &str)] = &[
    (
        "it",
        "Nel 2015 Rick Ross ha venduto 1500 dischi. Nel 2016 ne ha venduti 1200.",
    ),
    (
        "en",
        "In 2015 Rick Ross sold 1500 records. In 2016 he sold 1200.",
    ),
    (
        "es",
        "En 2015 Rick Ross vendió 1500 discos. En 2016 vendió 1200.",
    ),
    (
        "pt",
        "Em 2015 Rick Ross vendeu 1500 discos. Em 2016 vendeu 1200.",
    ),
    (
        "fr",
        "En 2015 Rick Ross a vendu 1500 disques. En 2016 il en a vendu 1200.",
    ),
    (
        "de",
        "Im Jahr 2015 verkaufte Rick Ross 1500 Platten. Im Jahr 2016 verkaufte er 1200.",
    ),
    (
        "nl",
        "In 2015 verkocht Rick Ross 1500 platen. In 2016 verkocht hij 1200.",
    ),
    (
        "pl",
        "W 2015 roku Rick Ross sprzedał 1500 płyt. W 2016 roku sprzedał 1200.",
    ),
    (
        "ru",
        "В 2015 году Рик Росс продал 1500 пластинок. В 2016 году он продал 1200.",
    ),
    (
        "tr",
        "2015 yılında Rick Ross 1500 plak sattı. 2016 yılında 1200 sattı.",
    ),
    (
        "id",
        "Pada 2015 Rick Ross menjual 1500 keping. Pada 2016 menjual 1200.",
    ),
];

/// Two different named subjects across the same year/value shape. No series
/// may merge them, in any language.
const CROSS_SUBJECT_SERIES: &[(&str, &str)] = &[
    (
        "it",
        "Nel 2011 Rick Ross ha venduto 25000 copie. Nel 2014 Eminem ne ha vendute 34999.",
    ),
    (
        "en",
        "In 2011 Rick Ross sold 25000 copies. In 2014 Eminem sold 34999.",
    ),
    (
        "es",
        "En 2011 Rick Ross vendió 25000 copias. En 2014 Eminem vendió 34999.",
    ),
    (
        "pt",
        "Em 2011 Rick Ross vendeu 25000 cópias. Em 2014 Eminem vendeu 34999.",
    ),
    (
        "fr",
        "En 2011 Rick Ross a vendu 25000 copies. En 2014 Eminem a vendu 34999.",
    ),
    (
        "de",
        "Im Jahr 2011 verkaufte Rick Ross 25000 Exemplare. Im Jahr 2014 verkaufte Eminem 34999.",
    ),
    (
        "nl",
        "In 2011 verkocht Rick Ross 25000 exemplaren. In 2014 verkocht Eminem 34999.",
    ),
    (
        "pl",
        "W 2011 roku Rick Ross sprzedał 25000 kopii. W 2014 roku Eminem sprzedał 34999.",
    ),
    (
        "ru",
        "В 2011 году Рик Росс продал 25000 копий. В 2014 году Эминем продал 34999.",
    ),
    (
        "tr",
        "2011 yılında Rick Ross 25000 kopya sattı. 2014 yılında Eminem 34999 sattı.",
    ),
    (
        "id",
        "Pada 2011 Rick Ross menjual 25000 kopi. Pada 2014 Eminem menjual 34999.",
    ),
];

#[test]
fn one_subject_forms_the_same_series_in_every_supported_language() {
    for (language, source) in CONTINUED_SERIES {
        let charts = charts(language, source);
        assert_eq!(charts.len(), 1, "{language}: {charts:?}");
        let items = charts[0]["chart_items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "{language}: {charts:?}");
        assert_eq!(items[0]["value"], 1500.0, "{language}");
        assert_eq!(items[1]["value"], 1200.0, "{language}");
    }
}

#[test]
fn a_different_subject_never_joins_the_series_in_any_supported_language() {
    for (language, source) in CROSS_SUBJECT_SERIES {
        let charts = charts(language, source);
        assert!(
            charts.is_empty(),
            "{language}: different subjects must not become one series: {charts:?}"
        );
    }
}

#[test]
fn an_ambiguous_sentence_breaks_the_chain_instead_of_guessing() {
    let source =
        "Nel 2015 Rick Ross ha venduto 1500 dischi. Nel 2016 Rick Ross ed Eminem ne hanno venduti 1200.";
    let charts = charts("it", source);
    assert!(
        charts.is_empty(),
        "an ambiguous second subject must not inherit the first: {charts:?}"
    );
}
