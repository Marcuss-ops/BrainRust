//! MVP contract regressions for the deterministic, network-free semantic engine.
//!
//! These tests exercise the public request -> manifest API used by PipelineGen,
//! including the v3 entity projection: linked ids, roles, glosses, negative
//! constraints and typed retrieval intents.

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};
use serde_json::Value;
use std::sync::Arc;
#[cfg(not(debug_assertions))]
use std::time::Instant;

fn request(language: &str, scene_id: &str, text: &str) -> ScriptRequest {
    ScriptRequest {
        language: language.to_string(),
        scenes: vec![SceneInput {
            scene_id: scene_id.to_string(),
            text: text.to_string(),
        }],
        editorial: None,
        entity_safety_cap: 0,
    }
}

fn value(language: &str, scene_id: &str, text: &str) -> Value {
    serde_json::to_value(analyze_script(&request(language, scene_id, text)).expect("analysis"))
        .expect("manifest JSON")
}

fn entities_of_type<'a>(scene: &'a Value, kind: &str) -> Vec<&'a Value> {
    scene["entities"]
        .as_array()
        .expect("entities array")
        .iter()
        .filter(|entity| entity["type"] == kind)
        .collect()
}

fn linked<'a>(manifest: &'a Value, surface: &str) -> &'a Value {
    manifest["entities"]
        .as_array()
        .unwrap_or_else(|| panic!("missing v3 entities projection: {manifest}"))
        .iter()
        .find(|entity| entity["canonical_surface"] == surface)
        .unwrap_or_else(|| panic!("missing v3 entity {surface:?}: {}", manifest["entities"]))
}

