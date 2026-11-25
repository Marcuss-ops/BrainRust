//! Contextual numeric series detection regression corpus.
//!
//! The cases mirror the review ticket: ordered/temporal series, unit-independent
//! comparisons, ASR/spoken-number robustness, and false-positive guards. Every
//! chart item is checked against the source slice so no value or label can be
//! invented.

mod common;

use common::*;
use serde_json::Value;

fn single_chart(language: &str, text: &str) -> (Value, Vec<f64>, Vec<String>) {
    let manifest = manifest_value_for(language, vec![scene("contextual", text)]);
    let scene = &manifest["scenes"][0];
    let charts = proposals_of(scene, "chart");
    assert_eq!(
        charts.len(),
        1,
        "expected exactly one chart for {text:?}; scene={scene}"
    );
    let chart = charts[0].clone();
    let items = chart["chart_items"].as_array().expect("chart items");
    let values: Vec<f64> = items
        .iter()
        .filter_map(|item| item["value"].as_f64())
        .collect();
    let labels: Vec<String> = items
        .iter()
        .filter_map(|item| item["label"].as_str().map(str::to_string))
        .collect();
    for item in items {
        let start = item["start_byte"].as_u64().unwrap_or_default() as usize;
        let end = item["end_byte"].as_u64().unwrap_or_default() as usize;
        assert_eq!(
            text.get(start..end),
            item["text"].as_str(),
            "chart item must be a byte-exact source slice: {item}"
        );
        let label = item["label"].as_str().unwrap_or_default();
        assert!(
            !label.is_empty() && label != text,
            "chart label must be grounded and specific: {item}"
        );
    }
    let start = chart["start_byte"].as_u64().unwrap_or_default() as usize;
    let end = chart["end_byte"].as_u64().unwrap_or_default() as usize;
    assert_eq!(
        text.get(start..end),
        chart["evidence"].as_str(),
        "chart evidence must be a byte-exact source slice"
    );
    (chart, values, labels)
}

#[test]
fn a_temporal_and_ordered_series_are_unit_independent() {
    let cases: &[(&str, &str, &[f64])] = &[
        (
            "A01",
            "Nel 2015 erano 1500, nel 2016 1200, nel 2018 1800.",
            &[1500.0, 1200.0, 1800.0],
        ),
        (
            "A02",
            "A gennaio 100, a febbraio 200, a marzo 350.",
            &[100.0, 200.0, 350.0],
        ),
        (
            "A03",
            "Il primo giorno 20, il secondo 40, il terzo 90.",
            &[20.0, 40.0, 90.0],
        ),
        (
            "A04",
            "Prima erano 1500, poi 1800, infine 2100.",
            &[1500.0, 1800.0, 2100.0],
        ),
        (
            "A05",
            "Siamo passati da 100 a 500 e poi a 900.",
            &[100.0, 500.0, 900.0],
        ),
        (
            "A07",
            "Partiva da 1000, è sceso a 800 ed è risalito a 1200.",
            &[1000.0, 800.0, 1200.0],
        ),
        (
            "A08",
            "Nel 2020 1500 nel 2021 1800 nel 2022 1900",
            &[1500.0, 1800.0, 1900.0],
        ),
    ];
    for (id, text, expected) in cases {
        let (_, values, _) = single_chart("it", text);
        assert_eq!(values, *expected, "{id}: {text}");
    }

    let (_, values, labels) =
        single_chart("it", "Nel 2015 erano 1500, nel 2016 1200, nel 2018 1800.");
    assert_eq!(values, [1500.0, 1200.0, 1800.0]);
    for year in ["2015", "2016", "2018"] {
        assert!(
            labels.iter().any(|label| label.contains(year)),
            "missing year label {year}: {labels:?}"
        );
    }
}

#[test]
fn a_temporal_series_crosses_sentences_without_a_metric_noun() {
    let (chart, values, labels) = single_chart(
        "it",
        "Nel 2020 erano 5000. Nel 2021 erano 8000. Nel 2022 erano 3000.",
    );
    assert_eq!(values, [5000.0, 8000.0, 3000.0], "A06");
    assert_eq!(chart["suggested_visual"], "timeline_growth");
    assert_eq!(chart["chart_meta"]["series_type"], "time");
    for year in ["2020", "2021", "2022"] {
        assert!(
            labels.iter().any(|label| label.contains(year)),
            "missing year label {year}: {labels:?}"
        );
    }
}

