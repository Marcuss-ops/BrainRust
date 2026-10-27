//! Grounded search and image-query composition with provenance invariants.

mod common;

use common::*;
use semantic_engine::{analyze_script, ScriptRequest};

#[test]
fn every_visual_entity_mention_gets_a_grounded_asset_search_query() {
    let text = "Stadio Aurora fu inaugurato nel 1959 e si trova a Torino.";
    let value = manifest_value(vec![scene("visual-search", text)]);
    let scene = &value["scenes"][0];
    let entities = scene["entities"].as_array().unwrap();
    let searches = proposals_of(scene, "entity_search");
    assert!(searches
        .iter()
        .all(|search| search["visual_query_suggestions"][0] != "si"));
    let expected: Vec<_> = entities
        .iter()
        .filter(|entity| {
            !["NUMBER", "PERCENT", "MONEY", "DATE"].contains(&entity["type"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(
        searches.len(),
        expected.len(),
        "each pictured mention needs a query: {scene}"
    );
    for search in searches {
        let query = search["visual_query_suggestions"][0].as_str().unwrap();
        assert!(text.contains(query), "query must be grounded: {query:?}");
        assert_eq!(search["entity_ids"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn entity_search_queries_include_the_grounded_sentence_context_for_images_and_stock() {
    let text = "I tifosi festeggiano sugli spalti dopo la vittoria della squadra.";
    let value = manifest_value(vec![scene("tifosi-in-festa", text)]);
    let searches = proposals_of(&value["scenes"][0], "entity_search");
    // The entity-anchored phrase is compact enough for provider search while
    // retaining the complete sentence and exact entity as fallbacks.
    let tifosi = searches
        .iter()
        .find(|proposal| {
            proposal["visual_query_suggestions"][0] == "tifosi festeggiano sugli spalti"
        })
        .expect("the source-grounded supporter concept gets a search proposal");
    let queries = tifosi["visual_query_suggestions"]
        .as_array()
        .expect("query suggestions array");
    assert_eq!(
        queries[0], "tifosi festeggiano sugli spalti",
        "lead with a compact phrase anchored at the visual subject"
    );
    assert!(
        queries
            .iter()
            .any(|query| query == "tifosi festeggiano sugli spalti"),
        "keep the exact entity span as a fallback query: {queries:?}"
    );
    for query in queries {
        assert!(text.contains(query.as_str().unwrap()), "grounded: {query}");
    }
    assert!(queries.iter().any(|query| query == text));
}

#[test]
fn literal_query_context_preserves_newlines_tabs_and_repeated_spaces() {
    let text = "I tifosi\tfesteggiano  sugli spalti\ndopo la vittoria della squadra.";
    let value = manifest_value(vec![scene("whitespace-context", text)]);
    let searches = proposals_of(&value["scenes"][0], "entity_search");
    assert!(!searches.is_empty());
    for proposal in searches {
        for query in proposal["visual_query_suggestions"].as_array().unwrap() {
            let query = query.as_str().unwrap();
            assert!(
                text.contains(query),
                "literal query changed source whitespace: {query:?}"
            );
        }
    }
}

#[test]
fn typed_visual_objects_all_receive_grounded_reusable_search_queries() {
    for (language, text, scene_id) in [
        ("en", "Ball, trophy, jersey.", "sports-objects-en"),
        ("it", "Pallone, trofeo, maglia.", "sports-objects-it"),
        ("es", "Balón, trofeo, camiseta.", "sports-objects-es"),
        ("fr", "Ballon, trophée, maillot.", "sports-objects-fr"),
        ("de", "Ball, Pokal, Trikot.", "sports-objects-de"),
        ("pt", "Bola, troféu, camiseta.", "sports-objects-pt"),
        ("pl", "Piłka, trofeum, koszulka.", "sports-objects-pl"),
        ("ru", "Мяч, трофей, футболка.", "sports-objects-ru"),
        ("tr", "Top, kupa, forma.", "sports-objects-tr"),
        ("id", "Bola, piala, seragam.", "sports-objects-id"),
    ] {
        let manifest = analyze_script(&ScriptRequest {
            language: language.to_string(),
            scenes: vec![scene(scene_id, text)],
            editorial: None,
            entity_safety_cap: 0,
        })
        .expect("object script analysis");
        let value = serde_json::to_value(manifest).unwrap();
        let scene = &value["scenes"][0];
        let objects: Vec<_> = scene["entities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entity| entity["type"] == "OBJECT")
            .collect();
        assert_eq!(objects.len(), 3, "all typed objects must survive: {scene}");
        let searches = proposals_of(scene, "entity_search");
        for object in objects {
            let id = object["id"].as_str().unwrap();
            let proposal = searches
                .iter()
                .find(|proposal| proposal["entity_ids"][0] == id)
                .expect("every object needs a search proposal");
            let query = proposal["visual_query_suggestions"][0].as_str().unwrap();
            assert!(text.contains(query));
        }
    }
}

#[test]
fn sentence_phrases_keywords_and_visual_queries_are_reusable_and_grounded() {
    let text =
        "Alberto Stasi entra nel tribunale. Andrea Sempio osserva il palazzo storico della città.";
    let value = manifest_value(vec![scene("frasi-e-query", text)]);
    let scene = &value["scenes"][0];
    let mut phrases = scene["short_phrases"].as_array().unwrap().clone();
    phrases.extend(scene["long_phrases"].as_array().unwrap().clone());
    assert_eq!(
        phrases.len(),
        2,
        "one phrase per canonical sentence: {scene}"
    );
    for (expected_index, phrase) in phrases.iter().enumerate() {
        assert_eq!(phrase["sentence_index"], expected_index);
        let start = phrase["start_byte"].as_u64().unwrap() as usize;
        let end = phrase["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), phrase["evidence"].as_str());
    }
    assert!(!scene["important_words"].as_array().unwrap().is_empty());
    for keyword in scene["important_words"].as_array().unwrap() {
        let start = keyword["start_byte"].as_u64().unwrap() as usize;
        let end = keyword["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), keyword["evidence"].as_str());
    }
}

#[test]
fn explicit_quotes_are_preserved_as_grounded_candidates_with_utf8_offsets() {
    let text = "Il testimone disse: «È stato lui». Poi ha aggiunto: “Non ricordo”.";
    let value = manifest_value(vec![scene("citazioni", text)]);
    let quotes = value["scenes"][0]["quotes"].as_array().unwrap();
    assert_eq!(
        quotes.len(),
        2,
        "both explicit quotations must survive: {quotes:?}"
    );
    for quote in quotes {
        let start = quote["start_byte"].as_u64().unwrap() as usize;
        let end = quote["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), quote["evidence"].as_str());
        assert_eq!(quote["text"], quote["evidence"]);
        assert_eq!(quote["type"], "QUOTE");
    }
}

#[test]
fn visual_query_suggestions_are_always_verbatim_source_slices() {
    // A co-mention and a chart used to suggest their joined labels
    // ("Alberto Stasi Andrea Sempio", "42% 18%"), which never occur in the
    // source. Every rung of the ladder is a provider query, so every rung
    // must be traceable to the text.
    let text = "Alberto Stasi e Andrea Sempio hanno incontrato il 42% dei tecnici, contro il 18% del gruppo di controllo.";
    let value = manifest_value(vec![scene("query-grounding", text)]);
    let scene = &value["scenes"][0];
    assert!(!proposals_of(scene, "multiple_entities").is_empty());
    assert!(!proposals_of(scene, "chart").is_empty());
    for proposal in scene["visual_proposals"].as_array().unwrap() {
        for query in proposal["visual_query_suggestions"].as_array().unwrap() {
            let query = query.as_str().unwrap();
            assert!(
                text.contains(query),
                "query {query:?} is not a source slice: {proposal}"
            );
        }
    }
}

#[test]
fn contextual_image_query_is_a_grounded_subject_place_composition() {
    let text = "Lo Stadio Aurora è un impianto sportivo di Torino.";
    let value = manifest_value(vec![scene("contextual-search", text)]);
    let scene_value = &value["scenes"][0];
    let queries = scene_value["contextual_image_queries"]
        .as_array()
        .expect("contextual image query collection");
    let query = queries
        .iter()
        .find(|query| query["query"] == "Stadio Aurora Torino")
        .unwrap_or_else(|| panic!("grounded subject-place query missing: {queries:?}"));
    assert_eq!(query["kind"], "contextual_image_query");
    assert_eq!(query["evidence"], text);
    assert!(query["query"] != query["evidence"]);
    assert_eq!(query["components"].as_array().unwrap().len(), 2);
    for component in query["components"].as_array().unwrap() {
        let start = component["start_byte"].as_u64().unwrap() as usize;
        let end = component["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&text[start..end], component["text"].as_str().unwrap());
    }
    assert!(queries.iter().all(|query| query["evidence"] == text));

    let co_mention = "Il rapporto cita Stadio Aurora e Torino come riferimenti distinti.";
    let unrelated = manifest_value(vec![scene("unrelated-context", co_mention)]);
    assert!(
        unrelated["scenes"][0]["contextual_image_queries"]
            .as_array()
            .unwrap()
            .is_empty(),
        "co-mention alone must not imply a location relation"
    );

    let negated = manifest_value(vec![scene(
        "negated-location",
        "Lo Stadio Aurora non si trova a Torino.",
    )]);
    assert!(
        negated["scenes"][0]["contextual_image_queries"]
            .as_array()
            .unwrap()
            .is_empty(),
        "a negated location relation must not become an image query"
    );
}

#[test]
fn contextual_production_query_inherits_a_nested_place_without_an_entity_owner() {
    let text = "The city built a treatment plant at Collins Park in East Toledo.";
    let manifest = analyze_script(&ScriptRequest {
        language: "en".to_string(),
        scenes: vec![scene("nested-place-context", text)],
        editorial: None,
        entity_safety_cap: 0,
    })
    .expect("script analysis");
    let value = serde_json::to_value(manifest).unwrap();
    let candidates = value["visual_consolidation"]["production_candidates"]
        .as_array()
        .expect("production candidates");
    let contextual = candidates
        .iter()
        .find(|candidate| {
            candidate["recipe"] == "contextual_search"
                && candidate["title"] == "plant Collins Park East Toledo"
        })
        .unwrap_or_else(|| panic!("nested place context missing: {candidates:?}"));
    let query = &contextual["queries"][0];
    assert_eq!(query["text"], "plant Collins Park East Toledo");
    assert_eq!(query["evidence"], text);
    for component in query["components"].as_array().unwrap() {
        let start = component["start_byte"].as_u64().unwrap() as usize;
        let end = component["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&text[start..end], component["text"].as_str().unwrap());
    }
}

#[test]
fn unbuilt_proposals_do_not_become_searches_for_existing_factories() {
    let text =
        "A proposed battery factory will be built in Michigan. Construction has not started.";
    let manifest = analyze_script(&ScriptRequest {
        language: "en".to_string(),
        scenes: vec![scene("proposed-factory", text)],
        editorial: None,
        entity_safety_cap: 0,
    })
    .expect("script analysis");
    let value = serde_json::to_value(manifest).unwrap();
    let candidates = value["visual_consolidation"]["production_candidates"]
        .as_array()
        .expect("production candidates");
    assert!(!candidates.iter().any(|candidate| {
        candidate["recipe"] == "contextual_search"
            && candidate["title"]
                .as_str()
                .is_some_and(|title| title.to_lowercase().contains("battery"))
    }));
}

#[test]
fn location_lists_do_not_become_one_joint_visual_query() {
    let text = "Windsor, Canada, and Fort Wayne, Indiana are closer than Columbus.";
    let manifest = analyze_script(&ScriptRequest {
        language: "en".to_string(),
        scenes: vec![scene("place-list", text)],
        editorial: None,
        entity_safety_cap: 0,
    })
    .expect("script analysis");
    let value = serde_json::to_value(manifest).unwrap();
    let queries = value["scenes"][0]["contextual_image_queries"]
        .as_array()
        .expect("contextual queries");
    assert!(
        !queries.iter().any(|query| {
            query["query"] == "Windsor Fort Wayne Indiana"
                || query["query"] == "Windsor Canada Fort Wayne Indiana"
        }),
        "place list should not become one joint query: {queries:?}"
    );
}

#[test]
fn locative_queries_require_a_locative_predicate_and_use_local_place_spans() {
    let text = "Nel 2018 il Comune di Torino affidò a Elena Conti il controllo dello Stadio Aurora. Stadio Aurora, inaugurato nel 1961 e situato a Torino, aveva 18.000 posti.";
    let value = manifest_value(vec![scene("explicit-locative", text)]);
    let scene = &value["scenes"][0];
    let queries = scene["contextual_image_queries"].as_array().unwrap();
    let query = queries
        .iter()
        .find(|query| query["query"] == "Stadio Aurora Torino")
        .unwrap_or_else(|| panic!("grounded place query missing: {scene}"));
    assert_eq!(
        queries.len(),
        1,
        "the org's Torino must not locate the stadium"
    );
    assert_eq!(
        query["evidence"],
        "Stadio Aurora, inaugurato nel 1961 e situato a Torino, aveva 18.000 posti."
    );
    assert!(queries
        .iter()
        .all(|query| query["query"] != "Stadio Aurora Elena Conti"));
    for component in query["components"].as_array().unwrap() {
        let start = component["start_byte"].as_u64().unwrap() as usize;
        let end = component["end_byte"].as_u64().unwrap() as usize;
        let evidence_start = query["evidence_start_byte"].as_u64().unwrap() as usize;
        let evidence_end = query["evidence_end_byte"].as_u64().unwrap() as usize;
        assert!(evidence_start <= start && start < end && end <= evidence_end);
        assert_eq!(&text[start..end], component["text"].as_str().unwrap());
    }
}

#[test]
fn street_addresses_are_extracted_as_locations_and_entity_co_mentions_do_not_make_queries() {
    let text = "Il corpo fu ritrovato nell'abitazione di via Pascoli. Nel caso compaiono Chiara Poggi e Alberto Stasi.";
    let value = manifest_value(vec![scene("indirizzo-e-ricerca", text)]);
    let scene = &value["scenes"][0];
    let entities = scene["entities"].as_array().unwrap();
    assert!(
        entities
            .iter()
            .any(|entity| entity["text"] == "via Pascoli" && entity["type"] == "LOCATION"),
        "street address should be retained as a place: {entities:?}"
    );
    let queries = scene["contextual_image_queries"].as_array().unwrap();
    assert!(
        queries
            .iter()
            .all(|query| query["query"] != "Chiara Poggi Alberto Stasi"),
        "mere co-mention must not produce a paired image query: {queries:?}"
    );
}

#[test]
fn visual_search_adds_a_compact_grounded_co_mention_query() {
    let text = "I campioni sui pedali della bicicletta vennero analizzati.";
    let value = manifest_value(vec![scene("query-contesto", text)]);
    let candidates = value["visual_consolidation"]["production_candidates"]
        .as_array()
        .unwrap();
    let search = candidates
        .iter()
        .find(|candidate| {
            candidate["recipe"] == "contextual_search"
                && candidate["title"] == "campioni analizzati"
        })
        .unwrap_or_else(|| panic!("contextual campioni query missing: {candidates:?}"));
    let context = search["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|query| query["origin"] == "derived_from_source_context")
        .unwrap_or_else(|| panic!("grounded contextual rung missing: {}", search["queries"]));
    assert_eq!(context["illustrative"], false);
    let query = context["text"].as_str().unwrap();
    assert!(query.split_whitespace().count() <= 7, "{query}");
    assert_eq!(context["evidence"], text);
    let components = context["components"].as_array().unwrap();
    assert_eq!(components.len(), 2);
    for component in components {
        let start = component["start_byte"].as_u64().unwrap() as usize;
        let end = component["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&text[start..end], component["text"].as_str().unwrap());
    }
    assert!(search["queries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|query| query["origin"] != "derived_from_semantic_template"));
}

#[test]
fn explicit_facility_location_queries_are_supported_across_corpus_languages() {
    let cases = [
        ("it", "Lo Stadio Aurora si trova a Torino."),
        ("en", "Aurora Stadium is located in Torino."),
        ("es", "El Estadio Aurora está ubicado en Torino."),
        ("pt", "O Estádio Aurora está localizado em Torino."),
        ("fr", "Le stade Aurora est situé à Torino."),
        ("de", "Das Stadion Aurora befindet sich in Torino."),
        ("nl", "Het stadion Aurora bevindt zich in Torino."),
        ("pl", "Stadion Aurora znajduje się w Torino."),
        ("ru", "Стадион Aurora находится в Torino."),
        ("id", "Stadion Aurora berada di Torino."),
    ];

    for (language, text) in cases {
        let mut req = request(vec![scene("facility-location", text)]);
        req.language = language.to_string();
        let value = analyze_script(&req).expect("analysis succeeds");
        let scene = serde_json::to_value(value).unwrap()["scenes"][0].clone();
        assert!(
            scene["entities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entity| entity["type"] == "FACILITY"),
            "facility not recognized for {language}: {scene}"
        );
        assert!(
            scene["contextual_image_queries"]
                .as_array()
                .is_some_and(|queries| !queries.is_empty()),
            "explicit location query missing for {language}: {scene}"
        );
    }
}

#[test]
fn a_chapter_heading_line_never_lands_inside_a_query() {
    // The heading is its own sentence, so the source_sentence rung of the
    // first sentence cannot quote it — in the proposal ladder and in the
    // search handshake alike.
    let text = "CAPITOLO SECONDO — LE INDAGINI\nNel 2007 Elena Messina viveva a Pavia.";
    let value = manifest_value(vec![scene("heading-line", text)]);
    let mut queries: Vec<String> = Vec::new();
    for proposal in value["scenes"][0]["visual_proposals"].as_array().unwrap() {
        if let Some(suggestions) = proposal["visual_query_suggestions"].as_array() {
            queries.extend(
                suggestions
                    .iter()
                    .filter_map(|query| query.as_str().map(str::to_string)),
            );
        }
    }
    for entry in value["search_feedback"]["entries"].as_array().unwrap() {
        for query in entry["queries"].as_array().unwrap() {
            if let Some(text) = query["text"].as_str() {
                queries.push(text.to_string());
            }
        }
    }
    assert!(!queries.is_empty(), "the person still produces queries");
    for query in &queries {
        assert!(
            !query.to_lowercase().contains("capitolo secondo"),
            "the heading leaked into a query: {query:?} (all: {queries:?})"
        );
    }
}

#[test]
fn composed_image_queries_keep_the_literal_substring_rule() {
    // A composed query spans grounded components the source states together,
    // and every piece keeps its own provenance. The query text is a verbatim
    // source slice: composition never joins surfaces the source never wrote.
    let text = "Chiara Poggi viveva a Pavia il 13 agosto 2007. Non è così.";
    let value = manifest_value(vec![scene("composte", text)]);
    let scene_value = &value["scenes"][0];
    let queries = scene_value["composed_image_queries"].as_array().unwrap();
    assert!(
        !queries.is_empty(),
        "components stated together compose a query"
    );
    for query in queries {
        let start = query["start_byte"].as_u64().unwrap() as usize;
        let end = query["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(query["query"].as_str().unwrap(), &text[start..end]);
        assert_eq!(query["kind"], "composed_image_query");
        let components = query["components"].as_array().unwrap();
        assert!(
            components.len() >= 2,
            "a composed query needs 2+ components: {query:?}"
        );
        for component in components {
            let c_start = component["start_byte"].as_u64().unwrap() as usize;
            let c_end = component["end_byte"].as_u64().unwrap() as usize;
            // Per-component provenance: verbatim and inside the query span.
            assert_eq!(component["text"].as_str().unwrap(), &text[c_start..c_end]);
            assert!(
                c_start >= start && c_end <= end,
                "component outside the query: {query:?}"
            );
        }
        // The grounded subject and the dated statement are both components.
        assert!(components.iter().any(|c| c["component_type"] == "PERSON"));
        assert!(components.iter().any(|c| c["component_type"] == "DATE"));
    }
    // The filler sentence carries no grounded component pair: no query.
    assert!(
        !queries.iter().any(|query| {
            let start = query["start_byte"].as_u64().unwrap() as usize;
            text[start..].starts_with("Non è così")
        }),
        "noise sentences compose nothing: {queries:?}"
    );
}

#[test]
fn one_component_never_composes_a_query() {
    // Without a second grounded component there is nothing to compose.
    let value = manifest_value(vec![scene("singola", "Milano.")]);
    let scene_value = &value["scenes"][0];
    let queries = scene_value["composed_image_queries"].as_array().unwrap();
    assert!(
        queries
            .iter()
            .all(|query| query["components"].as_array().unwrap().len() >= 2),
        "{queries:?}"
    );
    // "Milano" alone has only one grounded component, so no query at all.
    assert!(
        queries.is_empty(),
        "one component, no composition: {queries:?}"
    );
}

#[test]
fn search_feedback_keeps_exact_queries_and_stays_pending_without_context() {
    let text = "Alberto Stasi torna nel racconto del procedimento.";
    let value = manifest_value(vec![scene("ricerca", text)]);
    let entries = value["search_feedback"]["entries"].as_array().unwrap();
    assert!(!entries.is_empty(), "image-backed proposals need feedback");
    let proposal_ids: Vec<&str> = value["scenes"][0]["visual_proposals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|proposal| proposal["id"].as_str().unwrap())
        .collect();
    for entry in entries {
        let proposal_id = entry["proposal_id"].as_str().unwrap();
        assert!(proposal_ids.contains(&proposal_id), "unknown proposal id");
        assert_eq!(entry["scene_id"], "ricerca");
        assert_eq!(entry["status"], "pending", "only the planner can settle it");
        assert!(entry.get("retry").is_none(), "no failure reported yet");
        let queries = entry["queries"].as_array().unwrap();
        assert!(!queries.is_empty());
        assert_eq!(queries[0]["kind"], "exact_mention");
        assert_eq!(queries[0]["origin"], "literal");
        for query in queries {
            let query_text = query["text"].as_str().unwrap();
            assert!(
                text.contains(query_text),
                "literal query must be source-grounded: {query_text}"
            );
        }
    }
    for entry in entries {
        let proposal_id = entry["proposal_id"].as_str().unwrap();
        let proposal = value["scenes"][0]["visual_proposals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|proposal| proposal["id"] == proposal_id)
            .unwrap();
        let expected = if proposal["proposal_type"] == "entity_search" {
            3
        } else {
            1
        };
        assert_eq!(entry["queries"].as_array().unwrap().len(), expected);
    }
}
