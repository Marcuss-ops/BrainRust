//! Person-name integrity: initialisms, surname particles and aliasing.

use super::super::*;
use crate::ExtractOptions;
#[test]
fn spanish_propn_with_lowercase_lemma_is_a_person_and_a_place() {
    // The Spanish model lowercases every PROPN lemma ("Lucía" -> "lucía").
    // The lemma's case must not erase the person or the city.
    let source = "En Granada, Lucía Navarro y Ana Castillo comenzaron el proyecto.";
    let tokens = tokenize(source, "es").expect("es");
    if tokens.iter().all(|token| token.upos.is_none()) {
        eprintln!("UDPipe Spanish model not installed; PROPN assertions skipped");
        return;
    }
    let entities = extract(
        source,
        &ExtractOptions {
            language: "es".to_string(),
            entity_count: 50,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Lucía Navarro" && entity.r#type == "PERSON"),
        "Spanish name missing: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Granada" && entity.r#type == "LOCATION"),
        "Spanish city missing: {entities:?}"
    );
}

#[test]
fn dutch_surname_particle_stays_inside_the_person_name() {
    let source = "In Rotterdam begonnen Emma de Vries en Lotte Meijer het project.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "nl".to_string(),
            entity_count: 50,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Emma de Vries" && entity.r#type == "PERSON"),
        "Dutch particle name missing: {entities:?}"
    );
}

#[test]
fn a_place_word_as_a_given_name_stays_in_the_person() {
    // "Milan" is also a city, but here it opens "Milan van Dijk": the
    // surname particle rules out the city-plus-two-names split.
    let source = "In Leiden begonnen Sophie Smit en Milan van Dijk het project.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "nl".to_string(),
            entity_count: 50,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Milan van Dijk" && entity.r#type == "PERSON"),
        "given name colliding with a city missing: {entities:?}"
    );
}

