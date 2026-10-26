//! Chronology, process phases, completion sequences and timeline replay.

mod common;

use common::*;
use serde_json::Value;

#[test]
fn four_grounded_stadium_phases_form_one_chronological_group() {
    let text = "Il progetto dello stadio fu progettato nel 1950. La costruzione iniziò nel 1952. L'impianto fu inaugurato nel 1959. Lo stadio venne ampliato nel 1974.";
    let value = manifest_value(vec![scene("stadio-aurora", text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "one grounded sequence expected: {value}");
    assert_eq!(groups[0]["group_type"], "chronological");
    let members = groups[0]["members"].as_array().unwrap();
    assert_eq!(
        members.len(),
        4,
        "all four source phases must be linked: {members:?}"
    );
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
        assert!(member["temporal_surface"].as_str().is_some());
    }
}

#[test]
fn explicit_process_phases_link_only_when_the_same_entity_is_grounded() {
    let text = "Lo Stadio Aurora fu progettato. La costruzione dello Stadio Aurora iniziò. Lo Stadio Aurora fu inaugurato. Lo Stadio Aurora venne ampliato.";
    let value = manifest_value(vec![scene("processo-stadio", text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let process: Vec<_> = groups
        .iter()
        .filter(|group| group["group_type"] == "process")
        .collect();
    assert_eq!(process.len(), 1, "one grounded process expected: {value}");
    let members = process[0]["members"].as_array().unwrap();
    assert_eq!(members.len(), 4, "all explicit phases must be retained");
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
        assert!(member["evidence"]
            .as_str()
            .unwrap()
            .contains("Stadio Aurora"));
    }
}

#[test]
fn uncertain_or_negated_phases_do_not_complete_a_process_group() {
    let text = "Lo Stadio Aurora fu progettato. La costruzione dello Stadio Aurora non iniziò. Lo Stadio Aurora fu inaugurato. Forse lo Stadio Aurora venne ampliato.";
    let value = manifest_value(vec![scene("processo-incerto", text)]);
    assert!(value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["group_type"] != "process"));
}

#[test]
fn out_of_order_process_phases_are_not_joined() {
    let text = "Lo Stadio Aurora fu progettato. Lo Stadio Aurora venne ampliato. La costruzione dello Stadio Aurora iniziò. Lo Stadio Aurora fu inaugurato.";
    let value = manifest_value(vec![scene("processo-fuori-ordine", text)]);
    assert!(value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["group_type"] != "process"));
}

#[test]
fn process_phases_link_across_scenes_with_local_grounding() {
    let inputs = vec![
        scene("planning", "Lo Stadio Aurora fu progettato."),
        scene("construction", "La costruzione dello Stadio Aurora iniziò."),
        scene("opening", "Lo Stadio Aurora fu inaugurato."),
    ];
    let value = manifest_value(inputs.clone());
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let process = groups
        .iter()
        .find(|group| group["group_type"] == "process")
        .expect("one cross-scene process should be attached to its first scene");
    let members = process["members"].as_array().unwrap();
    assert_eq!(members.len(), 3);
    for (member, input) in members.iter().zip(inputs) {
        assert_eq!(member["scene_id"], input.scene_id);
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(input.text.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn process_phases_of_different_entities_are_not_joined_across_scenes() {
    let inputs = vec![
        scene("planning", "Lo Stadio Aurora fu progettato."),
        scene("construction", "La costruzione del Ponte Verdi iniziò."),
        scene("opening", "Il Teatro Nuovo fu inaugurato."),
    ];
    let value = manifest_value(inputs);
    for scene in value["scenes"].as_array().unwrap() {
        assert!(scene["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["group_type"] != "process"));
    }
}

#[test]
fn completion_phase_completes_a_four_step_construction_sequence() {
    let text = "La prima fase riguarda la progettazione dello Stadio Aurora. La seconda fase avvia la costruzione dello Stadio Aurora. La terza fase completa le tribune dello Stadio Aurora. La quarta fase amplia la capienza dello Stadio Aurora.";
    let value = manifest_value(vec![scene("quattro-fasi", text)]);
    let process: Vec<&Value> = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|group| group["group_type"] == "process")
        .collect();
    assert_eq!(process.len(), 1, "one process group expected: {value}");
    let members = process[0]["members"].as_array().unwrap();
    assert_eq!(
        members.len(),
        4,
        "all four phases must be linked: {members:?}"
    );
    for (index, member) in members.iter().enumerate() {
        assert_eq!(member["sentence_index"], index);
        assert!(member["evidence"]
            .as_str()
            .unwrap()
            .contains("Stadio Aurora"));
    }
}

#[test]
fn corrected_chronology_yields_the_documented_sequence() {
    // The same passage as the 3,127-word Garlasco fixture, with the trial years
    // corrected to the documented ones (2009, 2011, 2013, 2014, 2015).
    let text = "Passano gli anni e il caso prende una direzione precisa. Alberto Stasi viene rinviato a giudizio e il suo processo si sviluppa in un percorso che durerà diversi anni. Nel duemilanove il giudice di primo grado pronuncia una prima assoluzione, che nel duemilaundici viene confermata nel giudizio d'appello. Per molti sembra la fine di una vicenda, ma non è così. Nel duemilatredici la Cassazione interviene annullando con rinvio la sentenza, e il caso torna in aula con un nuovo giudizio d'appello che si conclude nel duemilaquattordici con una condanna a sedici anni di reclusione. Nel duemilaquindici questa condanna diventa definitiva.";
    let value = manifest_value(vec![scene("processo", text)]);
    let scene_manifest = &value["scenes"][0];

    let mut facts: Vec<(String, String)> = scene_manifest["entity_facts"]
        .as_array()
        .expect("facts array")
        .iter()
        .filter_map(|fact| {
            Some((
                fact["normalized_date"].as_str()?.to_string(),
                fact["normalized_value"].as_str()?.to_string(),
            ))
        })
        .collect();
    facts.sort();
    assert_eq!(
        facts,
        vec![
            ("2009".to_string(), "acquitted".to_string()),
            ("2011".to_string(), "acquittal_confirmed".to_string()),
            ("2013".to_string(), "annulled_with_remand".to_string()),
            ("2014".to_string(), "convicted".to_string()),
            ("2015".to_string(), "final".to_string()),
        ],
        "the engine must reproduce the script's own timeline, not a guessed one"
    );

    let group = scene_manifest["semantic_groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .find(|group| group["group_type"] == "step_sequence")
        .expect("five dated outcomes form one ordered sequence");
    let members = group["members"].as_array().expect("members array");
    let dates: Vec<&str> = members
        .iter()
        .map(|member| member["normalized_date"].as_str().unwrap())
        .collect();
    assert_eq!(dates, ["2009", "2011", "2013", "2014", "2015"]);
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(
            text.get(start..end),
            member["evidence"].as_str(),
            "every step keeps its verbatim source sentence"
        );
    }
    assert!(
        members[0]["text"]
            .as_str()
            .unwrap()
            .contains("assoluzione nel 2009"),
        "the first step is the 2009 acquittal: {members:?}"
    );
    assert!(
        members[4]["text"]
            .as_str()
            .unwrap()
            .contains("definitiva nel 2015"),
        "the last step is the 2015 finality: {members:?}"
    );
}

#[test]
fn a_complete_dated_sequence_is_one_timeline_and_closes_coverage() {
    let text = "Passano gli anni e il caso prende una direzione precisa. Alberto Stasi viene \
                rinviato a giudizio e il suo processo si sviluppa in un percorso che durerà \
                diversi anni. Nel duemilanove il giudice di primo grado pronuncia una prima \
                assoluzione, che nel duemilaundici viene confermata nel giudizio d'appello. \
                Nel duemilatredici la Cassazione interviene annullando con rinvio la sentenza, \
                e il caso torna in aula con un nuovo giudizio d'appello che si conclude nel \
                duemilaquattordici con una condanna a sedici anni di reclusione. \
                Nel duemilaquindici questa condanna diventa definitiva.";
    let value = manifest_value(vec![scene("sequenza", text)]);
    let scene_manifest = &value["scenes"][0];
    let sequences: Vec<&Value> = scene_manifest["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|group| group["group_type"] == "step_sequence")
        .collect();
    assert_eq!(
        sequences.len(),
        1,
        "one complete sequence expected: {value}"
    );
    let members = sequences[0]["members"].as_array().unwrap();
    assert_eq!(members.len(), 5, "five steps expected: {members:?}");
    let dates: Vec<&str> = members
        .iter()
        .filter_map(|member| member["normalized_date"].as_str())
        .collect();
    let mut unique = dates.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(dates.len(), unique.len(), "no duplicated step: {dates:?}");

    let timelines: Vec<&Value> = scene_manifest["visual_proposals"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|proposal| proposal["proposal_type"] == "timeline")
        .collect();
    assert_eq!(
        timelines.len(),
        1,
        "one timeline per sequence: {timelines:?}"
    );
    let end = timelines[0]["end_byte"].as_u64().unwrap();
    let last_step = members
        .iter()
        .find(|member| member["normalized_date"] == "2015")
        .expect("the 2015 step exists");
    assert!(
        last_step["end_byte"].as_u64().unwrap() <= end,
        "the timeline must reach the last step: {last_step}"
    );

    let coverage = &value["visual_coverage"];
    assert_eq!(
        coverage["covered_facts"], coverage["total_facts"],
        "every fact of a complete timeline is covered: {coverage}"
    );
}