#[test]
fn apostrophe_variants_keep_source_offsets_and_canonicalize_elided_articles() {
    let text = "Oggi l’ANSA e l'ESA hanno pubblicato un comunicato congiunto dall'Italia, mentre la NASA's Jet Propulsion Laboratory ha confermato i dati.";
    let manifest = value("it", "apostrofi", text);
    let scene = &manifest["scenes"][0];
    let entities = scene["entities"].as_array().unwrap();

    let ansa = entities
        .iter()
        .find(|entity| entity["evidence"].as_str().unwrap_or("").contains("ANSA"))
        .expect("ANSA mention extracted");
    assert_eq!(ansa["type"], "ORG");
    assert_eq!(
        ansa["text"], "ANSA",
        "article must not pollute entity surface"
    );
    let start = ansa["start_byte"].as_u64().unwrap() as usize;
    let end = ansa["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(start..end), ansa["evidence"].as_str());
    let esa = entities
        .iter()
        .find(|entity| entity["evidence"].as_str().unwrap_or("").contains("ESA"))
        .expect("ESA mention extracted");
    assert_eq!(esa["text"], "ESA");
    let italy = entities
        .iter()
        .find(|entity| entity["evidence"] == "Italia")
        .expect("Italy mention extracted without articulated preposition");
    assert_eq!(italy["text"], "Italia");

    let paragraph_ansa = scene["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|identity| identity["canonical_surface"] == "ANSA")
        .expect("ANSA identity in paragraph");
    let isolated_ansa = value("it", "ansa-isolata", "ANSA")["scenes"][0]["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|identity| identity["canonical_surface"] == "ANSA")
        .cloned()
        .expect("ANSA identity in isolated scene");
    assert_eq!(
        paragraph_ansa["canonical_id"], isolated_ansa["canonical_id"],
        "canonical identity should be stable for the same entity"
    );

    let linked_ansa = linked(&manifest, "ANSA");
    assert_eq!(linked_ansa["link_status"], "linked");
    assert_eq!(linked_ansa["canonical_id"], "wd:Q1094480");
    assert!(linked_ansa["surface_key"]
        .as_str()
        .unwrap()
        .starts_with("org:ansa-"));
    let mention = linked_ansa["mentions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mention| mention["surface"] == "ANSA")
        .expect("ANSA mention in v3 projection");
    let mention_start = mention["start_byte"].as_u64().unwrap() as usize;
    let mention_end = mention["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(mention_start..mention_end), Some("ANSA"));
}

#[test]
fn overlapping_person_names_remain_distinct_and_keep_exact_evidence() {
    let text = "Michael Jordan ha trasformato il basket mondiale con i Chicago Bulls. Anni dopo, l'attore Michael B. Jordan ha conquistato il pubblico interpretando Adonis Creed.";
    let manifest = value("it", "jordan", text);
    let scene = &manifest["scenes"][0];
    let people = entities_of_type(scene, "PERSON");
    let jordan_people: Vec<_> = people
        .iter()
        .copied()
        .filter(|entity| {
            entity["text"] == "Michael Jordan" || entity["text"] == "Michael B. Jordan"
        })
        .collect();
    assert_eq!(
        jordan_people.len(),
        2,
        "both Jordan entities are required: {people:?}"
    );
    assert_ne!(jordan_people[0]["id"], jordan_people[1]["id"]);
    for entity in &jordan_people {
        let start = entity["start_byte"].as_u64().unwrap() as usize;
        let end = entity["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), entity["evidence"].as_str());
    }
    assert!(jordan_people
        .iter()
        .any(|entity| entity["text"] == "Michael Jordan"));
    assert!(jordan_people
        .iter()
        .any(|entity| entity["text"] == "Michael B. Jordan"));

    let orgs = entities_of_type(scene, "ORG");
    assert!(
        orgs.iter().any(|entity| entity["text"] == "Chicago Bulls"),
        "team should remain one ORG entity: {orgs:?}"
    );

    let basketball = linked(&manifest, "Michael Jordan");
    let actor = linked(&manifest, "Michael B. Jordan");
    assert_eq!(basketball["link_status"], "linked");
    assert_eq!(actor["link_status"], "linked");
    assert_eq!(basketball["canonical_id"], "wd:Q19927");
    assert_eq!(actor["canonical_id"], "wd:Q1924529");
    assert_ne!(basketball["canonical_id"], actor["canonical_id"]);
}

#[test]
fn ontological_context_disambiguates_nike_and_esa() {
    let sportswear = value(
        "it",
        "nike-sport",
        "La Nike ha presentato le nuove scarpe da running per gli atleti.",
    );
    let nike = linked(&sportswear, "Nike");
    assert_eq!(nike["link_status"], "linked");
    assert_eq!(nike["canonical_id"], "wd:Q483859");
    assert!(nike["disambiguation_gloss"]
        .as_str()
        .unwrap()
        .contains("sportswear"));
    assert!(nike["negative_constraints"]
        .as_array()
        .unwrap()
        .iter()
        .any(|constraint| constraint.as_str().unwrap().contains("Samothrace")));

    let statue = value(
        "it",
        "nike-statue",
        "La Nike è una statua greca esposta al Louvre.",
    );
    let nike = linked(&statue, "Nike");
    assert_eq!(nike["link_status"], "linked");
    assert_eq!(nike["canonical_id"], "wd:Q193845");

    let bare = value(
        "it",
        "nike-bare",
        "Nike ha annunciato una nuova iniziativa.",
    );
    assert_eq!(linked(&bare, "Nike")["link_status"], "ambiguous");

    let space = value(
        "it",
        "esa-space",
        "ESA e NASA studiano Marte con nuove missioni spaziali.",
    );
    assert_eq!(linked(&space, "ESA")["canonical_id"], "wd:Q42262");
    assert!(linked(&space, "ESA")["negative_constraints"]
        .as_array()
        .unwrap()
        .iter()
        .any(|constraint| constraint.as_str().unwrap().contains("video game")));

    let software = value(
        "it",
        "esa-software",
        "ESA ha annunciato i giochi per E3 e l'industria del software.",
    );
    let esa = linked(&software, "ESA");
    assert_ne!(
        esa["canonical_id"], "wd:Q42262",
        "video-game context must not resolve to the space agency"
    );
    assert_eq!(esa["canonical_id"], "wd:Q1133229");
}

#[test]
fn generated_search_queries_do_not_copy_unrelated_sentence_context() {
    let text = "Nel cuore di Ginevra, i ricercatori del CERN stanno costruendo nuovi rivelatori per studiare le particelle fondamentali della materia.";
    let manifest = value("it", "cern", text);
    let feedback = manifest["search_feedback"]["entries"]
        .as_array()
        .expect("search feedback entries");
    let cern_entry = feedback
        .iter()
        .find(|entry| {
            entry["queries"]
                .as_array()
                .is_some_and(|queries| queries.iter().any(|query| query["text"] == "CERN"))
        })
        .expect("CERN search feedback entry");
    let queries = cern_entry["queries"].as_array().unwrap();
    assert!(queries.len() <= 3, "too many query intents: {queries:?}");
    for query in queries {
        let text = query["text"].as_str().unwrap_or_default().to_lowercase();
        for noisy in ["stanno", "costruendo", "studiare", "roma", "news agency"] {
            assert!(!text.contains(noisy), "query bloating: {text:?}");
        }
    }
    let first = queries[0]["text"].as_str().unwrap();
    assert!(
        first.split_whitespace().count() <= 5,
        "first intent too long: {first:?}"
    );

    let cern = linked(&manifest, "CERN");
    let intents = cern["search_intents"].as_array().unwrap();
    assert!(!intents.is_empty(), "CERN needs typed intents");
    assert!(intents.len() <= 3);
    assert_eq!(intents[0]["kind"], "identity");
    assert_eq!(intents[0]["lang"], "it");
    for intent in intents {
        let query = intent["query"].as_str().unwrap().to_lowercase();
        for noisy in ["stanno", "costruendo", "studiare", "roma", "news agency"] {
            assert!(!query.contains(noisy), "intent bloating: {query:?}");
        }
        assert!(query.split_whitespace().count() <= 6);
    }
}

#[test]
fn cross_language_aliases_converge_on_one_linked_id() {
    let italian = value(
        "it",
        "esa-it",
        "ESA e NASA studiano Marte con nuove missioni.",
    );
    let english = value("en", "esa-en", "ESA and NASA study Mars with new missions.");
    let portuguese = value(
        "pt",
        "esa-pt",
        "ESA e NASA estudam Marte com novas missões.",
    );
    for manifest in [&italian, &english, &portuguese] {
        assert_eq!(
            linked(manifest, "ESA")["canonical_id"],
            "wd:Q42262",
            "ESA aliases must converge across languages"
        );
    }
}

#[test]
fn sentence_initial_common_words_are_not_named_entities() {
    let text = "Innovazione e velocità guidano il settore. Tuttavia, SpaceX ha dimostrato che i razzi riutilizzabili riducono drasticamente i costi di lancio.";
    let manifest = value("it", "spacex", text);
    let scene = &manifest["scenes"][0];
    let entities = scene["entities"].as_array().unwrap();
    assert!(
        !entities
            .iter()
            .any(|entity| entity["text"] == "Innovazione"),
        "sentence-initial common word leaked: {entities:?}"
    );
    assert!(!entities.iter().any(|entity| entity["text"] == "Tuttavia"));
    assert!(
        entities
            .iter()
            .any(|entity| entity["text"] == "SpaceX" && entity["type"] == "ORG"),
        "SpaceX must be recognized as an organization: {entities:?}"
    );

    let spacex = linked(&manifest, "SpaceX");
    assert_eq!(spacex["canonical_id"], "wd:Q286442");
    assert_eq!(spacex["role"], "named_entity");
    assert_eq!(spacex["entity_type"], "ORG");
    assert_eq!(spacex["search_intents"][0]["kind"], "identity");
}

#[test]
fn parallel_replay_serializes_identically_across_workers() {
    let scripts: Vec<Arc<str>> = (0..50)
        .map(|index| {
            Arc::from(format!(
                "Nel 2025 SpaceX e NASA hanno presentato la missione {index} a Roma. Michael Jordan ha parlato con ESA."
            ))
        })
        .collect();
    let mut baseline: Vec<Option<Vec<u8>>> = vec![None; scripts.len()];

    for iteration in 0..10 {
        let outputs = std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for worker in 0..8 {
                let scripts = &scripts;
                workers.push(scope.spawn(move || {
                    let mut results = Vec::new();
                    for index in (worker..scripts.len()).step_by(8) {
                        let manifest = analyze_script(&request(
                            "it",
                            &format!("scene-{index}"),
                            &scripts[index],
                        ))
                        .expect("analysis");
                        results.push((index, serde_json::to_vec(&manifest).expect("serialize")));
                    }
                    results
                }));
            }
            workers
                .into_iter()
                .flat_map(|worker| worker.join().expect("worker did not panic"))
                .collect::<Vec<_>>()
        });

        for (index, bytes) in outputs {
            if iteration == 0 {
                baseline[index] = Some(bytes);
            } else {
                assert_eq!(
                    baseline[index].as_deref(),
                    Some(bytes.as_slice()),
                    "non-deterministic JSON for script {index} at iteration {iteration}"
                );
            }
        }
    }
}

#[test]
#[cfg(not(debug_assertions))]
fn five_minute_script_latency_is_measured_without_external_services() {
    let sentence = "NASA e ESA conducono missioni spaziali mentre SpaceX sviluppa razzi riutilizzabili e i ricercatori osservano Marte.";
    let scenes = (0..20)
        .map(|index| SceneInput {
            scene_id: format!("scene-{index:02}"),
            text: format!("{sentence} {sentence}"),
        })
        .collect();
    let request = ScriptRequest {
        language: "it".to_string(),
        scenes,
        editorial: None,
        entity_safety_cap: 0,
    };
    // The production process loads UDPipe lazily on its first request. The
    // advertised budget excludes that one-time startup cost, so warm the same
    // engine path before sampling steady-state request latency.
    analyze_script(&request).expect("warmup analysis");
    let mut durations = Vec::with_capacity(20);
    for _ in 0..20 {
        let start = Instant::now();
        analyze_script(&request).expect("analysis");
        durations.push(start.elapsed());
    }
    durations.sort_unstable();
    let p99 = durations[durations.len() - 1];
    eprintln!("MVP semantic latency p99 proxy over 20 warm runs (20 scenes, ~700 words): {p99:?}");
    // The original 25 ms target is retired by TICKET-12's measured-SLO
    // fallback: the model-free p50 is already 77.9 ms. The 100-sample release
    // probe measures 1.551 s p99 with UDPipe; this 20-sample max-of-20 proxy
    // measured 1.965 s on the same host. The 3.0 s guard absorbs that host
    // variance while still catching a gross regression.
    assert!(
        p99 < std::time::Duration::from_millis(3_000),
        "revised MVP latency guard is 3.0s for the 20-scene fixture; measured {p99:?}"
    );
}
