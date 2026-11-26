//! TICKET-45: ground the six new visual recipes only on extracted facts.

mod common;

use common::{manifest_value_for, scene};
use serde_json::Value;

fn candidates(language: &str, text: &str) -> Vec<Value> {
    let value = manifest_value_for(language, vec![scene("recipe", text)]);
    value["visual_consolidation"]["production_candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn recipe<'a>(candidates: &'a [Value], name: &str) -> Vec<&'a Value> {
    candidates
        .iter()
        .filter(|candidate| candidate["recipe"] == name)
        .collect()
}

fn assert_grounded(source: &str, candidate: &Value) {
    assert!(candidate["visual_payload"].is_object(), "{candidate:?}");
    let start = candidate["start_byte"].as_u64().unwrap() as usize;
    let end = candidate["end_byte"].as_u64().unwrap() as usize;
    let evidence = candidate["evidence"].as_str().unwrap();
    assert_eq!(&source[start..end], evidence, "{candidate:?}");
    // Coverage crosses the wire as canonical fact ids only: a recipe grounded
    // on a raw artifact (a quantity, a route, a role) contributes no coverage
    // rather than an id the consumer cannot resolve.
    for id in candidate["covered_fact_ids"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        assert!(
            id.starts_with("cf_"),
            "non-canonical coverage {id} in {candidate:?}"
        );
    }
}

#[test]
fn relationship_graph_grounds_two_people_and_their_edge() {
    let source = "Maya Chen is the daughter of John Chen.";
    let candidates = candidates("en", source);
    let graphs = recipe(&candidates, "relationship_graph");
    assert_eq!(graphs.len(), 1, "{candidates:?}");
    assert_grounded(source, graphs[0]);
    assert_eq!(graphs[0]["suggested_visual"], "investigation_board");
    assert!(graphs[0]["visual_payload"]["edges"]
        .as_array()
        .is_some_and(|edges| edges.len() == 1));
}

#[test]
fn role_lower_third_grounds_person_role_and_organization() {
    let source = "María García, alcaldesa de Madrid, presentó el presupuesto.";
    let candidates = candidates("es", source);
    let roles = recipe(&candidates, "role_lower_third");
    assert_eq!(roles.len(), 1, "{candidates:?}");
    assert_grounded(source, roles[0]);
    assert_eq!(roles[0]["visual_payload"]["role_code"], "mayor");
    assert_eq!(roles[0]["visual_payload"]["role_surface"], "alcaldesa");
}

#[test]
fn route_map_requires_two_coordinate_bound_waypoints() {
    let source = "They drove from Paris to Rome.";
    let candidates = candidates("en", source);
    let routes = recipe(&candidates, "route_map");
    assert_eq!(routes.len(), 1, "{candidates:?}");
    assert_grounded(source, routes[0]);
    let stops = routes[0]["visual_payload"]["stops"].as_array().unwrap();
    assert_eq!(stops.len(), 2);
    assert!(stops.iter().all(|stop| stop["lat"].is_number()));
}

#[test]
fn choropleth_requires_two_linked_country_quantities() {
    // One value per sentence: two percentages in the same sentence become a
    // `chart` proposal, and chart-owned spans are deliberately not duplicated
    // as choropleth cells.
    let source = "In France the rate was 12%. In Germany the rate was 18%.";
    let candidates = candidates("en", source);
    let maps = recipe(&candidates, "choropleth_value_map");
    assert_eq!(maps.len(), 1, "{candidates:?}");
    assert_grounded(source, maps[0]);
    assert!(maps[0]["visual_payload"]["items"]
        .as_array()
        .is_some_and(|items| items.len() >= 2));
}

#[test]
fn lifespan_requires_two_distinct_dated_life_events() {
    let source = "Maya Chen was born in 1980 and died in 2020.";
    let candidates = candidates("en", source);
    let timelines = recipe(&candidates, "lifespan_timeline");
    assert_eq!(timelines.len(), 1, "{candidates:?}");
    assert_grounded(source, timelines[0]);
    assert_eq!(timelines[0]["visual_payload"]["person_id"], "person_01");
}

#[test]
fn counter_odometer_requires_one_asserted_subject_bound_kpi() {
    let source = "Maya Chen reported revenue of 12 million dollars in 2023.";
    let candidates = candidates("en", source);
    let counters = recipe(&candidates, "counter_odometer");
    assert_eq!(counters.len(), 1, "{candidates:?}");
    assert_grounded(source, counters[0]);
    assert_eq!(counters[0]["suggested_visual"], "counter_odometer");
}

#[test]
fn missing_slots_fail_closed_for_every_recipe() {
    let cases = [
        ("en", "Maya Chen is the daughter of an unknown man."),
        ("es", "María García presentó el presupuesto."),
        ("en", "They drove to Rome."),
        ("en", "In France the rate was 12%."),
        ("en", "Maya Chen was born in 1980."),
        ("en", "Revenue reached 12 million dollars."),
    ];
    let names = [
        "relationship_graph",
        "role_lower_third",
        "route_map",
        "choropleth_value_map",
        "lifespan_timeline",
        "counter_odometer",
    ];
    for ((language, source), name) in cases.into_iter().zip(names) {
        let candidates = candidates(language, source);
        assert!(
            recipe(&candidates, name).is_empty(),
            "{name} must fail closed for {source:?}: {candidates:?}"
        );
    }
}

#[test]
fn neutral_text_emits_no_visual_recipe() {
    let candidates = candidates("en", "The weather was mild and the room was quiet.");
    for name in [
        "relationship_graph",
        "role_lower_third",
        "route_map",
        "choropleth_value_map",
        "lifespan_timeline",
        "counter_odometer",
    ] {
        assert!(
            recipe(&candidates, name).is_empty(),
            "{name}: {candidates:?}"
        );
    }
}
