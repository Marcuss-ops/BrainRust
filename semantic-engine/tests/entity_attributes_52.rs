//! TICKET-52 item 1: person attributes are grounded projections of existing
//! facts, resolved through Universal Dependencies. No test asserts a wording
//! the engine does not produce; every span is checked against the source.

mod common;

use common::{manifest_value_for, scene};
use serde_json::Value;

fn scene_manifest(language: &str, text: &str) -> Value {
    let manifest = manifest_value_for(language, vec![scene("attributes", text)]);
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

#[test]
fn one_person_card_groups_age_height_and_weight() {
    let source = "Andrea Semprio ha 45 anni, è alto 188 centimetri e pesa 789 chilogrammi.";
    let scene = scene_manifest("it", source);
    let lists = lists(&scene);
    assert_eq!(lists.len(), 1, "{lists:?}");
    let list = &lists[0];
    assert_eq!(list["label"], "Andrea Semprio");
    assert_eq!(list["attributes"].as_array().unwrap().len(), 3, "{list}");

    let age = attribute(list, "age").expect("age");
    assert_eq!(age["value"], "45");
    assert_eq!(age["unit"], "year");
    assert_eq!(age["unit_surface"], "anni");
    assert_eq!(age["status"], "asserted");

    let height = attribute(list, "height").expect("height");
    assert_eq!(height["value"], "188");
    assert_eq!(height["unit"], "cm");
    assert_eq!(height["unit_surface"], "centimetri");

    let weight = attribute(list, "weight").expect("weight");
    assert_eq!(weight["value"], "789");
    assert_eq!(weight["unit"], "kg");
    assert_eq!(weight["unit_surface"], "chilogrammi");

    for attribute in list["attributes"].as_array().unwrap() {
        let start = attribute["start_byte"].as_u64().unwrap() as usize;
        let end = attribute["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&source[start..end], attribute["value"].as_str().unwrap());
        assert_eq!(attribute["status"], "asserted");
    }
}

#[test]
fn two_people_keep_their_own_height() {
    let source = "Marco Rossi è alto 188 cm, Luca Bianchi 175 cm.";
    let scene = scene_manifest("it", source);
    let lists = lists(&scene);
    assert_eq!(lists.len(), 2, "{lists:?}");
    let by_label = |label: &str| {
        lists
            .iter()
            .find(|list| list["label"] == label)
            .unwrap_or_else(|| panic!("missing list for {label}: {lists:?}"))
    };
    assert_eq!(
        attribute(by_label("Marco Rossi"), "height").unwrap()["value"],
        "188"
    );
    assert_eq!(
        attribute(by_label("Luca Bianchi"), "height").unwrap()["value"],
        "175"
    );
}

#[test]
fn negated_weight_never_becomes_a_positive_attribute() {
    let source = "Marco Rossi non pesa 80 kg.";
    let scene = scene_manifest("it", source);
    assert!(
        lists(&scene).is_empty(),
        "{}",
        scene["entity_attribute_lists"]
    );
    let facts = scene["entity_facts"].as_array().unwrap();
    let weight = facts
        .iter()
        .find(|fact| fact["predicate"] == "weight")
        .expect("the negated claim stays in the inventory");
    assert_eq!(weight["status"], "negated");
}

#[test]
fn approximation_is_preserved() {
    let source = "Marco Rossi pesa circa 80 kg.";
    let scene = scene_manifest("it", source);
    let lists = lists(&scene);
    assert_eq!(lists.len(), 1, "{lists:?}");
    let weight = attribute(&lists[0], "weight").expect("weight");
    assert_eq!(weight["qualifier"], "approx");
    assert_eq!(weight["value"], "80");
}

#[test]
fn a_distance_run_is_not_a_height_attribute() {
    let source = "Marco Rossi ha corso 10 km.";
    let scene = scene_manifest("it", source);
    assert!(
        lists(&scene).is_empty(),
        "{}",
        scene["entity_attribute_lists"]
    );
}

#[test]
fn height_resolves_from_an_unseen_inflection_and_a_possessive_form() {
    for source in [
        "Marco Rossi pesava 82 kg.",
        "Il peso di Marco Rossi è di 80 kg.",
        "Marco Rossi, che pesa circa 80 kg, è un medico.",
    ] {
        let scene = scene_manifest("it", source);
        let lists = lists(&scene);
        assert_eq!(
            lists.len(),
            1,
            "{source}: {}",
            scene["entity_attribute_lists"]
        );
        let weight = attribute(&lists[0], "weight")
            .unwrap_or_else(|| panic!("{source}: missing weight: {lists:?}"));
        assert_eq!(
            weight["status"], "asserted",
            "{source}: {}",
            scene["entity_attribute_lists"]
        );
    }
}

#[test]
fn a_plural_subject_is_never_split_into_one_person() {
    let source = "Marco Rossi e Luca Bianchi sono alti 180 cm.";
    let scene = scene_manifest("it", source);
    assert!(
        lists(&scene).is_empty(),
        "{}",
        scene["entity_attribute_lists"]
    );
}

#[test]
fn the_card_is_one_production_recipe_and_owns_its_facts() {
    let source = "Andrea Semprio ha 45 anni, è alto 188 centimetri e pesa 789 chilogrammi.";
    let manifest = manifest_value_for("it", vec![scene("attributes", source)]);
    let candidates = manifest["visual_consolidation"]["production_candidates"]
        .as_array()
        .unwrap();
    let card = candidates
        .iter()
        .find(|candidate| candidate["recipe"] == "entity_attribute_list")
        .unwrap_or_else(|| panic!("missing attribute card: {candidates:?}"));
    let payload = &card["visual_payload"];
    assert_eq!(payload["schema"], "visual_recipe.v1");
    assert_eq!(
        payload["attributes"].as_array().unwrap().len(),
        3,
        "{payload}"
    );
    let lines = payload["lines"].as_array().unwrap();
    assert!(lines.iter().any(|line| line == "Età 45 anni"), "{lines:?}");
    assert!(
        lines.iter().any(|line| line == "Altezza 188 centimetri"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line == "Peso 789 chilogrammi"),
        "{lines:?}"
    );
    for id in card["covered_fact_ids"].as_array().unwrap() {
        assert!(
            id.as_str().unwrap().starts_with("cf_"),
            "non-canonical coverage {id} in {card:?}"
        );
    }

    // Every attribute fact is represented by the card, and no lone fact card
    // for the same claim survives.
    let facts = manifest["visual_consolidation"]["facts"]
        .as_array()
        .unwrap();
    assert_eq!(
        facts.len(),
        3,
        "one canonical fact per attribute: {facts:?}"
    );
    for fact in facts {
        assert_eq!(
            fact["representation"]["recipe"], "entity_attribute_list",
            "every covered fact is represented by the card: {fact}"
        );
    }
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate["recipe"] == "fact_card"),
        "an attribute fact must not also become a lone card: {candidates:?}"
    );
}

