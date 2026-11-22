use super::*;

fn request(text: &str) -> String {
    json!({
        "operation": "analyze_script",
        "language": "it",
        "scenes": [{"scene_id": "s1", "text": text}]
    })
    .to_string()
}

#[test]
fn ranks_entity_overlays_through_the_worker_protocol() {
    let worker = Worker::new(8);
    let line = json!({
        "operation": "rank_entity_overlays",
        "scenes": [{
            "entities": [
                {"entity_id":"org-a", "overlay_id":"a", "entity_type":"ORG", "confidence":0.9},
                {"entity_id":"person-b", "overlay_id":"b", "entity_type":"PERSON", "confidence":0.8}
            ]
        }]
    })
    .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["selected_indices"], json!([[1, 0]]));
}

#[test]
fn cascades_clustered_entrances_through_the_worker_protocol() {
    let worker = Worker::default();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(
            r#"{"operation":"cascade_clustered_entrances","fps_num":24,"fps_den":1,"items":[{"id":"phrase","kind":"text_phrase","start_ms":0,"end_ms":3000,"motion_params":{"enter_frames":8}},{"id":"image","kind":"entity_image","start_ms":100,"end_ms":3000,"motion_params":{"enter_frames":8}}]}"#,
        )).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["edits"][0]["index"], 1);
    assert_eq!(response["edits"][0]["enter_frames"], 20);
}

#[test]
fn second_identical_request_is_served_from_cache() {
    let worker = Worker::new(8);
    let line = request("Alberto Stasi e Andrea Sempio sono due nomi.");
    let first: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    let second: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(first["ok"], true);
    assert_eq!(first["cached"], false, "first call analyses");
    assert_eq!(second["ok"], true);
    assert_eq!(second["cached"], true, "second identical call is cached");
    assert_eq!(
        first["manifest"]["fingerprint"], second["manifest"]["fingerprint"],
        "cache must serve the same manifest"
    );
}

#[test]
fn generic_visual_concepts_do_not_enter_the_semantic_entity_graph() {
    let response: serde_json::Value = serde_json::from_str(
        &Worker::default().handle_line(&request("Corrado Augias visitò un museo storico.")),
    )
    .unwrap();
    assert_eq!(response["ok"], true);
    let entities = response["manifest"]["scenes"][0]["entities"]
        .as_array()
        .unwrap();
    assert!(entities
        .iter()
        .all(|entity| entity["type"] != "VISUAL_CONCEPT"));
}

#[test]
fn shared_worker_splits_editorial_embedding_passages_with_utf8_offsets() {
    let response: serde_json::Value = serde_json::from_str(&Worker::default().handle_line(
            r#"{"operation":"split_sentences","language":"it","transcript":"L’analisi è pronta. Prosegue il lavoro."}"#,
        ))
        .unwrap();
    assert_eq!(response["ok"], true);
    let sentences = response["sentences"].as_array().unwrap();
    assert_eq!(sentences.len(), 2);
    for sentence in sentences {
        let start = sentence["start_byte"].as_u64().unwrap() as usize;
        let end = sentence["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(
            &"L’analisi è pronta. Prosegue il lavoro."[start..end],
            sentence["text"].as_str().unwrap()
        );
    }
}

