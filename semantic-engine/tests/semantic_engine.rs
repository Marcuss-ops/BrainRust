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
