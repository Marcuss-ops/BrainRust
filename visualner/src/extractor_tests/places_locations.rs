//! Multi-word and nested place names, geographic cues and determinism.

use super::super::*;
use super::common::*;
use crate::ExtractOptions;

#[test]
fn a_known_city_before_two_names_stays_a_location() {
    let source = "В городе Пермь Мария Соколова и Наталья Белова начали проект.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "ru".to_string(),
            entity_count: 50,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Мария Соколова" && entity.r#type == "PERSON"),
        "person missing after the city: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Пермь" && entity.r#type == "LOCATION"),
        "city missing or absorbed: {entities:?}"
    );
    assert!(
        !entities
            .iter()
            .any(|entity| entity.text == "Пермь Мария Соколова"),
        "city must not open a person run: {entities:?}"
    );
}

#[test]
fn multilingual_scene_function_words_and_named_places_are_typed_safely() {
    for (language, source, expected_place) in [
        (
            "en",
            "The president Emmanuel Macron visited Berlin.",
            "Berlin",
        ),
        ("en", "J. D. Vance visited Berlin.", "Berlin"),
        (
            "it",
            "Il presidente Emmanuel Macron ha visitato Parigi.",
            "Parigi",
        ),
        ("es", "El presidente Emmanuel Macron visitó París.", "París"),
        (
            "pt",
            "O presidente Emmanuel Macron visitou Lisboa.",
            "Lisboa",
        ),
        (
            "fr",
            "Le président Emmanuel Macron a visité Paris.",
            "Paris",
        ),
        (
            "de",
            "Der Präsident Emmanuel Macron besuchte Berlin.",
            "Berlin",
        ),
    ] {
        let entities = extract(
            source,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        assert!(
            entities
                .iter()
                .any(
                    |entity| (entity.text == "Emmanuel Macron" || entity.text == "J. D. Vance")
                        && entity.r#type == "PERSON"
                ),
            "{language}: {entities:?}"
        );
        assert!(
            entities
                .iter()
                .any(|entity| entity.text == expected_place && entity.r#type == "LOCATION"),
            "{language}: {entities:?}"
        );
        assert!(
            !entities.iter().any(|entity| matches!(
                entity.text.to_lowercase().as_str(),
                "il" | "el"
                    | "o"
                    | "le"
                    | "der"
                    | "presidente"
                    | "président"
                    | "visited"
                    | "visitó"
                    | "visitato"
                    | "visitou"
                    | "visité"
                    | "besuchte"
            )),
            "function word leaked for {language}: {entities:?}"
        );
        for entity in &entities {
            assert_eq!(
                source.get(entity.start..entity.end),
                Some(entity.text.as_str())
            );
            assert_eq!(entity.evidence, entity.text);
        }
    }
}

#[test]
fn las_vegas_is_a_location_not_a_person() {
    let entities = top3("Mike Tyson fought in Las Vegas.");
    let las_vegas = entities.iter().find(|entity| entity.text == "Las Vegas");
    assert_eq!(
        las_vegas.map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
}

#[test]
fn italian_recipient_after_a_is_not_typed_as_a_location() {
    let text = "Il Comune affidò a Elena Conti la verifica. Lo Stadio Aurora si trova a Torino.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Elena Conti" && entity.r#type == "PERSON"),
        "the recipient is a person: {entities:?}"
    );
    assert!(!entities
        .iter()
        .any(|entity| entity.text == "Elena Conti" && entity.r#type == "LOCATION"));
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Torino" && entity.r#type == "LOCATION"),
        "the known city remains locative after the same preposition: {entities:?}"
    );
}

#[test]
fn named_location_survives_a_dense_top_five() {
    let text = "On November 22, 1986, in Las Vegas, he faced Trevor Berbick for the WBC heavyweight title. Tyson won by second-round technical knockout and became the youngest heavyweight world champion.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 5,
        },
    )
    .unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Las Vegas" && entity.r#type == "LOCATION"),
        "a named location must not be crowded out by visual-concept phrases: {entities:?}"
    );
}

