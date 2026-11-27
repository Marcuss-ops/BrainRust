//! TICKET-52 multilingual certification: one data row per language, no
//! per-language branch.
//!
//! The point of this suite is the directive that adding a language must cost a
//! model, a small language pack and a certification row — not a new extractor.
//! Every row runs the same generic UD subject–predicate–value resolver and
//! asserts the same canonical fact. Adding a twelfth language means adding its
//! model, its unit/role/cue data and one row in each table below; nothing in
//! `semantic-engine` changes.
//!
//! Tables are measured, not aspirational: a formulation is present only when
//! the engine projects the canonical fact on the current models. The Turkish
//! fused copula (`ağırlığındadır`) is listed as a fail-closed negative because
//! UDPipe emits it as one unknown token, so no dependency edge to the measure
//! exists; that is a model limit, not a language-specific rule.

mod common;

use common::{manifest_value_for, scene};
use serde_json::Value;

const SUPPORTED_LANGUAGES: &[&str] = &[
    "it", "en", "es", "pt", "fr", "de", "nl", "pl", "ru", "tr", "id",
];

/// `(language, sentence, canonical predicate, value, unit)`.
const ATTRIBUTES: &[(&str, &str, &str, &str, Option<&str>)] = &[
    ("it", "Marco Rossi ha 45 anni.", "age", "45", Some("year")),
    (
        "it",
        "Marco Rossi è alto 188 centimetri.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "it",
        "Marco Rossi pesa 80 chilogrammi.",
        "weight",
        "80",
        Some("kg"),
    ),
    (
        "it",
        "Marco Rossi è un medico.",
        "profession",
        "medico",
        None,
    ),
    ("en", "Marco Rossi is aged 45.", "age", "45", Some("year")),
    (
        "en",
        "Marco Rossi is 188 centimeters tall.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "en",
        "Marco Rossi weighs 80 kilograms.",
        "weight",
        "80",
        Some("kg"),
    ),
    (
        "en",
        "Marco Rossi is a doctor.",
        "profession",
        "doctor",
        None,
    ),
    (
        "es",
        "Marco Rossi tiene 45 años.",
        "age",
        "45",
        Some("year"),
    ),
    (
        "es",
        "Marco Rossi mide 188 centímetros.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "es",
        "Marco Rossi pesa 80 kilogramos.",
        "weight",
        "80",
        Some("kg"),
    ),
    ("es", "Marco Rossi es médico.", "profession", "médico", None),
    ("pt", "Marco Rossi tem 45 anos.", "age", "45", Some("year")),
    (
        "pt",
        "Marco Rossi mede 188 centímetros.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "pt",
        "Marco Rossi pesa 80 quilos.",
        "weight",
        "80",
        Some("kg"),
    ),
    ("pt", "Marco Rossi é médico.", "profession", "médico", None),
    ("fr", "Marco Rossi a 45 ans.", "age", "45", Some("year")),
    (
        "fr",
        "Marco Rossi mesure 188 centimètres.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "fr",
        "Marco Rossi pèse 80 kilogrammes.",
        "weight",
        "80",
        Some("kg"),
    ),
    (
        "fr",
        "Marco Rossi est médecin.",
        "profession",
        "médecin",
        None,
    ),
    (
        "de",
        "Marco Rossi ist 45 Jahre alt.",
        "age",
        "45",
        Some("year"),
    ),
    (
        "de",
        "Marco Rossi ist 188 Zentimeter groß.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "de",
        "Marco Rossi wiegt 80 Kilogramm.",
        "weight",
        "80",
        Some("kg"),
    ),
    ("de", "Marco Rossi ist Arzt.", "profession", "Arzt", None),
    (
        "nl",
        "Marco Rossi is 45 jaar oud.",
        "age",
        "45",
        Some("year"),
    ),
    (
        "nl",
        "Marco Rossi is 188 centimeter lang.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "nl",
        "Marco Rossi weegt 80 kilogram.",
        "weight",
        "80",
        Some("kg"),
    ),
    ("nl", "Marco Rossi is arts.", "profession", "arts", None),
    ("pl", "Marco Rossi ma 45 lat.", "age", "45", Some("year")),
    (
        "pl",
        "Marco Rossi ma 188 centymetrów wzrostu.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "pl",
        "Marco Rossi waży 80 kilogramów.",
        "weight",
        "80",
        Some("kg"),
    ),
    ("pl", "Marco Rossi to lekarz.", "profession", "lekarz", None),
    ("ru", "Марко Росси 45 лет.", "age", "45", Some("year")),
    (
        "ru",
        "Рост Марко Росси составляет 188 сантиметров.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "ru",
        "Марко Росси весит 80 килограммов.",
        "weight",
        "80",
        Some("kg"),
    ),
    ("ru", "Марко Росси — врач.", "profession", "врач", None),
    ("tr", "Marco Rossi 45 yaşında.", "age", "45", Some("year")),
    (
        "tr",
        "Marco Rossi 188 santimetre boyunda.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "tr",
        "Marco Rossi 80 kilo ağırlığında.",
        "weight",
        "80",
        Some("kg"),
    ),
    (
        "tr",
        "Marco Rossi bir doktor.",
        "profession",
        "doktor",
        None,
    ),
    (
        "id",
        "Marco Rossi berusia 45 tahun.",
        "age",
        "45",
        Some("year"),
    ),
    (
        "id",
        "Marco Rossi memiliki tinggi 188 sentimeter.",
        "height",
        "188",
        Some("cm"),
    ),
    (
        "id",
        "Marco Rossi memiliki berat 80 kilogram.",
        "weight",
        "80",
        Some("kg"),
    ),
    (
        "id",
        "Marco Rossi adalah dokter.",
        "profession",
        "dokter",
        None,
    ),
];