#[test]
fn the_card_projects_one_generic_attribute_micro_summary_without_a_legal_duplicate() {
    let source = "Andrea Semprio ha 45 anni, è alto 188 centimetri e pesa 789 chilogrammi.";
    let manifest = manifest_value_for("it", vec![scene("attributes", source)]);
    let summaries = manifest["scenes"][0]["micro_summaries"].as_array().unwrap();
    let attribute: Vec<&serde_json::Value> = summaries
        .iter()
        .filter(|summary| summary["kind"] == "attribute_summary")
        .collect();
    assert_eq!(attribute.len(), 1, "{summaries:?}");
    assert_eq!(
        attribute[0]["text"],
        "Andrea Semprio: Età 45 anni · Altezza 188 cm · Peso 789 kg"
    );
    assert_eq!(
        attribute[0]["evidence_fact_ids"].as_array().unwrap().len(),
        3
    );
    assert_eq!(attribute[0]["evidence"], source);
    assert!(
        summaries
            .iter()
            .filter(|summary| summary["kind"] == "micro_summary")
            .all(|summary| summary["text"] != "Andrea Semprio: 45 anni"),
        "the legal age phrase must not repeat the attribute summary: {summaries:?}"
    );
}

#[test]
fn the_same_ud_binding_works_in_every_supported_language() {
    let cases = [
        ("en", "Marco Rossi is 188 centimeters tall."),
        ("de", "Marco Rossi ist 188 Zentimeter groß."),
        ("fr", "Marco Rossi mesure 188 centimètres."),
        ("es", "Marco Rossi mide 188 centímetros."),
        ("pl", "Marco Rossi ma 188 centymetrów wzrostu."),
        ("ru", "Рост Марко Росси составляет 188 сантиметров."),
        ("tr", "Marco Rossi 188 santimetre boyunda."),
        ("id", "Marco Rossi memiliki tinggi 188 sentimeter."),
    ];
    for (language, source) in cases {
        let scene = scene_manifest(language, source);
        let lists = lists(&scene);
        let height = lists
            .first()
            .and_then(|list| attribute(list, "height"))
            .unwrap_or_else(|| panic!("{language}: no height card for {source}: {lists:?}"));
        assert_eq!(height["value"], "188", "{language}");
    }
}
