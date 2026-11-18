//! Go→Rust goldens for the editorial quality gate.
//!
//! `tests/fixtures/quality_gate/goldens.json` was frozen from the canonical Go
//! gate (`refactored/internal/capabilities/scripts/usecase/qualitygate`) before
//! the Rust cutover. Each case carries the exact wire request the Go adapter
//! sends plus the metrics and reason list the Go gate produced, so this test
//! pins both the verdict and the wording after the Go implementation is gone.
//!
//! The corpus covers every rule in the Go registry: language match (whole text
//! and per scene), source coverage per grounding policy, source date
//! preservation, clip coverage, unsupported claims, research candidate
//! coverage, segment narrative (count, chapter minimum, boxer topics) and
//! target words; plus the two pre-metric hard stops (empty and generic text).

use std::fs;
use std::path::PathBuf;

use script_qa::gate::evaluate;
use script_qa::model::{GateRequest, Quality};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct GoldenFile {
    version: u32,
    cases: Vec<GoldenCase>,
}

#[derive(Debug, Deserialize)]
struct GoldenCase {
    name: String,
    request: serde_json::Value,
    expected: ExpectedVerdict,
}

#[derive(Debug, Deserialize)]
struct ExpectedVerdict {
    quality: Option<Quality>,
    #[serde(default)]
    reasons: Vec<String>,
}

fn golden_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("quality_gate")
        .join("goldens.json")
}

fn load_goldens() -> GoldenFile {
    let path = golden_file();
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let goldens: GoldenFile = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    assert_eq!(goldens.version, 1, "unsupported goldens version");
    assert!(!goldens.cases.is_empty(), "goldens corpus is empty");
    goldens
}

#[test]
fn goldens_reproduce_the_go_quality_gate_verdicts() {
    let goldens = load_goldens();

    for case in &goldens.cases {
        let request: GateRequest = serde_json::from_value(case.request.clone())
            .unwrap_or_else(|error| panic!("case {}: invalid request: {error}", case.name));

        match (evaluate(&request), &case.expected.quality) {
            (None, None) => {}
            (Some(actual), Some(expected)) => {
                assert_eq!(&actual.quality, expected, "case {}: quality", case.name);
                assert_eq!(
                    actual.reasons, case.expected.reasons,
                    "case {}: reasons",
                    case.name
                );
            }
            (actual, expected) => panic!(
                "case {}: verdict shape mismatch (rust produced a verdict: {}, goldens carry one: {})",
                case.name,
                actual.is_some(),
                expected.is_some()
            ),
        }
    }
}

/// A failing golden without a reason would not pin anything: the adapter turns
/// an empty reason list into a `QualityGateError` with no explanation.
#[test]
fn failing_goldens_always_name_a_reason() {
    let goldens = load_goldens();
    for case in &goldens.cases {
        let Some(expected) = &case.expected.quality else {
            continue;
        };
        if expected.passed {
            assert!(
                case.expected.reasons.is_empty(),
                "case {}: a passing verdict must not carry reasons",
                case.name
            );
        } else {
            assert!(
                !case.expected.reasons.is_empty(),
                "case {}: a failing verdict must name at least one reason",
                case.name
            );
        }
    }
}

#[test]
fn golden_names_are_unique() {
    let goldens = load_goldens();
    let mut names: Vec<&str> = goldens
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(total, names.len(), "duplicate case name in the goldens");
}