/// One negated weight per language. The claim must stay in the fact inventory
/// as `negated` (or yield no fact when the model drops the clause) and never
/// enter a positive card.
const NEGATED_WEIGHTS: &[(&str, &str)] = &[
    ("it", "Marco Rossi non pesa 80 chilogrammi."),
    ("en", "Marco Rossi does not weigh 80 kilograms."),
    ("es", "Marco Rossi no pesa 80 kilogramos."),
    ("pt", "Marco Rossi não pesa 80 quilos."),
    ("fr", "Marco Rossi ne pèse pas 80 kilogrammes."),
    ("de", "Marco Rossi wiegt nicht 80 Kilogramm."),
    ("nl", "Marco Rossi weegt niet 80 kilogram."),
    ("pl", "Marco Rossi nie waży 80 kilogramów."),
    ("ru", "Марко Росси не весит 80 килограммов."),
    ("tr", "Marco Rossi 80 kilo değil."),
    ("id", "Marco Rossi tidak memiliki berat 80 kilogram."),
];

/// The measure-first formulation ("the weight of X is ...") in every language
/// where the model exposes a predicate edge. Turkish is deliberately absent:
/// `ağırlığındadır` is one unknown token, so the lane fails closed; see
/// `the_turkish_fused_copula_fails_closed` below.
const INVERSE_WEIGHTS: &[(&str, &str)] = &[
    ("it", "Il peso di Marco Rossi è di 80 chilogrammi."),
    ("en", "Marco Rossi's weight is 80 kilograms."),
    ("es", "El peso de Marco Rossi es de 80 kilogramos."),
    ("pt", "O peso de Marco Rossi é de 80 kg."),
    ("fr", "Le poids de Marco Rossi est de 80 kilogrammes."),
    ("de", "Das Gewicht von Marco Rossi beträgt 80 Kilogramm."),
    ("nl", "Het gewicht van Marco Rossi is 80 kilogram."),
    ("pl", "Waga Marco Rossi wynosi 80 kilogramów."),
    ("ru", "Вес Марко Росси составляет 80 килограммов."),
    ("id", "Berat Marco Rossi adalah 80 kilogram."),
];

/// Released for the 11 supported languages: `(language, sentence)`.
const PLURAL_SUBJECTS: &[(&str, &str)] = &[
    ("it", "Marco Rossi e Luca Bianchi sono alti 180 centimetri."),
    (
        "en",
        "Marco Rossi and Luca Bianchi are 180 centimeters tall.",
    ),
    ("es", "Marco Rossi y Luca Bianchi miden 180 centímetros."),
    ("pt", "Marco Rossi e Luca Bianchi medem 180 centímetros."),
    (
        "fr",
        "Marco Rossi et Luca Bianchi mesurent 180 centimètres.",
    ),
    (
        "de",
        "Marco Rossi und Luca Bianchi sind 180 Zentimeter groß.",
    ),
    (
        "nl",
        "Marco Rossi en Luca Bianchi zijn 180 centimeter lang.",
    ),
    (
        "pl",
        "Marco Rossi i Luca Bianchi mają 180 centymetrów wzrostu.",
    ),
    (
        "ru",
        "Марко Росси и Лука Бьянки имеют рост 180 сантиметров.",
    ),
    ("tr", "Marco Rossi ve Luca Bianchi 180 santimetre boyunda."),
    (
        "id",
        "Marco Rossi dan Luca Bianchi memiliki tinggi 180 sentimeter.",
    ),
];

