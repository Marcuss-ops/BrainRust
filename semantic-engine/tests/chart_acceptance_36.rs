//! Acceptance corpus for the 36 chart cases discussed in the BrainRust review.
//! Case IDs stay in test metadata; they are deliberately not prepended to the
//! source text passed to the analyzer.

mod common;

use common::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Case {
    id: u8,
    text: String,
    expected: String,
    expected_values: Vec<f64>,
}

#[test]
fn chart_decisions_match_the_36_case_acceptance_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("data/chart_acceptance_36.json"))
        .expect("chart acceptance corpus is valid JSON");
    assert_eq!(cases.len(), 36);
    let mut mismatches = Vec::new();

    for case in cases {
        let manifest = manifest_value(vec![scene(&format!("chart-{:02}", case.id), &case.text)]);
        let scene = &manifest["scenes"][0];
        let charts = proposals_of(scene, "chart");
        let found = if charts.is_empty() { "none" } else { "chart" };
        let expected_chart = case.expected != "none";
        let found_chart = !charts.is_empty();
        if expected_chart != found_chart {
            mismatches.push(format!(
                "case {:02}: expected {}, found {}; numeric_facts={} bullets={}",
                case.id, case.expected, found, scene["numeric_facts"], scene["bullets"]
            ));
            continue;
        }

        let mut actual_values: Vec<f64> = charts
            .iter()
            .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
            .filter_map(|item| item["value"].as_f64())
            .collect();
        let mut expected_values = case.expected_values.clone();
        actual_values.sort_by(f64::total_cmp);
        expected_values.sort_by(f64::total_cmp);
        if actual_values != expected_values {
            mismatches.push(format!(
                "case {:02}: expected values {:?}, found {:?}",
                case.id, expected_values, actual_values
            ));
        }
        for chart in &charts {
            for item in chart["chart_items"].as_array().into_iter().flatten() {
                let start = item["start_byte"].as_u64().unwrap_or_default() as usize;
                let end = item["end_byte"].as_u64().unwrap_or_default() as usize;
                if case.text.get(start..end) != item["text"].as_str() {
                    mismatches.push(format!(
                        "case {:02}: chart item is not an exact source slice: {item}",
                        case.id
                    ));
                }
            }
        }

        if case.expected == "conditional" {
            // Conditional values are allowed only while the complete source
            // claim (including its qualifiers) remains attached as evidence.
            if charts.iter().any(|chart| chart["evidence"] != case.text) {
                mismatches.push(format!(
                    "case {:02}: conditional qualifiers are absent from chart evidence",
                    case.id
                ));
            }
        }
    }

    assert!(
        mismatches.is_empty(),
        "chart acceptance mismatches:\n{}",
        mismatches.join("\n")
    );
}