#[test]
fn health_reports_cache_occupancy_and_misses() {
    let worker = Worker::new(4);
    let request = json!({
        "operation": "analyze_script",
        "language": "it",
        "scenes": [{"scene_id": "s1", "text": "Garlasco, 42 abitanti."}]
    });
    let first: serde_json::Value =
        serde_json::from_str(&worker.handle_line(&request.to_string())).unwrap();
    assert_eq!(first["ok"], true);
    let health: serde_json::Value =
        serde_json::from_str(&worker.handle_line(r#"{"operation":"health"}"#)).unwrap();
    assert_eq!(health["cache"]["entries"], 1);
    assert_eq!(health["cache"]["misses"], 1);
    assert_eq!(health["cache"]["hits"], 0);
    assert!(health["cache"]["bytes"].as_u64().unwrap() > 0);
}

#[test]
fn health_reports_lexicon_provenance_and_model_slots() {
    let worker = Worker::new(1);
    let response: serde_json::Value =
        serde_json::from_str(&worker.handle_line(r#"{"operation":"health"}"#)).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["lexicon"]["version"],
        semantic_core::lexicon::LEXICON_VERSION
    );
    assert!(["embedded", "override", "mixed"]
        .contains(&response["lexicon"]["source"].as_str().unwrap_or("")));
    assert!(response["lexicon"]["tables_from_override"]
        .as_u64()
        .is_some());
    assert!(response["udpipe_model_slots"].as_u64().is_some());
    assert!(response["udpipe"]["cached_slots"].as_u64().is_some());
    assert!(response["udpipe"]["loaded_languages"].is_array());
    assert!(response["udpipe"]["failed_languages"].is_array());
    assert!(response["udpipe"]["installed_languages"].is_array());
    assert!(response["udpipe"]["missing_languages"].is_array());
    assert!(
        ["disabled", "enabled", "auto"].contains(&response["ner"]["mode"].as_str().unwrap_or(""))
    );
    assert!(response["ner"]["model_installed"].is_boolean());
    assert!(response["ner"]["model_dir"].as_str().is_some());
    assert!(response["ner"]["worker_started"].is_boolean());
    assert!(response["cache"]["hits"].as_u64().is_some());
    assert!(response["cache"]["misses"].as_u64().is_some());
    assert!(response["cache"]["max_bytes"].as_u64().unwrap() > 0);
    assert_eq!(response["cache"]["disk_enabled"], false);
    assert_eq!(
        response["entity_linker_version"],
        semantic_core::entity_linker::linker_version()
    );
    assert_eq!(
        response["entity_linker_weights_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
}

#[test]
fn completeness_reports_partial_and_rejected_manifests() {
    let mut manifest = analyze_script(&ScriptRequest {
        language: "it".to_string(),
        scenes: vec![
            semantic_core::SceneInput {
                scene_id: "s1".to_string(),
                text: "Alberto Stasi entra nel tribunale.".to_string(),
            },
            semantic_core::SceneInput {
                scene_id: "s2".to_string(),
                text: "Il tribunale resta chiuso.".to_string(),
            },
        ],
        editorial: None,
        entity_safety_cap: 0,

        reference_date: None,
    })
    .expect("manifest");
    assert_eq!(manifest_completeness(&manifest)["status"], "complete");

    manifest.scenes[0].status = "rejected".to_string();
    manifest.scenes[0].reason = Some("extractor refused".to_string());
    let partial = manifest_completeness(&manifest);
    assert_eq!(partial["status"], "partial");
    assert_eq!(partial["rejected_scenes"], 1);
    assert_eq!(partial["rejections"][0]["scene_id"], "s1");

    manifest.scenes[1].status = "rejected".to_string();
    assert_eq!(manifest_completeness(&manifest)["status"], "rejected");
}

#[test]
fn fresh_and_cached_wire_responses_both_carry_completeness() {
    let worker = Worker::new(8);
    let line = request("Alberto Stasi e Andrea Sempio sono due nomi.");
    let first: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    let second: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(first["completeness"]["status"], "complete");
    assert_eq!(first["completeness"]["rejected_scenes"], 0);
    assert_eq!(first["completeness"]["rejections"], json!([]));
    assert_eq!(second["cached"], true);
    assert_eq!(
        second["completeness"], first["completeness"],
        "a cache hit must report the same completeness contract"
    );
}

#[test]
fn cached_partial_wire_response_lists_every_rejected_scene() {
    let worker = Worker::new(8);
    let line = serde_json::json!({
        "operation": "analyze_script",
        "language": "it",
        "scenes": [
            {"scene_id": "s1", "text": "Alberto Stasi entra nel tribunale."},
            {"scene_id": "s2", "text": "Il tribunale resta chiuso."}
        ]
    })
    .to_string();
    let request: ScriptRequest = serde_json::from_str(&line).unwrap();
    let key = ManifestCache::request_key(&request, &analyzer_versions());
    let mut manifest = analyze_script(&request).expect("manifest");
    manifest.scenes[1].status = "rejected".to_string();
    manifest.scenes[1].reason = Some("extractor refused scene two".to_string());
    worker
        .cache
        .lock()
        .expect("cache lock")
        .insert(key, Arc::new(manifest));

    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["cached"], true);
    assert_eq!(response["completeness"]["status"], "partial");
    assert_eq!(response["completeness"]["rejected_scenes"], 1);
    assert_eq!(response["completeness"]["rejections"][0]["scene_id"], "s2");
    assert_eq!(
        response["completeness"]["rejections"][0]["reason"],
        "extractor refused scene two"
    );
}

#[test]
fn selects_phrases_through_the_worker_protocol() {
    let worker = Worker::new(8);
    let line = json!({
        "operation": "select_phrases",
        "text": "Ada Lovelace built props. Later, they fix costumes.",
        "blocked_rune_spans": [[0, 12]],
        "limit": 5,
        "profile": {
            "stop_words": ["they", "later"],
            "function_words": ["they", "later"],
            "visual_verbs": ["built", "fix"],
            "min_words": 2,
            "max_words": 16,
            "max_results": 5,
            "reject_verbs_when_all": true
        }
    })
    .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["phrases"][0]["text"], "built props");
    assert_eq!(response["phrases"][1]["text"], "fix costumes");
}

#[test]
fn aligns_phrases_through_the_worker_protocol() {
    let worker = Worker::new(8);
    let line = json!({
        "operation": "align_phrases",
        "words": [
            {"index": 0, "text": "La", "start_us": 0, "end_us": 100},
            {"index": 1, "text": "pena", "start_us": 100, "end_us": 200},
            {"index": 2, "text": "divenne", "start_us": 200, "end_us": 300}
        ],
        "phrases": ["la pena", "pena divenne"]
    })
    .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["alignments"][0]["word_start"], 0);
    assert_eq!(response["alignments"][0]["word_end"], 1);
    assert_eq!(response["alignments"][1]["word_start"], 1);
    assert_eq!(response["alignments"][1]["end_us"], 300);
}

#[test]
fn align_phrases_allow_missing_skips_unknown_values() {
    let worker = Worker::new(8);
    let line = json!({
        "operation": "align_phrases",
        "words": [
            {"index": 0, "text": "La", "start_us": 0, "end_us": 100},
            {"index": 1, "text": "pena", "start_us": 100, "end_us": 200}
        ],
        "phrases": ["la pena", "assente"],
        "allow_missing": true
    })
    .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["alignments"].as_array().unwrap().len(), 1);
    assert_eq!(response["alignments"][0]["text"], "la pena");
}

#[test]
fn align_phrases_all_occurrences_returns_repetitions() {
    let worker = Worker::new(8);
    let line = json!({
        "operation": "align_phrases",
        "words": [
            {"index": 0, "text": "Garibaldi", "start_us": 0, "end_us": 100},
            {"index": 1, "text": "guidò", "start_us": 100, "end_us": 200},
            {"index": 2, "text": "Garibaldi", "start_us": 200, "end_us": 300}
        ],
        "phrases": ["Garibaldi"],
        "all_occurrences": true
    })
    .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], true);
    let alignments = response["alignments"].as_array().unwrap();
    assert_eq!(alignments.len(), 2);
    assert_eq!(alignments[1]["occurrence"], 2);
    assert_eq!(alignments[1]["start_us"], 200);
}