/// `(language, sentence, subject label, age label, height label)`. The labels
/// come from the one canonical ontology (`lexicon/attribute_predicates.json`),
/// never from a Rust match.
const ATTRIBUTE_SUMMARIES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "it",
        "Marco Rossi ha 45 anni ed è alto 188 centimetri.",
        "Marco Rossi",
        "Età",
        "Altezza",
    ),
    (
        "en",
        "Marco Rossi is aged 45 and is 188 centimeters tall.",
        "Marco Rossi",
        "Age",
        "Height",
    ),
    (
        "es",
        "Marco Rossi tiene 45 años y mide 188 centímetros.",
        "Marco Rossi",
        "Edad",
        "Altura",
    ),
    (
        "pt",
        "Marco Rossi tem 45 anos e mede 188 centímetros.",
        "Marco Rossi",
        "Idade",
        "Altura",
    ),
    (
        "fr",
        "Marco Rossi a 45 ans et mesure 188 centimètres.",
        "Marco Rossi",
        "Âge",
        "Taille",
    ),
    (
        "de",
        "Marco Rossi ist 45 Jahre alt und 188 Zentimeter groß.",
        "Marco Rossi",
        "Alter",
        "Größe",
    ),
    (
        "nl",
        "Marco Rossi is 45 jaar oud en 188 centimeter lang.",
        "Marco Rossi",
        "Leeftijd",
        "Lengte",
    ),
    (
        "pl",
        "Marco Rossi ma 45 lat i 188 centymetrów wzrostu.",
        "Marco Rossi",
        "Wiek",
        "Wzrost",
    ),
    (
        "ru",
        "Марко Росси 45 лет, рост 188 сантиметров.",
        "Марко Росси",
        "Возраст",
        "Рост",
    ),
    (
        "tr",
        "Marco Rossi 45 yaşında ve 188 santimetre boyunda.",
        "Marco Rossi",
        "Yaş",
        "Boy",
    ),
    (
        "id",
        "Marco Rossi memiliki tinggi 188 sentimeter dan berusia 45 tahun.",
        "Marco Rossi",
        "Tinggi",
        "Usia",
    ),
];

fn scene_manifest(language: &str, text: &str) -> Value {
    let manifest = manifest_value_for(language, vec![scene("certification", text)]);
    manifest["scenes"][0].clone()
}

fn lists(scene: &Value) -> Vec<Value> {
    scene["entity_attribute_lists"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn attribute<'a>(list: &'a Value, predicate: &str) -> Option<&'a Value> {
    list["attributes"]
        .as_array()?
        .iter()
        .find(|attribute| attribute["predicate"] == predicate)
}

fn asserted_attribute(scene: &Value, predicate: &str) -> Option<(Value, Value)> {
    lists(scene).into_iter().find_map(|list| {
        attribute(&list, predicate)
            .filter(|attribute| attribute["status"] == "asserted")
            .map(|attribute| (list.clone(), attribute.clone()))
    })
}

#[test]
fn every_supported_language_projects_the_same_canonical_attributes() {
    for (language, text, predicate, value, unit) in ATTRIBUTES {
        let scene = scene_manifest(language, text);
        let (list, attribute) = asserted_attribute(&scene, predicate).unwrap_or_else(|| {
            panic!("{language}: missing asserted {predicate} for {text}: {scene:?}")
        });
        assert_eq!(attribute["value"], *value, "{language}: {text}");
        match unit {
            Some(unit) => assert_eq!(attribute["unit"], *unit, "{language}: {text}"),
            None => assert!(attribute["unit"].is_null(), "{language}: {text}"),
        }
        // Every attribute keeps an exact slice of the subject sentence.
        let start = attribute["start_byte"].as_u64().unwrap() as usize;
        let end = attribute["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&text[start..end], *value, "{language}: {text}");
        assert!(!list["label"].as_str().unwrap_or_default().is_empty());
        assert!(
            text.contains(attribute["evidence"].as_str().unwrap()),
            "{language}: evidence is not the claiming sentence: {attribute}"
        );
    }
}

#[test]
fn every_supported_language_keeps_a_negated_weight_out_of_the_card() {
    for (language, text) in NEGATED_WEIGHTS {
        let scene = scene_manifest(language, text);
        assert!(
            asserted_attribute(&scene, "weight").is_none(),
            "{language}: negation became a positive card: {scene:?}"
        );
        // When the extractor kept the claim, it is explicitly negated, never
        // silently asserted.
        for fact in scene["entity_facts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            if fact["predicate"] == "weight" {
                assert_eq!(fact["status"], "negated", "{language}: {fact}");
            }
        }
    }
}

