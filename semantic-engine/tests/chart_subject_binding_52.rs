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