#[test]
fn align_phrases_fails_closed_on_missing_interior_word() {
    let worker = Worker::new(8);
    let line = json!({
        "operation": "align_phrases",
        "words": [
            {"index": 0, "text": "one", "start_us": 0, "end_us": 100},
            {"index": 1, "text": "three", "start_us": 100, "end_us": 200}
        ],
        "phrases": ["one two three"]
    })
    .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .unwrap_or_default()
        .contains("phrase 0"));
}

#[test]
fn compiles_subtitles_through_the_worker_protocol() {
    let worker = Worker::new(8);
    let line = json!({
        "operation": "subtitles_kernel",
        "mode": "compile",
        "style_id": "shorts-v1",
        "cues": [
            {"start_ms": 0, "end_ms": 3280, "text": "hello"},
            {"start_ms": 3280, "end_ms": 6000, "text": "world"}
        ]
    })
    .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["cue_count"], 2);
    assert_eq!(response["last_cue_end_ms"], 6000);
    assert!(response["ass_content"]
        .as_str()
        .unwrap_or_default()
        .contains("Dialogue: 0,0:00:00.00,0:00:03.28,shorts-v1,,0,0,0,,hello"));
}

#[test]
fn normalizes_cues_through_the_worker_protocol() {
    let worker = Worker::new(8);
    let line = json!({
            "operation": "subtitles_kernel",
            "mode": "normalize",
            "cues": [
                {"start_ms": 0, "end_ms": 5200, "text": "where he can tell what colored underwear a man is wearing just from looking at him"}
            ]
        })
        .to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], true);
    assert!(response["cues"].as_array().unwrap().len() >= 2);
}

#[test]
fn rejects_unknown_subtitles_kernel_mode() {
    let worker = Worker::new(8);
    let line = json!({"operation": "subtitles_kernel", "mode": "bogus"}).to_string();
    let response: serde_json::Value = serde_json::from_str(&worker.handle_line(&line)).unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"]
        .as_str()
        .unwrap_or_default()
        .contains("unsupported mode"));
}

#[test]
fn different_text_is_not_served_from_cache() {
    let worker = Worker::new(8);
    let a: serde_json::Value =
        serde_json::from_str(&worker.handle_line(&request("Primo testo."))).unwrap();
    let b: serde_json::Value =
        serde_json::from_str(&worker.handle_line(&request("Secondo testo."))).unwrap();
    assert_eq!(a["cached"], false);
    assert_eq!(b["cached"], false, "different text must miss the cache");
}

#[test]
fn error_envelopes_carry_a_machine_readable_code() {
    let worker = Worker::default();

    let decode: serde_json::Value = serde_json::from_str(&worker.handle_line("not json")).unwrap();
    assert_eq!(decode["ok"], false);
    assert_eq!(decode["error_code"], "decode");

    let unsupported: serde_json::Value =
        serde_json::from_str(&worker.handle_line(r#"{"operation":"nope"}"#)).unwrap();
    assert_eq!(unsupported["ok"], false);
    assert_eq!(unsupported["error_code"], "unsupported_operation");

    let execution: serde_json::Value = serde_json::from_str(
        &worker.handle_line(r#"{"operation":"split_sentences","transcript":"ciao","language":""}"#),
    )
    .unwrap();
    assert_eq!(execution["ok"], false);
    assert_eq!(execution["error_code"], "execution");
    assert_eq!(execution["error"], "split_sentences language is required");
}