#[test]
fn portuguese_propn_with_underscore_lemma_is_a_person_and_a_place() {
    // The Portuguese model returns "_" as the lemma; only the PROPN tag and
    // the surface survive.
    let source = "Em Braga, Clara Almeida e Ana Ferreira começaram o projeto.";
    let tokens = tokenize(source, "pt").expect("pt");
    if tokens.iter().all(|token| token.upos.is_none()) {
        eprintln!("UDPipe Portuguese model not installed; PROPN assertions skipped");
        return;
    }
    let entities = extract(
        source,
        &ExtractOptions {
            language: "pt".to_string(),
            entity_count: 50,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Clara Almeida" && entity.r#type == "PERSON"),
        "Portuguese name missing: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Braga" && entity.r#type == "LOCATION"),
        "Portuguese city missing: {entities:?}"
    );
}

#[test]
fn trump_is_retained_as_a_person_in_a_dense_scene() {
    let text = "Donald Trump's trajectory offers a profound case study into the interwoven nature of business success, intense media visibility, and political leadership within American public life. His career has consistently demonstrated how these three elements feed one another, creating a defining narrative arc for Donald Trump. Understanding Donald Trump's influence requires recognizing his public impact.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    let trump = entities
        .iter()
        .find(|entity| entity.r#type == "PERSON" && entity.text.to_lowercase().contains("trump"));
    assert!(
        trump.is_some(),
        "Donald Trump must survive the top-N visual entity bound: {entities:?}"
    );
    let trump = trump.unwrap();
    assert_eq!(trump.text, "Donald Trump");
    assert_eq!(trump.evidence, trump.text);
}

#[test]
fn person_runs_are_not_prefixed_or_suffixed_by_sentence_text() {
    let text = "Michael Jordan transformed basketball. Phil Jackson designed the offense. Scottie Pippen supplied versatile defense.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    let names: Vec<&str> = entities
        .iter()
        .filter(|entity| entity.r#type == "PERSON")
        .map(|entity| entity.text.as_str())
        .collect();
    assert!(
        names.contains(&"Michael Jordan"),
        "Michael Jordan missing: {entities:?}"
    );
    assert!(
        names.contains(&"Phil Jackson"),
        "Phil Jackson missing: {entities:?}"
    );
    assert!(
        names.contains(&"Scottie Pippen"),
        "Scottie Pippen missing: {entities:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.contains("designed") || name.contains("supplied")),
        "predicate leaked into person: {names:?}"
    );
}

#[test]
fn stopword_leads_and_known_locations_do_not_consume_person_slots() {
    let text = "Michael Jordan studied at the University of North Carolina with Dean Smith. When Jordan entered the game, Scottie Pippen and Phil Jackson supported him.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    let persons: Vec<&str> = entities
        .iter()
        .filter(|entity| entity.r#type == "PERSON")
        .map(|entity| entity.text.as_str())
        .collect();
    assert_eq!(
        persons,
        vec![
            "Michael Jordan",
            "Dean Smith",
            "Scottie Pippen",
            "Phil Jackson"
        ]
    );
    assert!(
        !entities.iter().any(|entity| entity.text == "When Jordan"),
        "stopword-prefixed false person must be rejected: {entities:?}"
    );
    assert!(
        !persons.contains(&"North Carolina"),
        "known location must not be typed as PERSON: {entities:?}"
    );
}

#[test]
fn dotted_person_initials_keep_the_full_grounded_name() {
    let source = "J. D. Vance visited Berlin.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    let vance = entities.iter().find(|entity| entity.text == "J. D. Vance");
    assert_eq!(
        vance.map(|entity| entity.r#type.as_str()),
        Some("PERSON"),
        "{entities:?}"
    );
    let vance = vance.unwrap();
    assert_eq!(
        source.get(vance.start..vance.end),
        Some(vance.text.as_str())
    );
    assert!(!entities
        .iter()
        .any(|entity| entity.text == "J" || entity.text == "D" || entity.text == "Vance"));
}

#[test]
fn dolly_places_and_song_titles_do_not_consume_person_slots() {
    let text = "Dolly Parton was born in Sevier County, Tennessee, in the Great Smoky Mountains. Porter Wagoner worked with Dolly Parton. I Will Always Love You became a song.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 10,
        },
    )
    .unwrap();
    let find = |name: &str| entities.iter().find(|entity| entity.text == name);
    assert_eq!(
        find("Dolly Parton").map(|entity| entity.r#type.as_str()),
        Some("PERSON")
    );
    assert_eq!(
        find("Porter Wagoner").map(|entity| entity.r#type.as_str()),
        Some("PERSON")
    );
    assert_eq!(
        find("Sevier County").map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
    assert_eq!(
        find("Great Smoky Mountains").map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
    assert_eq!(
        find("Will Always Love You").map(|entity| entity.r#type.as_str()),
        Some("WORK")
    );
    assert!(!entities.iter().any(|entity| {
        entity.r#type == "PERSON"
            && matches!(
                entity.text.as_str(),
                "Sevier County"
                    | "Great Smoky Mountains"
                    | "Will Always Love"
                    | "Will Always Love You"
            )
    }));
}

// TestEntitiesRequireSourceEvidence — explicit: an entity not in the
// source text must never be returned. Synthetic check.

#[test]
fn a_sentence_initial_name_is_still_a_person() {
    // UDPipe lemmatizes "Chiara" as the adjective "chiaro" when the name opens
    // the sentence; the PROPN tag on both tokens is the evidence that survives.
    let text = "Chiara Poggi aveva ventisei anni. Il 13 agosto 2007, Chiara Poggi fu uccisa.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 64,
        },
    )
    .expect("italian extraction");
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Chiara Poggi" && entity.r#type == "PERSON"),
        "sentence-initial name typed as a visual concept: {entities:?}"
    );
}

#[test]
fn a_surname_alias_points_at_its_full_name() {
    let context = script_person_tokens(
        &[
            "Alberto Stasi ha testimoniato. Il 12 dicembre 2015 la condanna divenne definitiva.",
            "La posizione di Stasi resta distinta da quella di Andrea Sempio.",
            "Andrea Sempio respinge le accuse.",
        ],
        "it",
    );
    assert_eq!(
        context.person_surnames.get("stasi").map(String::as_str),
        Some("Alberto Stasi"),
        "surname map: {:?}",
        context.person_surnames
    );
    assert_eq!(
        context.person_surnames.get("sempio").map(String::as_str),
        Some("Andrea Sempio"),
        "surname map: {:?}",
        context.person_surnames
    );
    // A shared surname stays ambiguous and is never aliased.
    let shared = script_person_tokens(&["Marco Rossi ha parlato. Luca Rossi ha risposto."], "it");
    assert!(
        !shared.person_surnames.contains_key("rossi"),
        "a shared surname must stay ambiguous: {:?}",
        shared.person_surnames
    );
}

#[test]
fn an_article_led_facility_name_keeps_its_city_a_place() {
    // The facility head of "Lo Stadio Aurora di Torino" is preceded by an
    // article. Reading the article as the head hides the facility cue, and the
    // run then seeds the trailing city as a person-name fragment — which drops
    // every grounded "Torino" of the scene, including the sentence-initial
    // one that has nothing to do with the facility.
    let source = "Torino. Lo Stadio Aurora di Torino ha 10.000 posti.";
    let context = script_person_tokens(&[source], "it");
    assert!(
        !context.person_name_tokens.contains("torino"),
        "the city must stay available as a place: {:?}",
        context.person_name_tokens
    );
    let entities = extract_with_context(
        source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
        &context,
    )
    .expect("italian extraction");
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Torino" && entity.r#type == "LOCATION"),
        "the grounded city must survive the facility name: {entities:?}"
    );
    // The extractor keeps the facility name whole and types the head span; the
    // engine's facility collection promotes the longer span (see
    // semantic-engine `collect_facilities`).
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Stadio Aurora di Torino"),
        "the facility keeps its genitive place: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Stadio Aurora" && entity.r#type == "FACILITY"),
        "the facility head is recognized: {entities:?}"
    );
}

#[test]
fn a_place_surname_stays_a_person_name() {
    // "Messina" is a known place, but a run opened by a real name keeps its
    // trailing surname: splitting there would destroy every person whose
    // surname doubles as a city.
    let entities = extract(
        "Elena Messina entra nel tribunale.",
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 40,
        },
    )
    .expect("italian extraction");
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Elena Messina" && entity.r#type == "PERSON"),
        "a person with a place surname must stay whole: {entities:?}"
    );
}
