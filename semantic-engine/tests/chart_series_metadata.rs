//! TICKET-41/42 acceptance: N-point series, chart metadata and dynamic layouts.

mod common;

use common::*;

const SERIES: &[(&str, &str)] = &[
    (
        "de",
        "2021 verkaufte Aurora 120 Autos. 2022 verkaufte Aurora 180 Autos. 2023 verkaufte Aurora 210 Autos. 2024 verkaufte Aurora 240 Autos.",
    ),
    (
        "en",
        "In 2021 Aurora sold 120 cars. In 2022 Aurora sold 180 cars. In 2023 Aurora sold 210 cars. In 2024 Aurora sold 240 cars.",
    ),
    (
        "es",
        "En 2021 Aurora vendió 120 coches. En 2022 Aurora vendió 180 coches. En 2023 Aurora vendió 210 coches. En 2024 Aurora vendió 240 coches.",
    ),
    (
        "fr",
        "En 2021, Aurora a vendu 120 voitures. En 2022, Aurora a vendu 180 voitures. En 2023, Aurora a vendu 210 voitures. En 2024, Aurora a vendu 240 voitures.",
    ),
    (
        "id",
        "Pada 2021 Aurora menjual 120 mobil. Pada 2022 Aurora menjual 180 mobil. Pada 2023 Aurora menjual 210 mobil. Pada 2024 Aurora menjual 240 mobil.",
    ),
    (
        "it",
        "Nel 2021 Aurora ha venduto 120 auto. Nel 2022 Aurora ha venduto 180 auto. Nel 2023 Aurora ha venduto 210 auto. Nel 2024 Aurora ha venduto 240 auto.",
    ),
    (
        "nl",
        "In 2021 verkocht Aurora 120 auto's. In 2022 verkocht Aurora 180 auto's. In 2023 verkocht Aurora 210 auto's. In 2024 verkocht Aurora 240 auto's.",
    ),
    (
        "pl",
        "W 2021 roku Aurora sprzedała 120 samochodów. W 2022 roku Aurora sprzedała 180 samochodów. W 2023 roku Aurora sprzedała 210 samochodów. W 2024 roku Aurora sprzedała 240 samochodów.",
    ),
    (
        "pt",
        "Em 2021, Aurora vendeu 120 carros. Em 2022, Aurora vendeu 180 carros. Em 2023, Aurora vendeu 210 carros. Em 2024, Aurora vendeu 240 carros.",
    ),
    (
        "ru",
        "В 2021 году Aurora продала 120 автомобилей. В 2022 году Aurora продала 180 автомобилей. В 2023 году Aurora продала 210 автомобилей. В 2024 году Aurora продала 240 автомобилей.",
    ),
    (
        "tr",
        "2021'de Aurora 120 otomobil sattı. 2022'de Aurora 180 otomobil sattı. 2023'te Aurora 210 otomobil sattı. 2024'te Aurora 240 otomobil sattı.",
    ),
];