#[test]
fn atlantic_city_is_a_location_not_a_person() {
    let entities = top3("In June 1988, Tyson met Michael Spinks in Atlantic City.");
    let atlantic_city = entities
        .iter()
        .find(|entity| entity.text == "Atlantic City");
    assert_eq!(
        atlantic_city.map(|entity| entity.r#type.as_str()),
        Some("LOCATION")
    );
}

#[test]
fn portuguese_geographic_cues_extract_city_and_neighborhood() {
    let text = "Isabelle Caracristi morreu em Fortaleza, no bairro Aldeota.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "pt".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    for place in ["Fortaleza", "Aldeota"] {
        let found = entities
            .iter()
            .find(|entity| entity.text == place)
            .unwrap_or_else(|| panic!("missing {place}: {entities:?}"));
        assert_eq!(found.r#type, "LOCATION", "{found:?}");
        assert_eq!(text.get(found.start..found.end), Some(place));
    }
}

#[test]
fn month_after_location_preposition_is_not_a_place() {
    let entities = extract(
        "The meeting was in May.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 8,
        },
    )
    .unwrap();
    assert!(!entities
        .iter()
        .any(|entity| entity.text == "May" && entity.r#type == "LOCATION"));
}

#[test]
fn deterministic_across_runs() {
    let text = "Greek salad contains tomatoes, feta cheese and olives.";
    let first = top3(text);
    for _ in 0..99 {
        let again = top3(text);
        assert_eq!(again, first, "non-deterministic extraction result");
    }
}

// The Italian city corpus of the "entity with text" run: every sentence names
// a regional capital and its region. The extractor must ground at least one
// LOCATION per scene, or the category-only request (locations) drops the
// scene and no entity card can be planned for it.
const ITALIAN_CITY_CORPUS: &[(&str, &str, &str)] = &[
    (
        "Roma",
        "Lazio",
        "Roma è la capitale dell'Italia e si trova nel Lazio.",
    ),
    (
        "Milano",
        "Lombardia",
        "Milano è il capoluogo della Lombardia e sorge nella pianura padana.",
    ),
    (
        "Torino",
        "Piemonte",
        "Torino si trova in Piemonte e ospita la Mole Antonelliana.",
    ),
    (
        "Napoli",
        "Campania",
        "Napoli è il capoluogo della Campania e si affaccia sul golfo.",
    ),
    (
        "Firenze",
        "Toscana",
        "Firenze è il capoluogo della Toscana e attraversata dall'Arno.",
    ),
    (
        "Venezia",
        "Veneto",
        "Venezia è il capoluogo del Veneto e sorge sulla laguna.",
    ),
    (
        "Bologna",
        "Emilia-Romagna",
        "Bologna è il capoluogo dell'Emilia-Romagna e ospita la più antica università.",
    ),
    (
        "Genova",
        "Liguria",
        "Genova è il capoluogo della Liguria e affacciata sul mar Ligure.",
    ),
    (
        "Palermo",
        "Sicilia",
        "Palermo è il capoluogo della Sicilia e si trova sul mar Tirreno.",
    ),
    (
        "Bari",
        "Puglia",
        "Bari è il capoluogo della Puglia e guarda il mare Adriatico.",
    ),
    (
        "Cagliari",
        "Sardegna",
        "Cagliari è il capoluogo della Sardegna e sorge nel golfo degli Angeli.",
    ),
    (
        "Trieste",
        "Friuli-Venezia Giulia",
        "Trieste è il capoluogo del Friuli-Venezia Giulia e si affaccia sull'Adriatico.",
    ),
    (
        "Perugia",
        "Umbria",
        "Perugia è il capoluogo dell'Umbria e si trova sul Tevere.",
    ),
    (
        "L'Aquila",
        "Abruzzo",
        "L'Aquila è il capoluogo dell'Abruzzo e sorge sull'Aterno.",
    ),
    (
        "Potenza",
        "Basilicata",
        "Potenza è il capoluogo della Basilicata e si trova nell'Appennino lucano.",
    ),
];

#[test]
fn italian_city_corpus_marks_every_scene_as_a_location() {
    let mut failures = Vec::new();
    for (city, region, text) in ITALIAN_CITY_CORPUS {
        let entities = extract(
            text,
            &ExtractOptions {
                language: "it".to_string(),
                entity_count: 24,
            },
        )
        .expect("it extraction");
        for place in [*city, *region] {
            let found = entities
                .iter()
                .find(|entity| entity.text == *place)
                .unwrap_or_else(|| panic!("{text:?}: missing {place:?} in {entities:?}"));
            if found.r#type != "LOCATION" {
                failures.push(format!(
                    "{text:?}: {place:?} typed {} ({found:?})",
                    found.r#type
                ));
            }
            if text.get(found.start..found.end) != Some(place) {
                failures.push(format!("{text:?}: {place:?} not grounded in the source"));
            }
        }
        for entity in &entities {
            // The verb guard covers the fallback path too: with no UDPipe
            // model there is no POS tag, so the curated verb vocabulary
            // must keep narration verbs out of the candidates entirely.
            if matches!(entity.r#type.as_str(), "VISUAL_CONCEPT")
                && [
                    "sorge",
                    "trova",
                    "affaccia",
                    "ospita",
                    "guarda",
                    "avvenne",
                    "arrivò",
                    "cambiò",
                    "nacque",
                    "conservato",
                ]
                .contains(&entity.text.as_str())
            {
                failures.push(format!(
                    "{text:?}: verb {:?} leaked as an entity",
                    entity.text
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn clause_openers_never_fuse_a_trailing_place() {
    // A voiceover imperative plus a city ("Visita Pavia oggi.", "Bezoek
    // Haarlem vandaag.") is a clause opener and a place, not one surface: the
    // place must survive as its own LOCATION in every language, and the
    // fused surface must never become an entity.
    for (language, sentence, opener, place) in [
        ("it", "Visita Pavia oggi.", "Visita", "Pavia"),
        ("en", "Visit London today.", "Visit", "London"),
        ("es", "Visita Madrid hoy.", "Visita", "Madrid"),
        ("pt", "Visite Lisboa hoje.", "Visite", "Lisboa"),
        ("de", "Besuche Berlin heute.", "Besuche", "Berlin"),
        ("fr", "Visitez Paris aujourd'hui.", "Visitez", "Paris"),
        ("nl", "Bezoek Haarlem vandaag.", "Bezoek", "Haarlem"),
    ] {
        let entities = extract(
            sentence,
            &ExtractOptions {
                language: language.to_string(),
                entity_count: 40,
            },
        )
        .unwrap_or_else(|error| panic!("{language} extraction failed: {error}"));
        assert!(
            entities
                .iter()
                .any(|entity| entity.text == place && entity.r#type == "LOCATION"),
            "{language}: {place} must survive as a LOCATION: {entities:?}"
        );
        let fused = format!("{opener} {place}");
        assert!(
            !entities.iter().any(|entity| {
                entity.text.eq_ignore_ascii_case(&fused) || entity.text == opener
            }),
            "{language}: the fused surface must not be an entity: {entities:?}"
        );
    }
}

#[test]
fn a_nested_place_survives_a_fused_institution_run() {
    // "Pengadilan Surabaya" is one ORG surface, but Surabaya is also a
    // grounded LOCATION: the fused run must never consume the place.
    let entities = extract(
        "Pengadilan Surabaya menjatuhkan putusan pada 2012.",
        &ExtractOptions {
            language: "id".to_string(),
            entity_count: 40,
        },
    )
    .expect("indonesian extraction");
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Pengadilan Surabaya" && entity.r#type == "ORG"),
        "the institution keeps its ORG surface: {entities:?}"
    );
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "Surabaya" && entity.r#type == "LOCATION"),
        "the nested place must survive the ORG surface: {entities:?}"
    );
}

#[test]
fn a_multiword_place_survives_a_clause_opener() {
    // "Visit New York today." must keep the whole place: the split rule fires
    // on the longest known-place suffix, never on its last token alone.
    let entities = extract(
        "Visit New York today.",
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 40,
        },
    )
    .expect("english extraction");
    assert!(
        entities
            .iter()
            .any(|entity| entity.text == "New York" && entity.r#type == "LOCATION"),
        "the multi-word place must stay whole: {entities:?}"
    );
    assert!(
        !entities.iter().any(|entity| entity.text == "York"),
        "the place must not be cut to its last token: {entities:?}"
    );
}
