//! Entity extraction, canonical identity and co-mention assertions.

mod common;

use common::*;
use semantic_engine::{analyze_script, ScriptRequest};
use serde_json::Value;

#[test]
fn r02_two_persons_in_one_sentence_are_a_co_mention_not_a_relation() {
    let text = "Alberto Stasi e Andrea Sempio sono due procedimenti distinti.";
    let value = manifest_value(vec![scene("due-procedimenti", text)]);
    let scene = &value["scenes"][0];

    assert_eq!(scene["status"], "valid");
    let groups = proposals_of(scene, "multiple_entities");
    assert_eq!(groups.len(), 1, "one co-mention group expected: {scene}");
    let ids = groups[0]["entity_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2, "two canonical ids expected: {ids:?}");
    // Two distinct persons must have two distinct canonical ids.
    assert_ne!(ids[0], ids[1]);
    assert_eq!(groups[0]["suggested_visual"], "side_by_side");
    assert_eq!(groups[0]["status"], "requires_semantic_validation");
    assert!(groups[0]["visual_query_suggestions"]
        .as_array()
        .is_some_and(|queries| !queries.is_empty()));
    assert!(groups[0]["visual_query_suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|query| query != "Alberto Stasi Andrea Sempio"));
    assert!(
        proposals_of(scene, "chart").is_empty(),
        "names must not produce charts"
    );
    // A co-mention must never become an asserted relation.
    assert!(scene["relations"]["asserted_relations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        scene["relations"]["co_mentions"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn same_person_in_two_scenes_keeps_one_canonical_id() {
    let scenes = vec![
        scene("a", "Alberto Stasi è il primo nome."),
        scene("b", "Alberto Stasi torna nel racconto."),
    ];
    let value = manifest_value(scenes);
    let id_a = value["scenes"][0]["entities"][0]["id"].as_str().unwrap();
    let id_b = value["scenes"][1]["entities"][0]["id"].as_str().unwrap();
    assert_eq!(id_a, id_b, "repeated person must keep one canonical id");
}

#[test]
fn distinct_persons_get_distinct_canonical_ids() {
    let text = "Alberto Stasi e Andrea Sempio sono due procedimenti distinti.";
    let value = manifest_value(vec![scene("distinti", text)]);
    let entities = value["scenes"][0]["entities"].as_array().unwrap();
    let stasi = entities
        .iter()
        .find(|e| e["text"].as_str().unwrap().contains("Stasi"))
        .unwrap();
    let sempio = entities
        .iter()
        .find(|e| e["text"].as_str().unwrap().contains("Sempio"))
        .unwrap();
    assert_ne!(
        stasi["id"], sempio["id"],
        "distinct subjects must not merge"
    );
}

#[test]
fn every_entity_and_proposal_is_an_exact_source_slice() {
    let text = "Alberto Stasi e Andrea Sempio sono due procedimenti distinti. Il 42% contro il 18% è citato nel fascicolo.";
    let source_scene = scene("griglia", text);
    let manifest = analyze_script(&request(vec![source_scene])).unwrap();
    let scene = &manifest.scenes[0];

    for entity in &scene.entities {
        assert_eq!(
            text.get(entity.start_byte..entity.end_byte),
            Some(entity.evidence.as_str())
        );
    }
    let value = serde_json::to_value(&manifest).unwrap();
    for proposal in value["scenes"][0]["visual_proposals"].as_array().unwrap() {
        let start = proposal["start_byte"].as_u64().unwrap() as usize;
        let end = proposal["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), proposal["evidence"].as_str());
    }
}

#[test]
fn every_mention_of_eight_people_survives_the_default_entity_budget() {
    // The engine keeps every grounded entity: picking a top-N is the Go
    // planner's job, so eight names must not be truncated to three.
    let text =
        "Alberto, Andrea, Marco, Luca, Giulia, Sara, Paolo e Chiara hanno partecipato al progetto.";
    let value = manifest_value(vec![scene("otto", text)]);
    let names = [
        "Alberto", "Andrea", "Marco", "Luca", "Giulia", "Sara", "Paolo", "Chiara",
    ];
    let entities = value["scenes"][0]["entities"].as_array().unwrap();
    for name in names {
        assert!(
            entities
                .iter()
                .any(|entity| entity["text"].as_str().unwrap().contains(name)),
            "missing {name}: {entities:?}"
        );
    }
    let proposals = value["scenes"][0]["visual_proposals"].as_array().unwrap();
    let entity_cards: Vec<_> = proposals
        .iter()
        .filter(|proposal| proposal["proposal_type"] == "entity_card")
        .collect();
    for name in names {
        assert!(
            entity_cards.iter().any(|proposal| {
                proposal["visual_query_suggestions"]
                    .as_array()
                    .is_some_and(|queries| queries.iter().any(|query| query.as_str() == Some(name)))
            }),
            "missing reusable grounded visual query for {name}: {entity_cards:?}"
        );
    }
}

#[test]
fn nested_location_survives_the_legal_org_upgrade() {
    // "Procura di Pavia" is one ORG span, but Pavia is also a grounded
    // LOCATION. Upgrading the head entity must not consume the nested place.
    let text = "La Procura di Pavia ha formalizzato una seconda conclusione delle indagini.";
    let value = manifest_value(vec![scene("procura-pavia", text)]);
    let entities = value["scenes"][0]["entities"].as_array().unwrap();
    let procura = entities
        .iter()
        .find(|entity| entity["text"] == "Procura di Pavia")
        .unwrap_or_else(|| panic!("missing Procura di Pavia: {entities:?}"));
    assert_eq!(procura["type"], "ORG");
    let pavia = entities
        .iter()
        .find(|entity| entity["text"] == "Pavia")
        .unwrap_or_else(|| panic!("nested Pavia was consumed: {entities:?}"));
    assert_eq!(pavia["type"], "LOCATION");
}

#[test]
fn a_clause_opener_place_survives_script_wide_name_scoring() {
    // "Bezoek Haarlem vandaag." opens with a narration verb. The fused
    // capitalised run must not seed a script-wide person-name token, or the
    // one-token LOCATION is later dropped as a "name fragment" of a person
    // that never existed.
    let manifest = analyze_script(&ScriptRequest {
        language: "nl".to_string(),
        scenes: vec![scene(
            "bezoek-haarlem",
            "Bezoek Haarlem vandaag. Het project begint in Haarlem.",
        )],
        editorial: None,
        entity_safety_cap: 0,
    })
    .expect("analysis must succeed");
    let value = serde_json::to_value(manifest).expect("manifest serializes");
    let entities = value["scenes"][0]["entities"].as_array().unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity["text"] == "Haarlem" && entity["type"] == "LOCATION"),
        "the place must survive the script-wide name scoring: {entities:?}"
    );
    assert!(
        !entities
            .iter()
            .any(|entity| entity["text"] == "Bezoek Haarlem"),
        "the fused clause opener must not be an entity: {entities:?}"
    );
}

#[test]
fn filler_phrases_headings_and_pronouns_never_become_search_proposals() {
    // «Non è così», «Ancora oggi …», «Per questo …», a chapter heading and
    // isolated pronouns are discourse noise: their text stays in the scene
    // inventory, but no search proposal may carry them.
    let text = "CAPITOLO SECONDO — IL PROCESSO. Non è così. Ancora oggi la città \
                ricorda. Per questo il caso resta aperto. Lui. Lei. Quello. \
                Chiara Poggi viveva a Pavia.";
    let value = manifest_value(vec![scene("rumore", text)]);
    let scene = &value["scenes"][0];

    // The inventory keeps the original text.
    let phrases: Vec<&str> = scene["short_phrases"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|phrase| phrase["text"].as_str())
        .collect();
    assert!(
        phrases.iter().any(|phrase| phrase.contains("Non è così")),
        "the filler sentence stays in the inventory: {phrases:?}"
    );
    assert!(
        phrases
            .iter()
            .any(|phrase| phrase.contains("CAPITOLO SECONDO")),
        "the chapter heading stays in the inventory: {phrases:?}"
    );

    // No search proposal may carry a filler phrase or a heading.
    for proposal in scene["visual_proposals"].as_array().unwrap() {
        for query in proposal["visual_query_suggestions"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let surface = query.as_str().unwrap_or_default().to_lowercase();
            for noise in [
                "non è così",
                "ancora oggi",
                "per questo",
                "capitolo secondo",
            ] {
                assert!(
                    !surface.contains(noise),
                    "noise {noise:?} leaked into query {surface:?}"
                );
            }
        }
    }
    // Isolated pronouns never become entities, hence never proposals.
    let entities = scene["entities"].as_array().unwrap();
    for pronoun in ["Lui", "Lei", "Quello"] {
        assert!(
            !entities.iter().any(|entity| entity["text"] == pronoun),
            "{pronoun} must not be an entity: {entities:?}"
        );
    }
    // A named entity in a clean sentence keeps its full widening ladder.
    let pavia = scene["visual_proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|proposal| {
            proposal["proposal_type"] == "entity_search"
                && proposal["visual_query_suggestions"]
                    .as_array()
                    .is_some_and(|queries| queries.iter().any(|query| query == "Pavia"))
        })
        .expect("Pavia keeps its search proposal");
    let queries = pavia["visual_query_suggestions"].as_array().unwrap();
    assert_eq!(queries.len(), 3, "full ladder expected: {queries:?}");
}

#[test]
fn a_bare_surname_is_the_same_person_and_the_same_identity() {
    let value = manifest_value(vec![
        scene(
            "s01",
            "Alberto Stasi aveva ventisei anni. Il 12 dicembre 2015 la condanna divenne definitiva in Cassazione.",
        ),
        scene(
            "s02",
            "La posizione di Stasi resta distinta da quella di Andrea Sempio.",
        ),
    ]);
    let first = &value["scenes"][0];
    let second = &value["scenes"][1];
    let person = first["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["text"] == "Alberto Stasi")
        .expect("full name");
    assert_eq!(person["type"], "PERSON");
    let surname = second["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["text"] == "Stasi")
        .expect("surname mention");
    assert_eq!(
        surname["type"], "PERSON",
        "a known surname must stay a person: {surname:?}"
    );
    let full_identity = first["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|identity| identity["canonical_surface"] == "Alberto Stasi")
        .expect("canonical identity");
    let surname_identities: Vec<&Value> = second["identities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|identity| {
            identity["mentions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mention| mention["scene_id"] == "s02" && mention["surface"] == "Stasi")
        })
        .collect();
    assert_eq!(
        surname_identities.len(),
        1,
        "the surname created its own identity: {surname_identities:?}"
    );
    assert_eq!(
        surname_identities[0]["id"], full_identity["id"],
        "the surname must resolve to the full name's identity"
    );
    // The finality sentence names the court after the marker; the fact still
    // belongs to the convicted person, not to Cassazione.
    let final_fact = first["micro_summaries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|summary| {
            summary["text"]
                .as_str()
                .unwrap_or("")
                .contains("definitiva")
        })
        .expect("final micro summary");
    assert!(
        final_fact["text"].as_str().unwrap_or("").contains("Stasi"),
        "finality attributed to the court: {final_fact:?}"
    );
}