#[test]
fn b_subject_comparisons_and_distributions_are_detected() {
    let cases: &[(&str, &str, &[f64])] = &[
        (
            "B01",
            "Mario 1500, Luca 1800, Marco 1200.",
            &[1500.0, 1800.0, 1200.0],
        ),
        (
            "B02",
            "L'Italia ne ha 1500, la Francia 1800 e la Germania 2100.",
            &[1500.0, 1800.0, 2100.0],
        ),
        (
            "B03",
            "Il primo ne ha fatti 80, il secondo 120 e il terzo 90.",
            &[80.0, 120.0, 90.0],
        ),
        (
            "B04",
            "Prima ne faceva 1500, adesso 1800.",
            &[1500.0, 1800.0],
        ),
        (
            "B06",
            "A Milano 1000, a Roma 1500, a Napoli 1200.",
            &[1000.0, 1500.0, 1200.0],
        ),
    ];
    for (id, text, expected) in cases {
        let (_, values, _) = single_chart("it", text);
        assert_eq!(values, *expected, "{id}: {text}");
    }

    let (_, values, labels) = single_chart("it", "Mario 1500, Luca 1800, Marco 1200.");
    assert_eq!(values, [1500.0, 1800.0, 1200.0]);
    for name in ["Mario", "Luca", "Marco"] {
        assert!(
            labels.iter().any(|label| label.contains(name)),
            "missing subject label {name}: {labels:?}"
        );
    }

    let (chart, values, _) = single_chart("it", "Il 60% preferisce A, il 30% B, il 10% C.");
    assert_eq!(values, [60.0, 30.0, 10.0], "B05");
    assert_eq!(chart["suggested_visual"], "segmented_total_bar");
}

