//! Cross-language regression for the chart comparison and hard-veto gates.
//! Covers the ten locales in the long-form multilingual corpus.

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};
use serde_json::Value;

struct Case {
    language: &'static str,
    positive: &'static str,
    negative: &'static str,
}

const CASES: &[Case] = &[
    Case {
        language: "it",
        positive: "Aurora ha registrato il 42% contro il 18% di Lumen.",
        negative: "La notizia è falsa: Aurora ha registrato il 42% contro il 18% di Lumen.",
    },
    Case {
        language: "en",
        positive: "Aurora recorded 42% compared with 18% for Lumen.",
        negative: "The claim is false: Aurora recorded 42% compared with 18% for Lumen.",
    },
    Case {
        language: "de",
        positive: "Aurora erreichte 42% gegenüber 18% bei Lumen.",
        negative: "Die Behauptung ist falsch: Aurora erreichte 42% gegenüber 18% bei Lumen.",
    },
    Case {
        language: "es",
        positive: "Aurora registró el 42% frente al 18% de Lumen.",
        negative: "La afirmación es falsa: Aurora registró el 42% frente al 18% de Lumen.",
    },
    Case {
        language: "fr",
        positive: "Aurora a enregistré 42% contre 18% pour Lumen.",
        negative: "L’affirmation est fausse : Aurora a enregistré 42% contre 18% pour Lumen.",
    },
    Case {
        language: "nl",
        positive: "Aurora noteerde 42% tegenover 18% voor Lumen.",
        negative: "De bewering is onjuist: Aurora noteerde 42% tegenover 18% voor Lumen.",
    },
    Case {
        language: "pl",
        positive: "Aurora uzyskała 42% wobec 18% grupy Lumen.",
        negative: "To twierdzenie jest fałszywe: Aurora uzyskała 42% wobec 18% grupy Lumen.",
    },
    Case {
        language: "ru",
        positive: "Аврора получила 42% против 18% у группы Люмен.",
        negative: "Утверждение ложно: Аврора получила 42% против 18% у группы Люмен.",
    },
    Case {
        language: "pt",
        positive: "Aurora registrou 42% contra 18% do grupo Lumen.",
        negative: "A afirmação é falsa: Aurora registrou 42% contra 18% do grupo Lumen.",
    },
    Case {
        language: "id",
        positive: "Aurora mencatat 42% dibandingkan dengan 18% pada kelompok Lumen.",
        negative: "Klaim itu tidak benar: Aurora mencatat 42% dibandingkan dengan 18% pada kelompok Lumen.",
    },
];

#[test]
fn chart_comparisons_and_epistemic_vetoes_work_in_ten_languages() {
    let mut failures = Vec::new();
    for case in CASES {
        let (positive, positive_values) = analyze(case.language, case.positive);
        let positive_charts = proposals_of(&positive, "chart");
        let mut actual: Vec<f64> = positive_charts
            .iter()
            .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
            .filter_map(|item| item["value"].as_f64())
            .collect();
        actual.sort_by(f64::total_cmp);
        if actual != [18.0, 42.0] {
            failures.push(format!(
                "{} positive: expected [18, 42], found {actual:?}; manifest={positive_values}",
                case.language
            ));
        }

        let (negative, negative_value) = analyze(case.language, case.negative);
        let negative_charts = proposals_of(&negative, "chart");
        if !negative_charts.is_empty() {
            failures.push(format!(
                "{} denied/false claim produced a chart: {negative_value}",
                case.language
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn analyze(language: &str, text: &str) -> (Value, Value) {
    let request = ScriptRequest {
        language: language.to_string(),
        scenes: vec![SceneInput {
            scene_id: "chart-multilingual".to_string(),
            text: text.to_string(),
        }],
        editorial: None,
        entity_safety_cap: 0,
    };
    let manifest = analyze_script(&request).expect("script analysis succeeds");
    let value = serde_json::to_value(manifest).expect("manifest serializes");
    (value["scenes"][0].clone(), value)
}

fn proposals_of<'a>(scene: &'a Value, kind: &str) -> Vec<&'a Value> {
    scene["visual_proposals"]
        .as_array()
        .expect("visual_proposals array")
        .iter()
        .filter(|proposal| proposal["proposal_type"] == kind)
        .collect()
}
