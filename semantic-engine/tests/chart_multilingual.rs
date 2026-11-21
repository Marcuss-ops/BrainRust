//! Cross-language regression for the chart comparison and hard-veto gates.
//! Covers all eleven locales supported by the semantic engine.

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
    Case {
        language: "tr",
        positive: "Aurora yüzde 42'ye ulaştı; bu oran Lumen'in yüzde 18'ine kıyasla daha yüksek.",
        negative: "Bu iddia yanlış: Aurora yüzde 42'ye ulaştı; bu oran Lumen'in yüzde 18'ine kıyasla daha yüksek.",
    },
];

const SERIES_CASES: &[Case] = &[
    Case {
        language: "it",
        positive: "Nel 2022 Aurora ha venduto 120 auto; nel 2023 ne ha vendute 180.",
        negative: "",
    },
    Case {
        language: "en",
        positive: "In 2022 Aurora sold 120 cars; in 2023 it sold 180.",
        negative: "",
    },
    Case {
        language: "de",
        positive: "2022 verkaufte Aurora 120 Autos; 2023 verkaufte Aurora 180 Autos.",
        negative: "",
    },
    Case {
        language: "es",
        positive: "En 2022 Aurora vendió 120 coches; en 2023 vendió 180.",
        negative: "",
    },
    Case {
        language: "fr",
        positive: "En 2022, Aurora a vendu 120 voitures ; en 2023, elle en a vendu 180.",
        negative: "",
    },
    Case {
        language: "nl",
        positive: "In 2022 verkocht Aurora 120 auto's; Aurora verkocht in 2023 180 auto's.",
        negative: "",
    },
    Case {
        language: "pl",
        positive: "W 2022 roku Aurora sprzedała 120 samochodów, a w 2023 roku 180.",
        negative: "",
    },
    Case {
        language: "ru",
        positive: "В 2022 году Aurora продала 120 автомобилей, а в 2023 году — 180.",
        negative: "",
    },
    Case {
        language: "pt",
        positive: "Em 2022, Aurora vendeu 120 carros; em 2023, vendeu 180.",
        negative: "",
    },
    Case {
        language: "id",
        positive: "Pada 2022, Aurora menjual 120 mobil; pada 2023, Aurora menjual 180 mobil.",
        negative: "",
    },
    Case {
        language: "tr",
        positive: "2022'de Aurora 120 otomobil sattı; 2023'te 180 otomobil sattı.",
        negative: "",
    },
];

struct CurrencyCase {
    language: &'static str,
    source: &'static str,
    incompatible: &'static str,
}

const CURRENCY_CASES: &[CurrencyCase] = &[
    CurrencyCase {
        language: "it",
        source: "Nel 2023 il fatturato di Alfa è stato di 2 milioni di euro; il fatturato di Beta è stato di 1 milione di euro.",
        incompatible: "Nel 2023 il fatturato di Alfa è stato di 2 milioni di euro; il fatturato di Beta è stato di 1 milione di dollari.",
    },
    CurrencyCase {
        language: "en",
        source: "In 2023 Alfa's revenue was 2 million euros; Beta's revenue was 1 million euros.",
        incompatible: "In 2023 Alfa's revenue was 2 million euros; Beta's revenue was 1 million dollars.",
    },
    CurrencyCase {
        language: "de",
        source: "2023 lag der Umsatz von Alfa bei 2 Millionen Euro; der Umsatz von Beta lag bei 1 Million Euro.",
        incompatible: "2023 lag der Umsatz von Alfa bei 2 Millionen Euro; der Umsatz von Beta lag bei 1 Million US-Dollar.",
    },
    CurrencyCase {
        language: "es",
        source: "En 2023 los ingresos de Alfa fueron 2 millones de euros; los ingresos de Beta fueron 1 millón de euros.",
        incompatible: "En 2023 los ingresos de Alfa fueron 2 millones de euros; los ingresos de Beta fueron 1 millón de dólares.",
    },
    CurrencyCase {
        language: "fr",
        source: "En 2023, Alfa a déclaré 2 millions d'euros de chiffre d'affaires; Beta a déclaré 1 million d'euros.",
        incompatible: "En 2023, Alfa a déclaré 2 millions d'euros de chiffre d'affaires; Beta a déclaré 1 million de dollars.",
    },
    CurrencyCase {
        language: "nl",
        source: "In 2023 bedroeg de omzet van Alfa 2 miljoen euro; de omzet van Beta bedroeg 1 miljoen euro.",
        incompatible: "In 2023 bedroeg de omzet van Alfa 2 miljoen euro; de omzet van Beta bedroeg 1 miljoen dollar.",
    },
    CurrencyCase {
        language: "pl",
        source: "W 2023 roku przychody Alfy wyniosły 2000000 euro; przychody Bety wyniosły 1000000 euro.",
        incompatible: "W 2023 roku przychody Alfy wyniosły 2000000 euro; przychody Bety wyniosły 1000000 dolarów.",
    },
    CurrencyCase {
        language: "ru",
        source: "В 2023 году выручка Alfa составила 2 миллиона евро; выручка Beta — 1 миллион евро.",
        incompatible: "В 2023 году выручка Alfa составила 2 миллиона евро; выручка Beta — 1 миллион долларов.",
    },
    CurrencyCase {
        language: "pt",
        source: "Em 2023, o faturamento da Alfa foi de 2 milhões de euros; o faturamento da Beta foi de 1 milhão de euros.",
        incompatible: "Em 2023, o faturamento da Alfa foi de 2 milhões de euros; o faturamento da Beta foi de 1 milhão de dólares.",
    },
    CurrencyCase {
        language: "id",
        source: "Pada 2023, pendapatan Alfa sebesar 2 juta euro; pendapatan Beta sebesar 1 juta euro.",
        incompatible: "Pada 2023, pendapatan Alfa sebesar 2 juta euro; pendapatan Beta sebesar 1 juta dolar.",
    },
    CurrencyCase {
        language: "tr",
        source: "2023'te Alfa'nın geliri 2 milyon avro, Beta'nın geliri 1 milyon avro oldu.",
        incompatible: "2023'te Alfa'nın geliri 2 milyon avro, Beta'nın geliri 1 milyon dolar oldu.",
    },
];

