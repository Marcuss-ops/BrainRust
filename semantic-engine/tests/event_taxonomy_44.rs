//! TICKET-44: the nine documentary/economic/science event types must ground
//! in all 11 languages and preserve asserted/negated/uncertain status.

mod common;

use common::{manifest_value_for, scene};

const LANGUAGES: [&str; 11] = ["de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr"];

const NEW_TYPES: [&str; 9] = [
    "protest_strike",
    "bankruptcy_layoff_ipo",
    "sanction_embargo",
    "scientific_discovery",
    "outbreak_epidemic",
    "natural_disaster",
    "crime_incident",
    "ceasefire_truce",
    "record_milestone",
];

fn trigger(language: &str, event_type: &str) -> (String, String) {
    let path = format!(
        "{}/../lexicon/events/{language}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(path).expect("event lexicon");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("event lexicon json");
    let first = &value["event_types"][event_type][0];
    (
        first["surface"].as_str().expect("surface").to_string(),
        first["lemma"].as_str().expect("lemma").to_string(),
    )
}

fn events_for(language: &str, source: &str) -> Vec<serde_json::Value> {
    let value = manifest_value_for(language, vec![scene("event", source)]);
    value["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn every_new_event_type_grounds_in_all_eleven_languages() {
    for language in LANGUAGES {
        for event_type in NEW_TYPES {
            let (surface, _) = trigger(language, event_type);
            let source = format!("Maya Chen {surface} 2023.");
            let value = manifest_value_for(language, vec![scene("event", &source)]);
            let events = value["events"].as_array().cloned().unwrap_or_default();
            if language == "en" && event_type == "natural_disaster" {
                eprintln!("ENTITIES {:?}", value["scenes"][0]["entities"]);
            }
            let grounded = events.iter().find(|event| {
                event["event_type"] == event_type
                    && event["status"] == "asserted"
                    && event["participants"]
                        .as_array()
                        .is_some_and(|participants| !participants.is_empty())
            });
            assert!(
                grounded.is_some(),
                "{language}/{event_type}: no asserted grounded frame in {source}: {events:?}"
            );
            let frame = grounded.unwrap();
            let trigger_text = frame["trigger"].as_str().expect("trigger");
            let start = source
                .find(trigger_text)
                .unwrap_or_else(|| panic!("{language}/{event_type}: trigger {trigger_text:?} not grounded"));
            assert_eq!(
                &source[start..start + trigger_text.len()],
                trigger_text,
                "{language}/{event_type}: trigger span"
            );
        }
    }
}

#[test]
fn every_new_event_type_preserves_negation_and_uncertainty() {
    let negation = [
        ("de", "nicht"),
        ("en", "not"),
        ("es", "no"),
        ("fr", "ne ... pas"),
        ("id", "tidak"),
        ("it", "non"),
        ("nl", "niet"),
        ("pl", "nie"),
        ("pt", "não"),
        ("ru", "не"),
        ("tr", "değil"),
    ];
    let uncertainty = [
        ("de", "angeblich"),
        ("en", "reportedly"),
        ("es", "supuestamente"),
        ("fr", "présumé"),
        ("id", "diduga"),
        ("it", "forse"),
        ("nl", "vermoedelijk"),
        ("pl", "rzekomo"),
        ("pt", "supostamente"),
        ("ru", "якобы"),
        ("tr", "muhtemelen"),
    ];
    for (language, cue) in negation {
        for event_type in NEW_TYPES {
            let (surface, _) = trigger(language, event_type);
            let source = if language == "fr" {
                format!("Maya Chen ne {surface} pas 2023.")
            } else {
                format!("Maya Chen {cue} {surface} 2023.")
            };
            let events = events_for(language, &source);
            assert!(
                events.iter().any(|event| {
                    event["event_type"] == event_type && event["status"] == "negated"
                }),
                "{language}/{event_type}: expected negation in {source}: {events:?}"
            );
        }
    }
    for (language, cue) in uncertainty {
        for event_type in NEW_TYPES {
            let (surface, _) = trigger(language, event_type);
            let source = format!("Maya Chen {cue} {surface} 2023.");
            let events = events_for(language, &source);
            assert!(
                events.iter().any(|event| {
                    event["event_type"] == event_type && event["status"] == "uncertain"
                }),
                "{language}/{event_type}: expected uncertainty in {source}: {events:?}"
            );
        }
    }
}

#[test]
fn neutral_non_event_text_produces_no_new_event_frames() {
    for language in LANGUAGES {
        let events = events_for(language, "Maya Chen legge il giornale 2023.");
        assert!(
            events.iter().all(|event| {
                !NEW_TYPES
                    .iter()
                    .any(|kind| event["event_type"] == *kind)
            }),
            "{language}: false positive on neutral text: {events:?}"
        );
    }
}