#[test]
fn every_supported_language_binds_a_measure_first_formulation() {
    for (language, text) in INVERSE_WEIGHTS {
        let scene = scene_manifest(language, text);
        let (_, weight) = asserted_attribute(&scene, "weight")
            .unwrap_or_else(|| panic!("{language}: missing weight for {text}: {scene:?}"));
        assert_eq!(weight["value"], "80", "{language}: {text}");
    }
}

#[test]
fn the_turkish_fused_copula_fails_closed() {
    // `ağırlığındadır` is one unknown token in the UDPipe 2.5 Turkish model.
    // No dependency edge reaches the measure noun, so the generic resolver
    // emits nothing instead of guessing the subject.
    let scene = scene_manifest("tr", "Marco Rossi 80 kilo ağırlığındadır.");
    assert!(asserted_attribute(&scene, "weight").is_none(), "{scene:?}");
}

/// Languages that express the predicate profession in the instrumental case
/// (`jest lekarzem`, `работает врачом`). The surface is pack data, so binding
/// it needs no branch: `(language, sentence, surface)`.
const INSTRUMENTAL_PROFESSIONS: &[(&str, &str, &str)] = &[
    ("pl", "Marco Rossi jest lekarzem.", "lekarzem"),
    ("ru", "Марко Росси работает врачом.", "врачом"),
];

#[test]
fn an_instrumental_profession_binds_to_its_subject() {
    for (language, text, surface) in INSTRUMENTAL_PROFESSIONS {
        let scene = scene_manifest(language, text);
        let (_, profession) = asserted_attribute(&scene, "profession")
            .unwrap_or_else(|| panic!("{language}: missing profession for {text}: {scene:?}"));
        assert_eq!(profession["value"], *surface, "{language}: {text}");
        let start = profession["start_byte"].as_u64().unwrap() as usize;
        let end = profession["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&text[start..end], *surface, "{language}: {text}");
    }
}

#[test]
fn a_plural_subject_is_never_split_in_any_language() {
    for (language, text) in PLURAL_SUBJECTS {
        let scene = scene_manifest(language, text);
        assert!(
            asserted_attribute(&scene, "height").is_none(),
            "{language}: a plural subject became one person's height: {scene:?}"
        );
    }
}

#[test]
fn every_supported_language_projects_the_same_localized_attribute_summary() {
    for (language, text, subject, age_label, height_label) in ATTRIBUTE_SUMMARIES {
        let scene = scene_manifest(language, text);
        let summaries: Vec<&Value> = scene["micro_summaries"]
            .as_array()
            .map(|summaries| {
                summaries
                    .iter()
                    .filter(|summary| summary["kind"] == "attribute_summary")
                    .collect()
            })
            .unwrap_or_default();
        let summary = summaries
            .first()
            .unwrap_or_else(|| panic!("{language}: missing attribute summary: {scene:?}"));
        let rendered = summary["text"].as_str().unwrap();
        assert!(
            rendered.starts_with(&format!("{subject}: ")),
            "{language}: {rendered}"
        );
        assert!(rendered.contains(age_label), "{language}: {rendered}");
        assert!(rendered.contains(height_label), "{language}: {rendered}");
        assert_eq!(
            summary["evidence_fact_ids"].as_array().unwrap().len(),
            2,
            "{language}: {summary}"
        );
        assert_eq!(summary["evidence"], *text, "{language}: {summary}");
    }
}

/// A new language is only certified when every table has a row for it. This
/// fails the moment a language is added to the engine without its data and
/// certification cases, which is the whole point of the table-driven design.
#[test]
fn every_table_covers_exactly_the_supported_languages() {
    let languages = |rows: Vec<&'static str>| {
        let mut set = rows;
        set.sort_unstable();
        set.dedup();
        set
    };
    let mut expected = SUPPORTED_LANGUAGES.to_vec();
    expected.sort_unstable();
    assert_eq!(
        languages(ATTRIBUTES.iter().map(|row| row.0).collect()),
        expected
    );
    assert_eq!(
        languages(NEGATED_WEIGHTS.iter().map(|row| row.0).collect()),
        expected
    );
    assert_eq!(
        languages(PLURAL_SUBJECTS.iter().map(|row| row.0).collect()),
        expected
    );
    assert_eq!(
        languages(ATTRIBUTE_SUMMARIES.iter().map(|row| row.0).collect()),
        expected
    );
    // The inverse table is one row short by design; the missing language is
    // the documented fail-closed case, not an untested one.
    let mut inverse = INVERSE_WEIGHTS.iter().map(|row| row.0).collect::<Vec<_>>();
    inverse.sort_unstable();
    let mut expected_inverse = expected.clone();
    expected_inverse.retain(|language| *language != "tr");
    assert_eq!(inverse, expected_inverse);
}