#[test]
fn chart_comparisons_and_epistemic_vetoes_work_in_eleven_languages() {
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

#[test]
fn annual_series_are_grounded_in_eleven_languages() {
    let mut failures = Vec::new();
    for case in SERIES_CASES {
        let (scene, _) = analyze(case.language, case.positive);
        let charts = proposals_of(&scene, "chart");
        let mut actual: Vec<f64> = charts
            .iter()
            .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
            .filter_map(|item| item["value"].as_f64())
            .collect();
        actual.sort_by(f64::total_cmp);
        if actual != [120.0, 180.0] {
            failures.push(format!(
                "{} annual series: expected [120, 180], found {actual:?}; scene={scene}",
                case.language
            ));
            continue;
        }
        let labels: Vec<&str> = charts
            .iter()
            .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
            .filter_map(|item| item["label"].as_str())
            .collect();
        if labels.len() != 2
            || !labels.iter().any(|label| label.contains("2022"))
            || !labels.iter().any(|label| label.contains("2023"))
        {
            failures.push(format!(
                "{} annual series labels must bind to years 2022 and 2023: {labels:?}",
                case.language
            ));
        }
        for item in charts
            .iter()
            .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
        {
            let start = item["start_byte"].as_u64().unwrap_or_default() as usize;
            let end = item["end_byte"].as_u64().unwrap_or_default() as usize;
            if case.positive.get(start..end) != item["text"].as_str() {
                failures.push(format!(
                    "{} annual series item is not a byte-exact source slice: {item}",
                    case.language
                ));
            }
            if item["label"].as_str().unwrap_or_default().is_empty() {
                failures.push(format!(
                    "{} annual series item has no label: {item}",
                    case.language
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn scaled_currency_comparisons_are_grounded_in_eleven_languages() {
    let mut failures = Vec::new();
    for case in CURRENCY_CASES {
        let (scene, _) = analyze(case.language, case.source);
        let charts = proposals_of(&scene, "chart");
        let mut actual: Vec<f64> = charts
            .iter()
            .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
            .filter_map(|item| item["value"].as_f64())
            .collect();
        actual.sort_by(f64::total_cmp);
        if actual != [1_000_000.0, 2_000_000.0] {
            failures.push(format!(
                "{} currency comparison: expected [1e6, 2e6], found {actual:?}; scene={scene}",
                case.language
            ));
            continue;
        }
        for item in charts
            .iter()
            .flat_map(|chart| chart["chart_items"].as_array().into_iter().flatten())
        {
            let start = item["start_byte"].as_u64().unwrap_or_default() as usize;
            let end = item["end_byte"].as_u64().unwrap_or_default() as usize;
            if case.source.get(start..end) != item["text"].as_str() {
                failures.push(format!(
                    "{} currency item is not a byte-exact source slice: {item}",
                    case.language
                ));
            }
            let label = item["label"].as_str().unwrap_or_default().to_lowercase();
            if !label.contains("alfa")
                && !label.contains("alfy")
                && !label.contains("beta")
                && !label.contains("bety")
            {
                failures.push(format!(
                    "{} currency item label is not Alfa/Beta: {:?}",
                    case.language, item["label"]
                ));
            }
        }
        let (incompatible_scene, _) = analyze(case.language, case.incompatible);
        if !proposals_of(&incompatible_scene, "chart").is_empty() {
            failures.push(format!(
                "{} mixed-currency comparison produced a chart: {}; quantities={}; charts={:?}",
                case.language,
                case.incompatible,
                incompatible_scene["quantities"],
                proposals_of(&incompatible_scene, "chart")
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

        reference_date: None,
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