#[test]
fn a_four_sentence_annual_series_is_one_n_point_line_series_in_eleven_languages() {
    let mut failures = Vec::new();
    for (language, text) in SERIES {
        let manifest = manifest_value_for(language, vec![scene("series", text)]);
        let charts = proposals_of(&manifest["scenes"][0], "chart");
        if charts.len() != 1 {
            failures.push(format!(
                "{language}: expected exactly one series chart, found {}; {}",
                charts.len(),
                manifest["scenes"][0]["visual_proposals"]
            ));
            continue;
        }
        let chart = charts[0];
        let items = chart["chart_items"].as_array().expect("chart items");
        let values: Vec<f64> = items
            .iter()
            .filter_map(|item| item["value"].as_f64())
            .collect();
        if values != [120.0, 180.0, 210.0, 240.0] {
            failures.push(format!(
                "{language}: expected 4 ordered values, found {values:?}"
            ));
        }
        if chart["suggested_visual"] != "timeline_growth" {
            failures.push(format!(
                "{language}: expected timeline_growth, found {}",
                chart["suggested_visual"]
            ));
        }
        if chart["chart_meta"]["series_type"] != "time" {
            failures.push(format!(
                "{language}: expected time series metadata, found {}",
                chart["chart_meta"]
            ));
        }
        let labels: Vec<&str> = items
            .iter()
            .filter_map(|item| item["label"].as_str())
            .collect();
        for year in ["2021", "2022", "2023", "2024"] {
            if !labels.iter().any(|label| label.contains(year)) {
                failures.push(format!("{language}: missing label for {year}: {labels:?}"));
            }
        }
        for item in items {
            let start = item["start_byte"].as_u64().unwrap_or_default() as usize;
            let end = item["end_byte"].as_u64().unwrap_or_default() as usize;
            if text.get(start..end) != item["text"].as_str() {
                failures.push(format!(
                    "{language}: item is not an exact source slice: {item}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_four_point_series_stops_at_the_six_point_cap() {
    let text = "In 2019 Aurora sold 10 cars. In 2020 Aurora sold 20 cars. In 2021 Aurora sold 30 cars. In 2022 Aurora sold 40 cars. In 2023 Aurora sold 50 cars. In 2024 Aurora sold 60 cars. In 2025 Aurora sold 70 cars.";
    let manifest = manifest_value_for("en", vec![scene("cap", text)]);
    let charts = proposals_of(&manifest["scenes"][0], "chart");
    assert_eq!(charts.len(), 1, "{charts:?}");
    let items = charts[0]["chart_items"].as_array().expect("items");
    assert_eq!(items.len(), 6, "{items:?}");
}

#[test]
fn chart_metadata_exposes_grounded_metric_unit_dimension_and_period() {
    let text = "Il fatturato è stato di 2 milioni di euro nel 2021. Il fatturato è stato di 3 milioni di euro nel 2022. Il fatturato è stato di 4 milioni di euro nel 2023.";
    let manifest = manifest_value_for("it", vec![scene("meta", text)]);
    let charts = proposals_of(&manifest["scenes"][0], "chart");
    assert_eq!(charts.len(), 1, "{charts:?}");
    let chart = charts[0];
    let meta = &chart["chart_meta"];
    assert_eq!(meta["metric"], "revenue", "{meta}");
    assert_eq!(meta["unit"], "EUR", "{meta}");
    assert_eq!(meta["dimension"], "currency", "{meta}");
    assert_eq!(meta["series_type"], "time", "{meta}");
    assert_eq!(chart["suggested_visual"], "timeline_growth");
    let items = chart["chart_items"].as_array().expect("items");
    assert_eq!(items[0]["period"], "2021", "{items:?}");
    assert_eq!(items[1]["period"], "2022", "{items:?}");
    assert!(
        items.iter().all(|item| item["qualifier"] == "exact"),
        "{items:?}"
    );
    let production = manifest["visual_consolidation"]["production_candidates"]
        .as_array()
        .expect("production candidates")
        .iter()
        .find(|candidate| candidate["recipe"] == "chart")
        .expect("chart production candidate");
    assert_eq!(&production["chart_meta"], meta);
}

#[test]
fn layouts_distinguish_share_from_comparison() {
    let donut_text = "Aurora holds 42% of the market share, Lumen 33% and Nova 20%.";
    let donut = manifest_value_for("en", vec![scene("donut", donut_text)]);
    let charts = proposals_of(&donut["scenes"][0], "chart");
    assert!(!charts.is_empty(), "{}", donut["scenes"][0]);
    assert_eq!(charts[0]["suggested_visual"], "segmented_total_bar");

    let bar_text = "Aurora recorded 42% compared with 18% for Lumen.";
    let bar = manifest_value_for("en", vec![scene("bar", bar_text)]);
    let charts = proposals_of(&bar["scenes"][0], "chart");
    assert!(!charts.is_empty(), "{}", bar["scenes"][0]);
    assert_eq!(charts[0]["suggested_visual"], "bar_comparison");

    let scenario_text = "Aurora recorded 42% compared with 18% for Lumen in the forecast.";
    let scenario = manifest_value_for("en", vec![scene("scenario", scenario_text)]);
    let charts = proposals_of(&scenario["scenes"][0], "chart");
    assert!(!charts.is_empty(), "{}", scenario["scenes"][0]);
    assert_eq!(charts[0]["suggested_visual"], "scenario_split");
}

#[test]
fn grounded_change_comparison_emits_a_before_after_delta_proposal() {
    let text = "Il fatturato è al 10% ed è salito al 12% nel 2023.";
    let manifest = manifest_value_for("it", vec![scene("delta", text)]);
    let scene = &manifest["scenes"][0];
    let deltas = proposals_of(scene, "delta_comparison");
    assert_eq!(deltas.len(), 1, "{}", scene["quantity_comparisons"]);
    let delta = deltas[0];
    assert_eq!(delta["suggested_visual"], "before_after_metric");
    let items = delta["chart_items"].as_array().expect("delta items");
    assert_eq!(items.len(), 2, "{items:?}");
    let start = delta["start_byte"].as_u64().unwrap() as usize;
    let end = delta["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(start..end), delta["evidence"].as_str());
    for item in items {
        let start = item["start_byte"].as_u64().unwrap() as usize;
        let end = item["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), item["text"].as_str());
    }
    assert!(manifest["visual_consolidation"]["production_candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .any(|candidate| candidate["recipe"] == "delta_comparison"));
}

#[test]
fn incompatible_units_emit_no_delta_proposal() {
    let text = "Il fatturato è a 10 dollari ed è salito al 12% nel 2023.";
    let manifest = manifest_value_for("it", vec![scene("delta-bad", text)]);
    assert!(proposals_of(&manifest["scenes"][0], "delta_comparison").is_empty());
}

#[test]
fn ranked_quantities_emit_a_ranking_bar_without_a_chart_proposal() {
    let text = "Top 10 revenues:\n\
                1. France earned 10 EUR.\n\
                2. Italy earned 9 EUR.\n\
                3. Spain earned 8 EUR.\n\
                4. Poland earned 7 EUR.\n\
                5. Portugal earned 6 EUR.\n\
                6. Greece earned 5 EUR.\n\
                7. Austria earned 4 EUR.\n\
                8. Belgium earned 3 EUR.\n\
                9. Sweden earned 2 EUR.\n\
                10. Norway earned 1 EUR.";
    let manifest = manifest_value_for("en", vec![scene("ranking-bar", text)]);
    let scene = &manifest["scenes"][0];
    assert!(
        proposals_of(scene, "chart").is_empty(),
        "rank indices must not become chart values: {}",
        scene["visual_proposals"]
    );
    let ranking = manifest["visual_consolidation"]["production_candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .find(|candidate| candidate["recipe"] == "ranking_bar")
        .unwrap_or_else(|| panic!("missing ranking_bar: {}", manifest["visual_consolidation"]));
    assert_eq!(ranking["suggested_visual"], "ranking_bar");
    let items = ranking["chart_items"].as_array().expect("items");
    assert_eq!(items.len(), 10, "{items:?}");
    let values: Vec<f64> = items
        .iter()
        .filter_map(|item| item["value"].as_f64())
        .collect();
    assert_eq!(values, [10.0, 9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0]);
    assert_eq!(ranking["chart_meta"]["unit"], "EUR");
    assert_eq!(ranking["chart_meta"]["dimension"], "currency");
    let start = ranking["start_byte"].as_u64().unwrap() as usize;
    let end = ranking["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(start..end), ranking["evidence"].as_str());
}
