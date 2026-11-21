//! NDJSON protocol surface and request validation.

mod common;

use common::*;
use semantic_engine::{analyze_script, protocol::handle_line};
use serde_json::{json, Value};
#[test]
fn replay_of_the_same_script_gives_identical_manifest_and_fingerprint() {
    let scenes = vec![
        scene(
            "a",
            "Alberto Stasi e Andrea Sempio sono due procedimenti distinti.",
        ),
        scene("b", "Il 42% contro il 18% è nel fascicolo."),
    ];
    let first = serde_json::to_value(analyze_script(&request(scenes.clone())).unwrap()).unwrap();
    let second = serde_json::to_value(analyze_script(&request(scenes)).unwrap()).unwrap();

    assert_eq!(first, second);
    assert_eq!(first["schema_version"], "semantic_manifest.v3");
    assert_eq!(first["fingerprint"].as_str().unwrap().len(), 16);
    assert_eq!(first["analyzers"]["visualner"], "visualner");
}

#[test]
fn place_links_are_projected_on_the_scene_manifest() {
    let value = manifest_value_for(
        "en",
        vec![scene("places", "The capital of France is Paris.")],
    );
    let places = value["scenes"][0]["places"].as_array().unwrap();
    assert!(places.iter().any(|place| {
        place["name"] == "France"
            && place["link_status"] == "linked"
            && place["kb_id"] == "wd:Q142"
            && place["lat"].is_number()
            && place["lon"].is_number()
    }));
    for place in places {
        let start = place["start_byte"].as_u64().unwrap() as usize;
        let end = place["end_byte"].as_u64().unwrap() as usize;
        let text = "The capital of France is Paris.";
        assert_eq!(&text[start..end], place["evidence"].as_str().unwrap());
    }
}

