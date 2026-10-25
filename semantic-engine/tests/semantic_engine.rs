use semantic_engine::protocol::handle_line;
use semantic_engine::{analyze_script, SceneInput, ScriptRequest};
use serde_json::{json, Value};

fn scene(id: &str, text: &str) -> SceneInput {
    SceneInput {
        scene_id: id.to_string(),
        text: text.to_string(),
    }
}

fn request(scenes: Vec<SceneInput>) -> ScriptRequest {
    ScriptRequest {
        language: "it".to_string(),
        scenes,
        editorial: None,
        entity_safety_cap: 0,
    }
}

fn manifest_value(scenes: Vec<SceneInput>) -> Value {
    let manifest = analyze_script(&request(scenes)).expect("analysis must succeed");
    serde_json::to_value(manifest).expect("manifest serializes")
}

fn proposals_of<'a>(scene: &'a Value, kind: &str) -> Vec<&'a Value> {
    scene["visual_proposals"]
        .as_array()
        .expect("visual_proposals array")
        .iter()
        .filter(|c| c["proposal_type"] == kind)
        .collect()
}

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
fn one_script_batch_can_return_phrase_impact_editorial_products() {
    let text = "Il gruppo prepara il progetto e controlla ogni passaggio. Gli operatori registrano i risultati e verificano tutte le procedure. La squadra completa il lavoro e pubblica il rapporto finale.";
    let mut req = request(vec![scene("editorial", text)]);
    req.editorial = Some(
        serde_json::from_value(serde_json::json!({
            "request": {
                "transcript": text,
                "language": "it",
                "embeddings": [],
                "lexical_only": true,
                "stopwords": []
            },
            "scenes": [{"scene_id":"editorial", "text":text, "start_byte":0, "end_byte":text.len()}],
        }))
        .expect("editorial request follows the shared Phrase Impact contract"),
    );

    let manifest = analyze_script(&req).expect("combined analysis must succeed");
    let editorial = manifest
        .editorial
        .expect("editorial result belongs to same manifest");
    assert!(!editorial.summary.is_empty());
    assert_eq!(
        editorial.chapter_manifest.schema_version,
        "chapter_manifest.v1"
    );
    assert!(editorial
        .scene_highlights
        .iter()
        .any(|highlight| highlight.scene_id == "editorial"));
    let semantic_bullet_texts: std::collections::BTreeSet<_> = manifest.scenes[0]
        .bullets
        .iter()
        .map(|bullet| bullet.text.as_str())
        .collect();
    assert!(editorial
        .bullet_points
        .iter()
        .all(|bullet| semantic_bullet_texts.contains(bullet.text.as_str())));
}

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
    let tifosi = searches
        .iter()
        .find(|proposal| proposal["visual_query_suggestions"][0] == "tifosi")
        .expect("the source-grounded supporter mention gets a search proposal");
    let queries = tifosi["visual_query_suggestions"]
        .as_array()
        .expect("query suggestions array");
    assert_eq!(queries[0], "tifosi", "preserve the exact entity query");
    assert_eq!(
        queries[1], "I tifosi festeggiano sugli spalti dopo",
        "offer an exact multiword source phrase as a provider-independent context query"
    );
    assert!(text.contains(queries[1].as_str().unwrap()));
    assert!(queries.iter().any(|query| query == text));
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
fn explicit_collaboration_marker_produces_a_grounded_asserted_relation() {
    let text = "Alberto Stasi collabora con Andrea Sempio.";
    let value = manifest_value(vec![scene("collaborazione", text)]);
    let relations = value["scenes"][0]["relations"]["asserted_relations"]
        .as_array()
        .unwrap();
    assert_eq!(relations.len(), 1, "explicit predicate only: {value}");
    assert_eq!(relations[0]["predicate"], "collaboration");
    assert_eq!(relations[0]["status"], "Asserted");
    assert_eq!(relations[0]["evidence"], text);
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
fn ten_thousand_word_script_keeps_every_canonical_sentence_candidate() {
    let mut text = String::with_capacity(100_000);
    for index in 0..2_500 {
        if index > 0 {
            text.push(' ');
        }
        text.push_str("La storia continua oggi.");
    }
    assert_eq!(text.split_whitespace().count(), 10_000);
    let value = manifest_value(vec![scene("copione-lungo", &text)]);
    let scene = &value["scenes"][0];
    let phrases = scene["short_phrases"].as_array().unwrap();
    assert_eq!(phrases.len(), 2_500, "the full script must be represented");
    for (index, phrase) in phrases.iter().enumerate() {
        assert_eq!(phrase["sentence_index"], index);
        let start = phrase["start_byte"].as_u64().unwrap() as usize;
        let end = phrase["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), phrase["evidence"].as_str());
    }
}

#[test]
fn r09_two_percentages_yield_a_chart_with_both_values() {
    let text =
        "Il 42% dei tecnici ha segnalato il problema, contro il 18% del gruppo di controllo.";
    let value = manifest_value(vec![scene("confronto", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");

    assert_eq!(charts.len(), 1, "one chart expected: {value}");
    let mut values: Vec<f64> = charts[0]["chart_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["value"].as_f64().unwrap())
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(values, vec![18.0, 42.0]);
    assert_eq!(charts[0]["suggested_visual"], "bar_comparison");
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
fn spelled_percentages_form_one_chart_of_both_values() {
    // T05: spelled values ("quarantadue per cento") resolve exactly like
    // digits ("42%") when they state their unit.
    let text = "Il quarantadue per cento dei tecnici ha segnalato il problema, \
                contro il diciotto per cento del gruppo di controllo.";
    let value = manifest_value(vec![scene("confronto-scritto", text)]);
    let charts = proposals_of(&value["scenes"][0], "chart");
    assert_eq!(charts.len(), 1, "one chart expected: {value}");
    let mut values: Vec<f64> = charts[0]["chart_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["value"].as_f64().unwrap())
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(values, vec![18.0, 42.0]);
}

#[test]
fn a_spelled_value_without_a_unit_enters_no_numeric_projection() {
    // A spelled quantity must state its unit: "quarantadue" alone is not a
    // measurement, so it is neither a chart value nor a numeric fact.
    let text = "Il quarantadue dei tecnici ha segnalato il problema, \
                contro il diciotto del gruppo di controllo.";
    let value = manifest_value(vec![scene("senza-unita", text)]);
    let scene = &value["scenes"][0];
    assert!(proposals_of(scene, "chart").is_empty(), "{value}");
    assert!(
        scene["numeric_facts"].as_array().unwrap().is_empty(),
        "no unit, no numeric fact: {}",
        scene["numeric_facts"]
    );
}

#[test]
fn different_kinds_never_form_a_chart() {
    // Charts compare same-kind values: a percent against money is not a
    // comparison, whatever the comparison cue says.
    let text = "Il 42% dei tecnici ha speso 5.000 euro contro il piano iniziale.";
    let value = manifest_value(vec![scene("tipi-misti", text)]);
    assert!(proposals_of(&value["scenes"][0], "chart").is_empty(), "{value}");
}

#[test]
fn single_number_never_becomes_a_chart() {
    let text = "Il 42% dei tecnici ha segnalato il problema durante la prova.";
    let value = manifest_value(vec![scene("singolo", text)]);
    assert!(proposals_of(&value["scenes"][0], "chart").is_empty());
}

#[test]
fn unrelated_values_in_one_sentence_do_not_become_a_chart_without_comparison_language() {
    let text = "Il 42% dei clienti ha risposto; il 18% del budget è stato destinato alla ricerca.";
    let value = manifest_value(vec![scene("valori-separati", text)]);
    assert!(proposals_of(&value["scenes"][0], "chart").is_empty());
}

#[test]
fn r08_dates_are_kept_and_no_chart_is_invented() {
    let text = "Il 15 marzo 2008 è la data del delitto. Il 1 giugno 2009 inizia il processo.";
    let value = manifest_value(vec![scene("cronologia", text)]);
    let scene = &value["scenes"][0];

    let facts = scene["temporal_facts"].as_array().unwrap();
    assert!(facts.len() >= 2, "both dates must be kept: {scene}");
    assert!(proposals_of(scene, "chart").is_empty());
}

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
fn explicit_comparison_cue_creates_a_grounded_comparison_group() {
    let text = "Alberto Stasi versus Andrea Sempio.";
    let value = manifest_value(vec![scene("confronto", text)]);
    let group = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["group_type"] == "comparison")
        .expect("explicit comparison marker and two entities should form a group");
    let members = group["members"].as_array().unwrap();
    assert_eq!(
        members.len(),
        2,
        "unexpected comparison members: {members:?}"
    );
    for member in members {
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn co_mention_without_explicit_comparison_cue_does_not_form_a_comparison_group() {
    let text = "Alberto Stasi and Andrea Sempio are mentioned in the same report.";
    let value = manifest_value(vec![scene("co-mention", text)]);
    assert!(value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .all(|group| group["group_type"] != "comparison"));
}

#[test]
fn repeated_explicit_comparison_links_grounded_members_across_scenes() {
    let inputs = vec![
        scene(
            "comparison-first",
            "Lo Stadio Aurora è più capiente rispetto allo Stadio Olimpico.",
        ),
        scene(
            "comparison-second",
            "Lo Stadio Aurora è più antico rispetto allo Stadio Olimpico.",
        ),
    ];
    let value = manifest_value(inputs.clone());
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let group = groups
        .iter()
        .find(|group| {
            group["group_type"] == "comparison"
                && group["ranking_basis"] == "explicit_pair_comparison_across_scenes"
        })
        .unwrap_or_else(|| {
            panic!("same explicitly compared identity pair should be joined across scenes: {value}")
        });
    assert_eq!(group["members"].as_array().unwrap().len(), 4);
    for member in group["members"].as_array().unwrap() {
        let scene_id = member["scene_id"].as_str().unwrap();
        let source = &inputs
            .iter()
            .find(|input| input.scene_id == scene_id)
            .expect("member scene must belong to this request")
            .text;
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(source.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn repeated_co_mentions_without_explicit_comparison_do_not_form_cross_scene_comparison() {
    let inputs = vec![
        scene(
            "co-mention-first",
            "Stadio Aurora e Stadio Olimpico sono impianti distinti.",
        ),
        scene(
            "co-mention-second",
            "Stadio Aurora e Stadio Olimpico sono stati citati.",
        ),
    ];
    let value = manifest_value(inputs);
    assert!(value["scenes"].as_array().unwrap().iter().all(|scene| {
        scene["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| {
                group["group_type"] != "comparison"
                    || group["ranking_basis"] != "explicit_pair_comparison_across_scenes"
            })
    }));
}

#[test]
fn asserted_causal_relation_produces_a_cause_effect_group() {
    let text = "Lo Stadio Aurora causa l'espansione di Torino.";
    let value = manifest_value(vec![scene("causa-effetto", text)]);
    let group = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["group_type"] == "cause_effect")
        .expect("explicit positive causal relation should form a group");
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
}

#[test]
fn negated_or_uncertain_causal_relation_does_not_form_a_cause_effect_group() {
    for text in [
        "Lo Stadio Aurora non causa l'espansione di Torino.",
        "Forse lo Stadio Aurora causa l'espansione di Torino.",
    ] {
        let value = manifest_value(vec![scene("causa-non-certa", text)]);
        assert!(value["scenes"][0]["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["group_type"] != "cause_effect"));
    }
}

#[test]
fn explicit_causal_chain_links_grounded_evidence_across_scenes() {
    let inputs = vec![
        scene(
            "cause-origin",
            "Lo Stadio Aurora causa la crescita di Torino.",
        ),
        scene("cause-next", "Torino causa la crescita di Milano."),
    ];
    let value = manifest_value(inputs.clone());
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let group = groups
        .iter()
        .find(|group| {
            group["group_type"] == "cause_effect"
                && group["ranking_basis"] == "explicit_asserted_causal_chain_across_scenes"
        })
        .expect("explicit causal edges sharing an endpoint should form a cross-scene chain");
    assert_eq!(group["members"].as_array().unwrap().len(), 4);
    for member in group["members"].as_array().unwrap() {
        let scene_id = member["scene_id"].as_str().unwrap();
        let source = &inputs
            .iter()
            .find(|input| input.scene_id == scene_id)
            .expect("member scene must belong to this request")
            .text;
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(source.get(start..end), member["evidence"].as_str());
    }
}

#[test]
fn negated_second_causal_edge_does_not_form_cross_scene_chain() {
    let inputs = vec![
        scene(
            "cause-origin",
            "Lo Stadio Aurora causa la crescita di Torino.",
        ),
        scene("cause-next", "Torino non causa la crescita di Milano."),
    ];
    let value = manifest_value(inputs);
    assert!(value["scenes"].as_array().unwrap().iter().all(|scene| {
        scene["semantic_groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| {
                group["group_type"] != "cause_effect"
                    || group["ranking_basis"] != "explicit_asserted_causal_chain_across_scenes"
            })
    }));
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
fn fictional_stadium_date_and_capacity_facts_link_to_the_facility() {
    let text = "Stadio Aurora fu inaugurato nel 1959, ha capienza superiore a 10.000 posti, è di proprietà della famiglia Rossi e si trova a Torino.";
    let value = manifest_value(vec![scene("stadio-aurora", text)]);
    let scene = &value["scenes"][0];
    let facility = scene["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["type"] == "FACILITY")
        .expect("explicitly named facility should be recognized");
    let facts = scene["entity_facts"].as_array().unwrap();
    let inauguration = facts
        .iter()
        .find(|fact| fact["predicate"] == "inaugurated_year")
        .unwrap();
    assert_eq!(inauguration["subject_entity_id"], facility["id"]);
    assert_eq!(inauguration["value"], "1959");
    let capacity = facts
        .iter()
        .find(|fact| fact["predicate"] == "capacity")
        .unwrap();
    assert_eq!(capacity["subject_entity_id"], facility["id"]);
    assert_eq!(capacity["normalized_value"], "10000");
    assert_eq!(capacity["operator"], ">");
    assert_eq!(capacity["unit"], "seats");
    for predicate in ["owned_by", "located_in"] {
        let linked = facts
            .iter()
            .find(|fact| fact["predicate"] == predicate)
            .unwrap();
        assert_eq!(linked["subject_entity_id"], facility["id"]);
        assert_eq!(linked["type"], "ENTITY");
        assert!(linked["target_entity_id"].as_str().is_some());
    }
    let asserted = scene["relations"]["asserted_relations"].as_array().unwrap();
    assert_eq!(
        asserted.len(),
        2,
        "only explicit ownership/location edges: {asserted:?}"
    );
    assert!(asserted
        .iter()
        .all(|relation| relation["status"] == "Asserted"));
    for fact in facts {
        let start = fact["start_byte"].as_u64().unwrap() as usize;
        let end = fact["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), fact["value"].as_str());
        let evidence_start = fact["evidence_start_byte"].as_u64().unwrap() as usize;
        let evidence_end = fact["evidence_end_byte"].as_u64().unwrap() as usize;
        assert_eq!(
            text.get(evidence_start..evidence_end),
            fact["evidence"].as_str()
        );
    }
}

#[test]
fn explicit_negated_ownership_is_not_reported_as_a_positive_relation() {
    let text = "Stadio Aurora non è di proprietà della famiglia Rossi e si trova a Torino.";
    let value = manifest_value(vec![scene("proprieta-negata", text)]);
    let scene = &value["scenes"][0];
    let relations = scene["relations"]["asserted_relations"].as_array().unwrap();
    assert_eq!(relations.len(), 2, "both explicit assertions are preserved");
    let ownership = relations
        .iter()
        .find(|edge| edge["predicate"] == "ownership")
        .unwrap();
    let location = relations
        .iter()
        .find(|edge| edge["predicate"] == "location")
        .unwrap();
    assert_eq!(ownership["status"], "Negated");
    assert_eq!(location["status"], "Asserted");
}

#[test]
fn unrelated_dates_and_values_do_not_create_a_semantic_group() {
    let text = "La scheda cita il 1950 accanto al nome di Marco Rossi e riporta 18%. Il fascicolo menziona il 1974 e il valore di 42%.";
    let value = manifest_value(vec![scene("non-collegati", text)]);
    let scene = &value["scenes"][0];
    assert!(scene["semantic_groups"].as_array().unwrap().is_empty());
    assert!(proposals_of(scene, "chart").is_empty());
}

#[test]
fn explicit_top_ten_reports_incomplete_lists_without_filling_them() {
    let mut incomplete = String::from("Top 10 momenti importanti:\n");
    for rank in (5..=10).rev() {
        incomplete.push_str(&format!("{rank}. Momento {rank} descritto nel copione.\n"));
    }
    let value = manifest_value(vec![scene("top-10-incompleta", &incomplete)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["group_type"], "ranking");
    assert_eq!(groups[0]["target_count"], 10);
    assert_eq!(groups[0]["complete"], false);
    assert_eq!(groups[0]["members"].as_array().unwrap().len(), 6);

    let mut complete = String::from("Top 10 momenti importanti:\n");
    for rank in (1..=10).rev() {
        complete.push_str(&format!("{rank}. Momento {rank} descritto nel copione.\n"));
    }
    let value = manifest_value(vec![scene("top-10-completa", &complete)]);
    let group = &value["scenes"][0]["semantic_groups"][0];
    assert_eq!(group["complete"], true);
    assert_eq!(group["members"].as_array().unwrap().len(), 10);
}

#[test]
fn facility_facts_survive_anaphoric_sentences_without_duplicating_capacity() {
    let text = "Lo Stadio Aurora è un impianto sportivo di Torino. Lo Stadio Aurora fu inaugurato nel 1959. La capienza è di oltre 10.000 posti e l'impianto è di proprietà della famiglia Rossi.";
    let value = manifest_value(vec![scene("stadio-anaphora", text)]);
    let scene = &value["scenes"][0];
    let facility_id = scene["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["type"] == "FACILITY")
        .and_then(|entity| entity["id"].as_str())
        .expect("facility identity");
    let facts = scene["entity_facts"].as_array().unwrap();
    // The capacity sentence does not repeat the stadium name: a scene with a
    // single facility still attributes its facts to that facility.
    let capacities: Vec<&Value> = facts
        .iter()
        .filter(|fact| fact["predicate"] == "capacity")
        .collect();
    assert_eq!(
        capacities.len(),
        1,
        "overlapping numeric spans must yield one capacity fact: {facts:?}"
    );
    assert_eq!(capacities[0]["unit"], "seats");
    assert_eq!(capacities[0]["operator"], ">");
    for predicate in ["inaugurated_year", "owned_by", "located_in"] {
        let fact = facts
            .iter()
            .find(|fact| fact["predicate"] == predicate)
            .unwrap_or_else(|| panic!("missing {predicate}: {facts:?}"));
        assert_eq!(fact["subject_entity_id"], facility_id);
    }
    let location = facts
        .iter()
        .find(|fact| fact["predicate"] == "located_in")
        .unwrap();
    assert_eq!(location["value"], "Torino");
    assert!(location["target_entity_id"].as_str().is_some());
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
fn inline_numbered_top_ten_is_one_complete_grounded_ranking() {
    let items: Vec<String> = (1..=10)
        .map(|rank| format!("{rank}. Momento numero {rank} del racconto."))
        .collect();
    let text = format!("Top 10 momenti importanti: {}", items.join(" "));
    let value = manifest_value(vec![scene("top-10-inline", &text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let ranking = groups
        .iter()
        .find(|group| group["group_type"] == "ranking")
        .unwrap_or_else(|| panic!("inline ranking expected: {groups:?}"));
    assert_eq!(ranking["target_count"], 10);
    assert_eq!(ranking["complete"], true);
    let members = ranking["members"].as_array().unwrap();
    assert_eq!(members.len(), 10);
    for (index, member) in members.iter().enumerate() {
        let rank = index + 1;
        assert_eq!(member["rank"], rank);
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
        assert!(
            member["text"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{rank}.")),
            "each item keeps its own rank marker: {}",
            member["text"]
        );
        if rank < 10 {
            assert!(
                !member["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!(" {}. ", rank + 1)),
                "an item must not absorb the next one: {}",
                member["text"]
            );
        }
    }
}

#[test]
fn inline_numbered_list_without_a_declared_top_ten_stays_an_enumeration() {
    let text = "Le fasi sono: 1. Progettazione. 2. Costruzione. 3. Collaudo.";
    let value = manifest_value(vec![scene("elenco-inline", text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let enumeration = groups
        .iter()
        .find(|group| group["group_type"] == "enumeration")
        .unwrap_or_else(|| panic!("enumeration expected: {groups:?}"));
    assert_eq!(enumeration["members"].as_array().unwrap().len(), 3);
    assert!(enumeration.get("complete").is_none());
}

#[test]
fn bullet_candidates_carry_the_shared_semantic_salience() {
    let text =
        "Torino. Lo Stadio Aurora di Torino ha 10.000 posti e appartiene alla famiglia Rossi.";
    let value = manifest_value(vec![scene("bullets-score", text)]);
    let bullets = value["scenes"][0]["bullets"].as_array().unwrap();
    assert_eq!(bullets.len(), 2, "both entity sentences are candidates");
    assert!(bullets.iter().all(|bullet| {
        let score = bullet["score"].as_f64().unwrap();
        (0.0..=1.0).contains(&score) && bullet["provenance"] == "semantic_core_bullets"
    }));
    let sparse = bullets[0]["score"].as_f64().unwrap();
    let rich = bullets[1]["score"].as_f64().unwrap();
    assert!(
        rich > sparse,
        "the evidence-rich sentence must outrank the fragment: {bullets:?}"
    );
}

#[test]
fn filler_sentences_are_never_bullets() {
    // A bullet needs a grounded signal: discourse noise stays out of the
    // editorial bullets while the named sentence remains the only candidate.
    let text = "Alberto Stasi viveva a Pavia. Non è così. Lui. Lei.";
    let value = manifest_value(vec![scene("bullets-rumore", text)]);
    let bullets = value["scenes"][0]["bullets"].as_array().unwrap();
    assert_eq!(bullets.len(), 1, "only the grounded sentence: {bullets:?}");
    assert_eq!(bullets[0]["text"], "Alberto Stasi viveva a Pavia.");
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
    assert_eq!(first["schema_version"], "semantic_manifest.v2");
    assert_eq!(first["fingerprint"].as_str().unwrap().len(), 16);
    assert_eq!(first["analyzers"]["visualner"], "visualner");
}

#[test]
fn chart_items_are_verbatim_source_slices() {
    let text =
        "Il 42% dei tecnici ha segnalato il problema, contro il 18% del gruppo di controllo.";
    let value = manifest_value(vec![scene("confronto", text)]);
    let chart = &proposals_of(&value["scenes"][0], "chart")[0];
    for item in chart["chart_items"].as_array().unwrap() {
        let start = item["start_byte"].as_u64().unwrap() as usize;
        let end = item["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), item["text"].as_str());
        assert!(matches!(
            item["kind"].as_str(),
            Some("NUMBER" | "PERCENT" | "MONEY")
        ));
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
        phrases.iter().any(|phrase| phrase.contains("CAPITOLO SECONDO")),
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
fn important_segments_stay_on_linguistic_boundaries() {
    // Segments are 2–5 consecutive sentences cut at the canonical segmenter's
    // boundaries, anchored by grounded signals, non-overlapping and verbatim.
    let text = "Chiara Poggi viveva a Pavia. Il 13 agosto 2007 il caso cambiò. \
                La Procura di Pavia aprì un'indagine. Alberto Stasi fu rinviato \
                a giudizio. Non è così. Il processo iniziò nel 2009.";
    let value = manifest_value(vec![scene("segmenti", text)]);
    let scene_value = &value["scenes"][0];
    let segments = scene_value["important_segments"].as_array().unwrap();
    assert!(!segments.is_empty(), "a signal-rich scene has segments");
    assert!(segments.len() <= 3, "bounded selection: {segments:?}");

    let spans = semantic_core::split_sentences(text, "it", None);
    let mut ends: Vec<(usize, usize)> = Vec::new();
    for segment in segments {
        let start = segment["sentence_index_start"].as_u64().unwrap() as usize;
        let end = segment["sentence_index_end"].as_u64().unwrap() as usize;
        assert!(end >= start && end - start + 1 >= 2 && end - start + 1 <= 5,
            "2..=5 sentences expected: {segment:?}");
        // Linguistic boundaries: the span starts at the first sentence's start
        // and ends at the last sentence's end.
        assert_eq!(segment["start_byte"].as_u64().unwrap() as usize, spans[start].start_byte);
        assert_eq!(segment["end_byte"].as_u64().unwrap() as usize, spans[end].end_byte);
        // Verbatim source slice (NO EVIDENCE → NO CANDIDATE).
        let start_byte = segment["start_byte"].as_u64().unwrap() as usize;
        let end_byte = segment["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(segment["evidence"].as_str().unwrap(), &text[start_byte..end_byte]);
        assert_eq!(segment["kind"], "important_segment");
        ends.push((start_byte, end_byte));
    }
    // Segments never overlap.
    ends.sort_unstable();
    for pair in ends.windows(2) {
        assert!(pair[0].1 <= pair[1].0, "segments overlap: {segments:?}");
    }
    // Deterministic replay: the same request yields the same segments.
    let again = manifest_value(vec![scene("segmenti", text)]);
    assert_eq!(
        again["scenes"][0]["important_segments"], scene_value["important_segments"]
    );
}

#[test]
fn a_scene_without_signals_keeps_no_segments() {
    // A successful analysis with no grounded signals stays empty instead of
    // inventing passages.
    let value = manifest_value(vec![scene("vuota-di-segnali", "Non è così. Non era così. Non sarà così.")]);
    let scene = &value["scenes"][0];
    assert_eq!(scene["status"], "valid");
    assert!(
        scene["important_segments"].as_array().unwrap().is_empty(),
        "no signals, no segments: {}",
        scene["important_segments"]
    );
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
    assert!(!queries.is_empty(), "components stated together compose a query");
    for query in queries {
        let start = query["start_byte"].as_u64().unwrap() as usize;
        let end = query["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(query["query"].as_str().unwrap(), &text[start..end]);
        assert_eq!(query["kind"], "composed_image_query");
        let components = query["components"].as_array().unwrap();
        assert!(components.len() >= 2, "a composed query needs 2+ components: {query:?}");
        for component in components {
            let c_start = component["start_byte"].as_u64().unwrap() as usize;
            let c_end = component["end_byte"].as_u64().unwrap() as usize;
            // Per-component provenance: verbatim and inside the query span.
            assert_eq!(component["text"].as_str().unwrap(), &text[c_start..c_end]);
            assert!(c_start >= start && c_end <= end, "component outside the query: {query:?}");
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
        queries.iter().all(|query| query["components"].as_array().unwrap().len() >= 2),
        "{queries:?}"
    );
    // "Milano" alone has only one grounded component, so no query at all.
    assert!(queries.is_empty(), "one component, no composition: {queries:?}");
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

#[test]
fn entity_facts_carry_importance_and_visual_concept_ids_deterministically() {
    let text = "Il Tribunale di Torino ha assolto Mario Rossi nel 2009. Mario Rossi è stato assolto in appello.";
    let first = manifest_value(vec![scene("fatti-importanza", text)]);
    let second = manifest_value(vec![scene("fatti-importanza", text)]);
    assert_eq!(first, second, "same input must give the same manifest");

    let facts = first["scenes"][0]["entity_facts"].as_array().unwrap();
    assert!(!facts.is_empty(), "the acquittal must produce a fact");
    for fact in facts {
        let importance = fact["importance"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&importance), "importance in [0,1]");
        let ids = fact["visual_concept_ids"].as_array().unwrap();
        assert_eq!(ids[0], fact["subject_entity_id"], "subject leads the ids");
        if let Some(target) = fact["target_entity_id"].as_str() {
            assert!(ids.iter().any(|id| id == target), "target id is bound");
        }
    }
    let outcome = facts
        .iter()
        .find(|fact| fact["normalized_value"] == "acquitted")
        .expect("acquitted outcome fact");
    assert_eq!(outcome["importance"].as_f64().unwrap(), 0.9_f32 as f64);
}

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
fn a_lone_dated_legal_fact_gets_a_date_card() {
    // "emise la sua decisione" is an event the predicate list does not name
    // (it is tuned for construction and outcome stems), but the sentence does
    // assert a dated fact. Without the card the fact had no representation.
    let text = "È fondamentale ricordare la data del 17 dicembre 2009, giorno in cui il Giudice dell'Udienza Preliminare emise la sua decisione di assoluzione nei confronti di Alberto Stasi.";
    let value = manifest_value(vec![scene("gup", text)]);
    let timelines = proposals_of(&value["scenes"][0], "timeline");
    assert_eq!(
        timelines.len(),
        1,
        "one dated event, one card: {timelines:?}"
    );
    assert_eq!(timelines[0]["suggested_visual"], "date_card");
    assert_eq!(
        value["visual_coverage"]["covered_facts"], value["visual_coverage"]["total_facts"],
        "the card represents the dated fact: {}",
        value["visual_coverage"]
    );
    for fact in value["visual_coverage"]["scenes"][0]["facts"]
        .as_array()
        .unwrap()
    {
        assert_eq!(fact["status"], "covered", "every dated fact is covered");
    }
}

#[test]
fn a_date_without_an_asserted_fact_gets_no_card() {
    let text = "Il 17 dicembre 2009 il cielo sopra il paese era limpido.";
    let value = manifest_value(vec![scene("cielo", text)]);
    let timelines = proposals_of(&value["scenes"][0], "timeline");
    assert!(
        timelines.is_empty(),
        "a bare date is not an event: {timelines:?}"
    );
    assert_eq!(value["visual_coverage"]["total_facts"], 0);
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
fn search_feedback_offers_a_widening_ladder_and_stays_pending() {
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
        for query in queries {
            let query_text = query["text"].as_str().unwrap();
            assert!(
                text.contains(query_text),
                "every rung is an exact source slice: {query_text}"
            );
        }
    }
    let search = entries
        .iter()
        .find(|entry| entry["queries"].as_array().unwrap().len() == 3)
        .expect("an entity search widens to phrase and sentence");
    assert_eq!(search["queries"][1]["kind"], "context_phrase");
    assert_eq!(search["queries"][2]["kind"], "source_sentence");
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

#[test]
fn a_noun_list_with_a_comparison_cue_is_not_a_comparison() {
    // The cue alone used to turn every entity of the sentence into a member,
    // so a paragraph listing eleven nouns became an eleven-way comparison.
    let text = "La vicenda è più complicata rispetto alla cronologia, e coinvolge \
                colpevolezza, diritto, procedura, testimonianze, atti, sentenze, \
                perizie, indagini e dibattimento.";
    let value = manifest_value(vec![scene("noun-list", text)]);
    let groups = &value["scenes"][0]["semantic_groups"];
    assert!(
        groups
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["group_type"] != "comparison"),
        "generic nouns are not comparable subjects: {groups}"
    );
}

#[test]
fn a_comparison_has_exactly_the_two_named_subjects_around_the_cue() {
    let text = "Garlasco è più piccolo rispetto a Pavia, ma il caso ha avuto la stessa risonanza.";
    let value = manifest_value(vec![scene("two-places", text)]);
    let group = value["scenes"][0]["semantic_groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["group_type"] == "comparison")
        .unwrap_or_else(|| panic!("two named places with a cue must compare: {value}"));
    let members = group["members"].as_array().unwrap();
    let texts: Vec<&str> = members
        .iter()
        .map(|member| member["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, vec!["Garlasco", "Pavia"], "members: {members:?}");
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