#[test]
fn c_asr_spoken_scaled_and_signed_values_are_preserved() {
    let cases: &[(&str, &str, &[f64])] = &[
        (
            "C01",
            "Rick ross nel 2015 ha venuto 1500 dischi nel 2016 1200 nel 2018 1800",
            &[1500.0, 1200.0, 1800.0],
        ),
        (
            "C02",
            "Erano millecinquecento, poi duemila, poi tremila.",
            &[1500.0, 2000.0, 3000.0],
        ),
        (
            "C03",
            "Nel 2015 1.500, nel 2016 1200, nel 2018 1.800.",
            &[1500.0, 1200.0, 1800.0],
        ),
        (
            "C06",
            "Nel 2015 erano 1,5 milioni, nel 2016 2 milioni.",
            &[1_500_000.0, 2_000_000.0],
        ),
        (
            "C07",
            "Prima meno 20, poi zero, poi più 30.",
            &[-20.0, 0.0, 30.0],
        ),
    ];
    for (id, text, expected) in cases {
        let (_, values, _) = single_chart("it", text);
        assert_eq!(values, *expected, "{id}: {text}");
    }

    // C04: approximation cues stay attached to the grounded value.
    let (chart, values, _) = single_chart("it", "Nel 2015 circa 1500, nel 2016 più o meno 1200.");
    assert_eq!(values, [1500.0, 1200.0]);
    let items = chart["chart_items"].as_array().expect("items");
    assert!(
        items.iter().all(|item| item["qualifier"] == "approximate"),
        "approximations must be preserved: {items:?}"
    );

    // C05: a self-correction may stay unplotted, but it can never invent a
    // value. If the engine resolves it, every point is one of the spoken ones.
    let text = "Erano tipo 100 no forse 120 e poi sono arrivati a 200.";
    let manifest = manifest_value_for("it", vec![scene("C05", text)]);
    for chart in proposals_of(&manifest["scenes"][0], "chart") {
        let values: Vec<f64> = chart["chart_items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["value"].as_f64())
            .collect();
        assert!(
            values
                .iter()
                .all(|value| [100.0, 120.0, 200.0].contains(value)),
            "C05 invented a value: {values:?}"
        );
    }
}

#[test]
fn d_false_positive_guards_hold() {
    let cases = [
        (
            "D01",
            "È nato nel 1980, si è sposato nel 2010 ed è morto nel 2022.",
        ),
        ("D02", "Ha 30 anni, pesa 80 chili ed è alto 180 centimetri."),
        ("D03", "Nel 2015 ha comprato un'auto, nel 2016 una casa."),
        ("D04", "Il codice è 1500, la password è 1800."),
        ("D05", "Non è vero che ha venduto 1500 o 2000 dischi."),
        (
            "D06",
            "Nel 2020 ha guadagnato 1500 euro e venduto 300 dischi.",
        ),
        (
            "D07",
            "Nel 2020 ha venduto 1500 copie, spera di venderne 3000 nel 2027.",
        ),
        ("D08", "Nel 2015, nel 2016 e nel 2018 è stato in Italia."),
    ];
    for (id, text) in cases {
        let manifest = manifest_value_for("it", vec![scene(id, text)]);
        let charts = proposals_of(&manifest["scenes"][0], "chart");
        assert!(
            charts.is_empty(),
            "{id} must not produce a quantitative chart: {text}\n{charts:?}"
        );
    }
}

#[test]
fn unformatted_four_digit_quantities_are_not_eaten_by_year_detection() {
    // The original bug: 1500/1200/1800 were typed as years because they are
    // four digits in 1000..=2099, leaving zero numeric facts to chart.
    let cases = [
        (
            "it",
            "Nel 2015 Aurora ha venduto 1500 pezzi, nel 2016 1200, nel 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "en",
            "In 2015 Aurora sold 1500 units, in 2016 1200, in 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "de",
            "Im Jahr 2015 verkaufte Aurora 1500 Stück, im Jahr 2016 1200, im Jahr 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "es",
            "En 2015 Aurora vendió 1500 unidades, en 2016 1200, en 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "fr",
            "En 2015, Aurora a vendu 1500 unités, en 2016 1200, en 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "nl",
            "In 2015 verkocht Aurora 1500 stuks, in 2016 1200, in 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "pl",
            "W 2015 roku Aurora sprzedała 1500 sztuk, w 2016 roku 1200, w 2018 roku 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "ru",
            "В 2015 году Aurora продала 1500 штук, в 2016 году 1200, в 2018 году 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "pt",
            "Em 2015 a Aurora vendeu 1500 unidades, em 2016 1200, em 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "id",
            "Pada 2015 Aurora menjual 1500 unit, pada 2016 1200, pada 2018 1800.",
            ["2015", "2016", "2018"],
        ),
        (
            "tr",
            "2015'te Aurora 1500 adet sattı, 2016'da 1200, 2018'de 1800.",
            ["2015", "2016", "2018"],
        ),
    ];
    let mut failures = Vec::new();
    for (language, text, years) in cases {
        let manifest = manifest_value_for(language, vec![scene("four-digit", text)]);
        let scene = &manifest["scenes"][0];
        let charts = proposals_of(scene, "chart");
        if charts.len() != 1 {
            failures.push(format!(
                "{language}: expected one chart, found {}; numeric_facts={} scene={scene}",
                charts.len(),
                scene["numeric_facts"]
            ));
            continue;
        }
        let items = charts[0]["chart_items"].as_array().expect("items");
        let values: Vec<f64> = items
            .iter()
            .filter_map(|item| item["value"].as_f64())
            .collect();
        if values != [1500.0, 1200.0, 1800.0] {
            failures.push(format!(
                "{language}: expected [1500,1200,1800], got {values:?}"
            ));
        }
        let labels: Vec<&str> = items
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect();
        for year in years {
            if !labels.iter().any(|label| label.contains(year)) {
                failures.push(format!("{language}: missing label {year}: {labels:?}"));
            }
        }
        for item in items {
            let start = item["start_byte"].as_u64().unwrap_or_default() as usize;
            let end = item["end_byte"].as_u64().unwrap_or_default() as usize;
            if text.get(start..end) != item["text"].as_str() {
                failures.push(format!("{language}: item is not a source slice: {item}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn ticket_51_single_and_multi_sentence_disco_series() {
    for text in [
        "Rick Ross nel 2015 ha venduto 1500 dischi, nel 2016 1200, nel 2018 1800.",
        "Nel 2015 Rick Ross ha venduto 1500 dischi. Nel 2016 ne ha venduti 1200. Nel 2018 ne ha venduti 1800.",
    ] {
        let manifest = manifest_value_for("it", vec![scene("dischi", text)]);
        let scene = &manifest["scenes"][0];
        let numeric: Vec<&str> = scene["numeric_facts"]
            .as_array()
            .expect("numeric facts")
            .iter()
            .filter_map(|fact| fact["surface"].as_str())
            .collect();
        assert_eq!(numeric, ["1500", "1200", "1800"], "{text}");
        let charts = proposals_of(scene, "chart");
        assert_eq!(charts.len(), 1, "{text}: {scene}");
        assert_eq!(charts[0]["suggested_visual"], "timeline_growth");
        assert_eq!(charts[0]["chart_meta"]["series_type"], "time");
        let items = charts[0]["chart_items"].as_array().expect("items");
        let values: Vec<f64> = items
            .iter()
            .filter_map(|item| item["value"].as_f64())
            .collect();
        assert_eq!(values, [1500.0, 1200.0, 1800.0], "{text}");
        let labels: Vec<&str> = items
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect();
        for year in ["2015", "2016", "2018"] {
            assert!(
                labels.iter().any(|label| label.contains(year)),
                "{text}: missing {year}: {labels:?}"
            );
        }
    }
}

#[test]
fn ticket_51_ranges_and_historical_years_stay_dates() {
    for text in [
        "Dal 2015 al 2018 non ci furono modifiche.",
        "Tra il 1998 e il 2004 ci furono modifiche.",
        "Tra 1998 e 2004 ci furono modifiche.",
        "Nel 1500 Leonardo arrivò a Venezia.",
    ] {
        let manifest = manifest_value_for("it", vec![scene("year-guard", text)]);
        let scene = &manifest["scenes"][0];
        assert!(
            scene["numeric_facts"].as_array().unwrap().is_empty(),
            "{text}: years must stay dates: {}",
            scene["numeric_facts"]
        );
        assert!(
            proposals_of(scene, "chart").is_empty(),
            "{text}: date ranges must not become charts: {}",
            scene["visual_proposals"]
        );
    }
}