#[test]
fn spanish_person_role_is_projected_from_the_third_role_lexicon() {
    let text = "María García, alcaldesa de Madrid, presentó el presupuesto.";
    let value = manifest_value_for("es", vec![scene("spanish-role", text)]);
    assert_eq!(value["analyzers"]["roles"], "person_roles.v4.all_11");
    let roles = value["scenes"][0]["person_roles"].as_array().unwrap();
    let role = roles
        .iter()
        .find(|role| role["role_code"] == "mayor")
        .expect("Spanish mayor role");
    assert_eq!(role["role_surface"], "alcaldesa");
    assert_eq!(role["gender"], "feminine");
    let start = role["start_byte"].as_u64().unwrap() as usize;
    let end = role["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&text[start..end], "alcaldesa");
}

#[test]
fn explicit_route_is_projected_with_grounded_order_and_distance() {
    let text = "They drove from Paris to Rome.";
    let value = manifest_value_for("en", vec![scene("route", text)]);
    let routes = value["scenes"][0]["routes"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    let route = &routes[0];
    assert_eq!(route["mode"], "road");
    assert_eq!(route["status"], "asserted");
    assert_eq!(route["stops"][0]["role"], "origin");
    assert_eq!(route["stops"][1]["role"], "destination");
    assert!(route["straight_line_km"].as_f64().unwrap() > 100.0);
    let start = route["start_byte"].as_u64().unwrap() as usize;
    let end = route["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&text[start..end], route["evidence"].as_str().unwrap());
}

#[test]
fn consecutive_cross_scene_route_hops_merge_with_scene_qualified_stops() {
    let value = manifest_value_for(
        "en",
        vec![
            scene("leg-1", "Maya Chen drove from Paris to Rome."),
            scene("leg-2", "Maya Chen drove from Rome to Berlin."),
        ],
    );
    assert_eq!(
        value["analyzers"]["routes"],
        "place_routes.v3.cross_scene_hops"
    );
    let routes = value["cross_scene_routes"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    let route = &routes[0];
    assert_eq!(route["status"], "asserted");
    assert_eq!(route["mode"], "road");
    assert_eq!(route["stops"].as_array().unwrap().len(), 3);
    assert_eq!(route["stops"][0]["scene_id"], "leg-1");
    assert_eq!(route["stops"][0]["role"], "origin");
    assert_eq!(route["stops"][1]["scene_id"], "leg-1");
    assert_eq!(route["stops"][1]["role"], "waypoint");
    assert_eq!(route["stops"][2]["scene_id"], "leg-2");
    assert_eq!(route["stops"][2]["role"], "destination");
    assert_eq!(route["evidence"].as_array().unwrap().len(), 2);
    assert!(route["straight_line_km"].as_f64().unwrap() > 100.0);
}

#[test]
fn cross_scene_route_hops_from_different_people_stay_separate() {
    let value = manifest_value_for(
        "en",
        vec![
            scene("leg-1", "Maya Chen drove from Paris to Rome."),
            scene("leg-2", "Ada Lovelace drove from Rome to Berlin."),
        ],
    );
    assert!(value
        .get("cross_scene_routes")
        .and_then(serde_json::Value::as_array)
        .is_none_or(Vec::is_empty));
    let first = &value["scenes"][0]["routes"][0];
    let second = &value["scenes"][1]["routes"][0];
    assert_ne!(first["subject_id"], second["subject_id"]);
    assert_eq!(first["stops"].as_array().unwrap().len(), 2);
    assert_eq!(second["stops"].as_array().unwrap().len(), 2);
}

#[test]
fn event_frames_are_projected_from_grounded_script_analysis() {
    let value = manifest_value_for(
        "en",
        vec![scene("event", "Maya Chen was arrested in Paris in 2019.")],
    );
    assert_eq!(
        value["analyzers"]["events"],
        "event_frames.v3.udpipe_lemma_grounding"
    );
    let events = value["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event["event_type"], "arrest");
    assert_eq!(event["trigger"], "arrested");
    assert_eq!(event["participants"][0]["role"], "patient");
    assert!(!event["participants"][0]["entity_id"]
        .as_str()
        .unwrap()
        .is_empty());
    assert_eq!(event["scene_ids"][0], "event");
}

#[test]
fn spanish_event_triggers_project_into_the_script_manifest() {
    let value = manifest_value_for(
        "es",
        vec![scene("birth", "Maya Chen nació en Madrid en 1985.")],
    );
    let events = value["events"].as_array().unwrap();
    assert!(events.iter().any(|event| {
        event["event_type"] == "birth"
            && event["participants"]
                .as_array()
                .is_some_and(|participants| !participants.is_empty())
            && event["scene_ids"] == json!(["birth"])
    }));
}

#[test]
fn repeated_dated_event_merges_across_scenes_for_the_same_person() {
    let text = "Maya Chen was arrested in Paris in 2019.";
    let value = manifest_value_for("en", vec![scene("first", text), scene("second", text)]);
    let events = value["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["scene_ids"],
        serde_json::json!(["first", "second"])
    );
    assert_eq!(events[0]["evidence"].as_array().unwrap().len(), 2);
}

#[test]
fn identity_link_candidates_expose_a_prior_from_a_resolved_discourse_thread() {
    let value = manifest_value_for(
        "en",
        vec![
            scene("state", "Washington is in the Pacific Northwest."),
            scene("return", "Washington was discussed again."),
        ],
    );
    let entity = value["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["canonical_surface"] == "Washington")
        .expect("Washington identity should be linked");
    assert_eq!(entity["link_status"], "linked");
    assert_eq!(entity["canonical_id"], "wd:Q1223");
    let candidates = entity["link_candidates"].as_array().unwrap();
    let planet = candidates
        .iter()
        .find(|candidate| candidate["kb_id"] == "wd:Q1223")
        .unwrap();
    let prior = planet["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|feature| feature["name"] == "discourse_prior")
        .unwrap_or_else(|| {
            panic!(
                "candidate explanation should include its thread prior; discourse={} identities={}",
                value["discourse"],
                value["scenes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|scene| scene["identities"].clone())
                    .collect::<Vec<_>>()
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        });
    assert_eq!(prior["terms"], json!(["Pacific Northwest"]));
    let span = prior["spans"][0].clone();
    assert_eq!(span["scene_id"], "state");
    assert_eq!(
        &"Washington is in the Pacific Northwest."[span["start_byte"].as_u64().unwrap() as usize
            ..span["end_byte"].as_u64().unwrap() as usize],
        "Pacific Northwest"
    );
}

#[test]
fn explicit_age_fact_is_projected_with_a_conservative_birth_year() {
    let text = "In 1998, Maya Chen was aged 30.";
    let value = manifest_value_for("en", vec![scene("age", text)]);
    assert_eq!(
        value["analyzers"]["age_facts"],
        "person_age_facts.v2.localized_dated_age"
    );
    let projected = &value["scenes"][0]["age_facts"];
    assert_eq!(projected.as_array().unwrap().len(), 1);
    let fact = &projected[0];
    assert_eq!(fact["age"], 30);
    assert_eq!(fact["as_of"], "1998");
    assert_eq!(fact["derived_birth_year"], 1968);
    let start = fact["start_byte"].as_u64().unwrap() as usize;
    let end = fact["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(&text[start..end], "30");
}

#[test]
fn spelled_out_person_ages_are_projected_in_all_supported_languages() {
    let examples = [
        ("en", "Maya Chen was thirty years old.", "thirty"),
        ("it", "Maya Chen aveva trenta anni.", "trenta"),
        ("es", "Maya Chen tenía treinta años.", "treinta"),
        ("fr", "Maya Chen avait trente ans.", "trente"),
        ("pt", "Maya Chen tinha trinta anos.", "trinta"),
        ("de", "Maya Chen war dreißig Jahre alt.", "dreißig"),
        ("nl", "Maya Chen was dertig jaar.", "dertig"),
        ("pl", "Maya Chen miała trzydzieści lat.", "trzydzieści"),
        ("ru", "Maya Chen, ему было тридцать лет.", "тридцать"),
        ("tr", "Maya Chen otuz yaşındaydı.", "otuz"),
        ("id", "Maya Chen berusia tiga puluh tahun.", "tiga puluh"),
    ];

    for (language, text, number_word) in examples {
        let value = manifest_value_for(language, vec![scene("spelled-age", text)]);
        let age_facts = value["scenes"][0]["age_facts"]
            .as_array()
            .unwrap_or_else(|| panic!("{language}: {text}; manifest={value}"));
        assert_eq!(age_facts.len(), 1, "{language}: {text}; value={value}");
        assert_eq!(age_facts[0]["age"], 30, "{language}: {text}");
        let start = age_facts[0]["start_byte"].as_u64().unwrap() as usize;
        let end = age_facts[0]["end_byte"].as_u64().unwrap() as usize;
        assert!(
            text[start..end].starts_with(number_word),
            "{language}: span {:?} should start with {number_word:?}",
            &text[start..end]
        );
        assert_eq!(age_facts[0]["derived_birth_year"], serde_json::Value::Null);
    }
}

#[test]
fn explicit_person_relation_is_projected_with_direction_and_status() {
    let text = "Maya Chen is the daughter of John Chen.";
    let value = manifest_value_for("en", vec![scene("relation", text)]);
    let relations = value["scenes"][0]["person_relations"].as_array().unwrap();
    assert_eq!(relations.len(), 1);
    let relation = &relations[0];
    assert_eq!(relation["kind"], "child_of");
    assert_eq!(relation["status"], "asserted");
    assert_ne!(relation["from_id"], relation["to_id"]);
    assert_eq!(relation["evidence"], text);
}

#[test]
fn empty_scene_is_valid_and_empty() {
    let value = manifest_value(vec![scene("vuota", "")]);
    let scene = &value["scenes"][0];
    assert_eq!(scene["status"], "valid");
    assert!(scene["visual_proposals"].as_array().unwrap().is_empty());
}

#[test]
fn missing_language_and_duplicate_scene_ids_are_rejected() {
    let mut no_language = request(vec![scene("a", "Testo.")]);
    no_language.language = "  ".to_string();
    assert!(analyze_script(&no_language).is_err());

    let duplicate = request(vec![scene("a", "Testo."), scene("a", "Altro testo.")]);
    assert!(analyze_script(&duplicate)
        .unwrap_err()
        .contains("duplicate scene_id"));
}

#[test]
fn ndjson_protocol_serves_health_analysis_and_errors() {
    let health: Value = serde_json::from_str(&handle_line(r#"{"operation":"health"}"#)).unwrap();
    assert_eq!(health["ok"], true);
    assert_eq!(health["schema_version"], "semantic_manifest.v3");
    assert_eq!(health["entity_index_version"], "entity_index.v1");
    assert!(health["entity_index_entries"].as_u64().unwrap() >= 8);

    let request = json!({
        "operation": "analyze_script",
        "language": "it",
        "scenes": [{"scene_id": "s1", "text": "Il 42% contro il 18%."}]
    });
    let response: Value = serde_json::from_str(&handle_line(&request.to_string())).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["manifest"]["scenes"][0]["scene_id"], "s1");

    let unknown: Value = serde_json::from_str(&handle_line(r#"{"operation":"render"}"#)).unwrap();
    assert_eq!(unknown["ok"], false);

    let broken: Value = serde_json::from_str(&handle_line("not json")).unwrap();
    assert_eq!(broken["ok"], false);
}
