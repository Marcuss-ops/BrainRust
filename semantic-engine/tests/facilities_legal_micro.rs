//! Facility facts, legal-pass facts and fact-backed micro summaries.

mod common;

use common::*;
use serde_json::Value;

#[test]
fn fictional_stadium_date_and_capacity_facts_link_to_the_facility() {
    let text = "Stadio Aurora fu inaugurato nel 1959, ha capienza superiore a 10.000 posti, è di proprietà della famiglia Rossi e si trova a Torino.";
    let value = manifest_value(vec![scene("stadio-aurora", text)]);
    let scene = &value["scenes"][0];
    let facility = scene["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["type"] == "FACILITY")
        .expect("explicitly named facility should be recognized");
    let facts = scene["entity_facts"].as_array().unwrap();
    let inauguration = facts
        .iter()
        .find(|fact| fact["predicate"] == "inaugurated_year")
        .unwrap();
    assert_eq!(inauguration["subject_entity_id"], facility["id"]);
    assert_eq!(inauguration["value"], "1959");
    let capacity = facts
        .iter()
        .find(|fact| fact["predicate"] == "capacity")
        .unwrap();
    assert_eq!(capacity["subject_entity_id"], facility["id"]);
    assert_eq!(capacity["normalized_value"], "10000");
    assert_eq!(capacity["operator"], ">");
    assert_eq!(capacity["unit"], "seats");
    for predicate in ["owned_by", "located_in"] {
        let linked = facts
            .iter()
            .find(|fact| fact["predicate"] == predicate)
            .unwrap();
        assert_eq!(linked["subject_entity_id"], facility["id"]);
        assert_eq!(linked["type"], "ENTITY");
        assert!(linked["target_entity_id"].as_str().is_some());
    }
    let asserted = scene["relations"]["asserted_relations"].as_array().unwrap();
    assert_eq!(
        asserted.len(),
        2,
        "only explicit ownership/location edges: {asserted:?}"
    );
    assert!(asserted
        .iter()
        .all(|relation| relation["status"] == "Asserted"));
    for fact in facts {
        let start = fact["start_byte"].as_u64().unwrap() as usize;
        let end = fact["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), fact["value"].as_str());
        let evidence_start = fact["evidence_start_byte"].as_u64().unwrap() as usize;
        let evidence_end = fact["evidence_end_byte"].as_u64().unwrap() as usize;
        assert_eq!(
            text.get(evidence_start..evidence_end),
            fact["evidence"].as_str()
        );
    }
}

#[test]
fn explicit_negated_ownership_is_not_reported_as_a_positive_relation() {
    let text = "Stadio Aurora non è di proprietà della famiglia Rossi e si trova a Torino.";
    let value = manifest_value(vec![scene("proprieta-negata", text)]);
    let scene = &value["scenes"][0];
    let relations = scene["relations"]["asserted_relations"].as_array().unwrap();
    assert_eq!(relations.len(), 2, "both explicit assertions are preserved");
    let ownership = relations
        .iter()
        .find(|edge| edge["predicate"] == "ownership")
        .unwrap();
    let location = relations
        .iter()
        .find(|edge| edge["predicate"] == "location")
        .unwrap();
    assert_eq!(ownership["status"], "Negated");
    assert_eq!(location["status"], "Asserted");
}

#[test]
fn unrelated_dates_and_values_do_not_create_a_semantic_group() {
    let text = "La scheda cita il 1950 accanto al nome di Marco Rossi e riporta 18%. Il fascicolo menziona il 1974 e il valore di 42%.";
    let value = manifest_value(vec![scene("non-collegati", text)]);
    let scene = &value["scenes"][0];
    assert!(scene["semantic_groups"].as_array().unwrap().is_empty());
    assert!(proposals_of(scene, "chart").is_empty());
}

#[test]
fn facility_facts_survive_anaphoric_sentences_without_duplicating_capacity() {
    let text = "Lo Stadio Aurora è un impianto sportivo di Torino. Lo Stadio Aurora fu inaugurato nel 1959. La capienza è di oltre 10.000 posti e l'impianto è di proprietà della famiglia Rossi.";
    let value = manifest_value(vec![scene("stadio-anaphora", text)]);
    let scene = &value["scenes"][0];
    let facility_id = scene["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["type"] == "FACILITY")
        .and_then(|entity| entity["id"].as_str())
        .expect("facility identity");
    let facts = scene["entity_facts"].as_array().unwrap();
    // The capacity sentence does not repeat the stadium name: a scene with a
    // single facility still attributes its facts to that facility.
    let capacities: Vec<&Value> = facts
        .iter()
        .filter(|fact| fact["predicate"] == "capacity")
        .collect();
    assert_eq!(
        capacities.len(),
        1,
        "overlapping numeric spans must yield one capacity fact: {facts:?}"
    );
    assert_eq!(capacities[0]["unit"], "seats");
    assert_eq!(capacities[0]["operator"], ">");
    for predicate in ["inaugurated_year", "owned_by", "located_in"] {
        let fact = facts
            .iter()
            .find(|fact| fact["predicate"] == predicate)
            .unwrap_or_else(|| panic!("missing {predicate}: {facts:?}"));
        assert_eq!(fact["subject_entity_id"], facility_id);
    }
    let location = facts
        .iter()
        .find(|fact| fact["predicate"] == "located_in")
        .unwrap();
    assert_eq!(location["value"], "Torino");
    assert!(location["target_entity_id"].as_str().is_some());
}

#[test]
fn entity_facts_carry_importance_and_visual_concept_ids_deterministically() {
    let text = "Il Tribunale di Torino ha assolto Mario Rossi nel 2009. Mario Rossi è stato assolto in appello.";
    let first = manifest_value(vec![scene("fatti-importanza", text)]);
    let second = manifest_value(vec![scene("fatti-importanza", text)]);
    assert_eq!(first, second, "same input must give the same manifest");

    let facts = first["scenes"][0]["entity_facts"].as_array().unwrap();
    assert!(!facts.is_empty(), "the acquittal must produce a fact");
    for fact in facts {
        let importance = fact["importance"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&importance), "importance in [0,1]");
        let ids = fact["visual_concept_ids"].as_array().unwrap();
        assert_eq!(ids[0], fact["subject_entity_id"], "subject leads the ids");
        if let Some(target) = fact["target_entity_id"].as_str() {
            assert!(ids.iter().any(|id| id == target), "target id is bound");
        }
    }
    let outcome = facts
        .iter()
        .find(|fact| fact["normalized_value"] == "acquitted")
        .expect("acquitted outcome fact");
    assert_eq!(outcome["importance"].as_f64().unwrap(), 0.9_f32 as f64);
}

#[test]
fn co_evidenced_facility_facts_keep_a_faithful_extractive_micro_summary() {
    let text = "Stadio Aurora, inaugurato nel 1961 e situato a Torino, aveva una capienza di 18.000 posti.";
    let value = manifest_value(vec![scene("micro-facility", text)]);
    let summaries = value["scenes"][0]["micro_summaries"]
        .as_array()
        .expect("micro summaries array");
    let summary = summaries
        .iter()
        .find(|summary| {
            summary["text"] == text
                && summary["evidence"] == text
                && summary["evidence_fact_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.len() >= 2)
        })
        .expect("co-evidenced facts stay together in a grounded summary");
    let start = summary["start_byte"].as_u64().unwrap() as usize;
    let end = summary["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&text[start..end], summary["evidence"].as_str().unwrap());
}
