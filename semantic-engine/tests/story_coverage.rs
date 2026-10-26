//! Story graph, visual coverage and deterministic replay.

mod common;

use common::*;
#[test]
fn story_graph_joins_one_identity_and_its_facts_across_distant_scenes() {
    let inputs = vec![
        scene("opening", "Alberto Stasi è il primo nome del procedimento."),
        scene("middle", "La città segue il caso con attenzione."),
        scene(
            "finale",
            "Il 17 dicembre 2009 Alberto Stasi fu assolto in primo grado.",
        ),
    ];
    let value = manifest_value(inputs);
    let graph = &value["story_graph"];
    let nodes = graph["nodes"].as_array().expect("nodes array");
    let identity = nodes
        .iter()
        .find(|node| node["kind"] == "identity" && node["label"] == "Alberto Stasi")
        .expect("the person is a story node");
    assert_eq!(
        identity["scene_ids"].as_array().unwrap().len(),
        2,
        "the person is grounded in two scenes: {identity}"
    );
    assert_eq!(identity["scene_id"], "opening", "anchor is the first scene");

    let edges = graph["edges"].as_array().expect("edges array");
    let about = edges
        .iter()
        .find(|edge| edge["kind"] == "about" && edge["from_id"] == identity["id"])
        .expect("the identity is linked to its fact");
    assert_eq!(
        about["cross_scene"], true,
        "the fact lives in another scene"
    );
    assert_eq!(
        about["evidence"],
        "Il 17 dicembre 2009 Alberto Stasi fu assolto in primo grado."
    );
    // Dates and percentages never become story nodes of their own.
    assert!(nodes.iter().all(|node| {
        let id = node["id"].as_str().unwrap();
        !id.starts_with("identity:date_") && !id.starts_with("identity:number_")
    }));
    // A single-scene mention with no fact is not a story node.
    let linked: Vec<&str> = edges
        .iter()
        .filter_map(|edge| edge["from_id"].as_str())
        .collect();
    for node in nodes.iter().filter(|node| node["kind"] == "identity") {
        let crosses = node["scene_ids"].as_array().unwrap().len() >= 2;
        assert!(
            crosses || linked.contains(&node["id"].as_str().unwrap()),
            "isolated single-scene identity in the graph: {node}"
        );
    }
}

#[test]
fn story_graph_orders_one_identity_dated_facts_across_scenes() {
    let inputs = vec![
        scene("early", "Il 17 dicembre 2009 Alberto Stasi fu assolto."),
        scene(
            "late",
            "Nel 2015 Alberto Stasi fu condannato in prima istanza.",
        ),
    ];
    let value = manifest_value(inputs);
    let edges = value["story_graph"]["edges"].as_array().unwrap();
    let chronological = edges
        .iter()
        .filter(|edge| edge["kind"] == "chronological")
        .collect::<Vec<_>>();
    assert_eq!(chronological.len(), 1, "one ordered pair: {edges:?}");
    assert_eq!(chronological[0]["cross_scene"], true);
    assert!(chronological[0]["from_id"]
        .as_str()
        .unwrap()
        .contains("acquitted"));
    assert!(chronological[0]["to_id"]
        .as_str()
        .unwrap()
        .contains("convicted"));
}

#[test]
fn story_graph_stays_empty_without_a_cross_scene_link() {
    let value = manifest_value(vec![scene(
        "solo",
        "Il 17 dicembre 2009 Alberto Stasi fu assolto.",
    )]);
    let edges = value["story_graph"]["edges"].as_array().unwrap();
    assert!(
        edges.iter().all(|edge| edge["cross_scene"] == false),
        "a single scene cannot produce a cross-scene edge: {edges:?}"
    );
    assert!(
        edges.iter().all(|edge| edge["kind"] != "chronological"),
        "one scene already has scene-local chronology groups"
    );
}

#[test]
fn visual_coverage_marks_facts_without_a_representation() {
    let text = "Lo Stadio Aurora è stato inaugurato nel 1959 e ha una capienza di 10.000 posti.";
    let value = manifest_value(vec![scene("stadio", text)]);
    let coverage = &value["visual_coverage"];
    let total = coverage["total_facts"].as_u64().unwrap();
    assert!(
        total >= 2,
        "inauguration and capacity are facts: {coverage}"
    );
    // The dated inauguration is presented by the one-event date card; the
    // capacity (a measurement with no date) stays uncovered, because a
    // searchable name or a surrounding sentence is not a representation.
    assert_eq!(
        coverage["covered_facts"], 1,
        "only the dated fact is represented: {coverage}"
    );
    let facts = coverage["scenes"][0]["facts"].as_array().unwrap();
    for fact in facts {
        let start = fact["start_byte"].as_u64().unwrap() as usize;
        let end = fact["end_byte"].as_u64().unwrap() as usize;
        assert!(
            text.get(start..end).is_some(),
            "the fact value is a source slice"
        );
        if fact["predicate"] == "inaugurated_year" {
            assert_eq!(fact["status"], "covered");
            assert_eq!(fact["proposal_types"][0], "timeline");
        } else {
            assert_eq!(fact["status"], "uncovered");
            assert!(fact["proposal_types"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn visual_coverage_counts_a_charted_value_as_covered() {
    let text = "La capienza è del 42% contro il 24% della struttura precedente.";
    let value = manifest_value(vec![scene("percentuali", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");
    assert_eq!(charts.len(), 1, "an explicit comparison makes a chart");
    assert!(
        value["visual_coverage"]["covered_facts"]
            .as_u64()
            .unwrap_or(0)
            > 0
            || value["visual_coverage"]["total_facts"] == 0
    );
}

#[test]
fn temporal_expressions_without_a_fact_are_still_recovered() {
    let text = "Chiara Poggi viene uccisa il tredici agosto duemilasette. \
                Nel duemilaventicinque la Procura riapre il caso. \
                Nel duemilanove Alberto Stasi fu assolto.";
    let value = manifest_value(vec![scene("date-senza-fatto", text)]);
    let scene_manifest = &value["scenes"][0];

    let normalized: Vec<&str> = scene_manifest["temporal_facts"]
        .as_array()
        .expect("temporal facts array")
        .iter()
        .filter_map(|fact| fact["normalized_date"].as_str())
        .collect();
    for expected in ["2007-08-13", "2025", "2009"] {
        assert!(
            normalized.contains(&expected),
            "unattached date {expected} must still be recovered: {normalized:?}"
        );
    }

    let facts = scene_manifest["entity_facts"].as_array().unwrap();
    assert_eq!(
        facts.len(),
        1,
        "only the acquittal is a fact; the other two dates carry no entity fact"
    );
    assert_eq!(facts[0]["normalized_date"], "2009");
}
