//! Newline-delimited JSON protocol over stdin/stdout, one response per request.
//!
//! Requests: `{"operation":"health"}` and
//! `{"operation":"analyze_script","language":"it","scenes":[{"scene_id":"s1","text":"..."}]}`.
//! Responses: `{"ok":true,...}` or `{"ok":false,"error":"..."}`.
//!
//! The worker is persistent, so it keeps one [`ManifestCache`] across requests:
//! re-analysing the same script text (a replay, or a repeated job) is served
//! from the cache instead of re-running the extractor.

use std::io;
use std::sync::{Arc, Mutex};

use semantic_core::phrase_align::{locate_phrase, LocatedPhrase, SpeechWord};
use semantic_core::phrase_select::{
    select_phrases, PhraseExtractionPolicy, PhraseProfile, SelectedPhrase,
};
use semantic_core::SemanticManifest;
use serde::Deserialize;
use serde_json::json;

use crate::cache::ManifestCache;
use crate::{analyze_script, analyzer_versions, ScriptRequest, SCHEMA_VERSION};
use overlay_compiler::{
    cascade_clustered_entrances, compile_editorial_plan, compile_overlay_plan_v2,
    rank_entity_overlays, resolve_map_overlaps, trim_map_windows_before_budget,
    CascadeEntrancesRequest, CompileEditorialPlanRequest, EditorialPlanV2Request,
    RankEntityOverlaysRequest, ResolveMapOverlapsRequest, EDITORIAL_PLAN_V2_SCHEMA_VERSION,
};

/// Entries kept per persistent worker. Large enough that a full script batch
/// plus its replay stay warm, small enough to bound RSS.
const CACHE_ENTRIES: usize = 256;

#[derive(Deserialize)]
struct Envelope {
    operation: String,
}

#[derive(Deserialize)]
struct SplitRequest {
    transcript: String,
    language: String,
}

#[derive(Deserialize, Default)]
struct SelectPhrasesProfileRequest {
    #[serde(default)]
    stop_words: Vec<String>,
    #[serde(default)]
    function_words: Vec<String>,
    #[serde(default)]
    visual_verbs: Vec<String>,
    #[serde(default)]
    min_words: Option<usize>,
    #[serde(default)]
    max_words: Option<usize>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    reject_verbs_when_all: Option<bool>,
}

#[derive(Deserialize, Default)]
struct AlignPhrasesRequest {
    #[serde(default)]
    words: Vec<SpeechWord>,
    #[serde(default)]
    phrases: Vec<String>,
    // `allow_missing` keeps the batch usable for optional entity/moment
    // annotations: a value the speech never said is skipped, not a failure
    // for the whole scene. The default stays fail-closed for phrase timing.
    #[serde(default)]
    allow_missing: bool,
    // `all_occurrences` returns every document-order match. Phrase timing
    // needs only the first, while moment grounding renders all repetitions.
    #[serde(default)]
    all_occurrences: bool,
}

#[derive(Deserialize, Default)]
struct LexiconProfileRequest {
    #[serde(default)]
    language: String,
}

#[derive(Deserialize, Default)]
struct LocalizeIdentitiesRequest {
    #[serde(default)]
    language: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    sources: Vec<semantic_core::localized_identity::SourceEntity>,
    #[serde(default)]
    visual: Vec<semantic_core::localized_identity::VisualPerson>,
    #[serde(default)]
    existing: Vec<semantic_core::localized_identity::IdentityMatch>,
}

#[derive(Deserialize, Default)]
struct SelectPhrasesRequest {
    text: String,
    #[serde(default)]
    blocked_rune_spans: Vec<[usize; 2]>,
    #[serde(default)]
    limit: usize,
    #[serde(default)]
    min_words: usize,
    #[serde(default)]
    max_words: usize,
    #[serde(default)]
    corpus: Vec<String>,
    #[serde(default)]
    profile: Option<SelectPhrasesProfileRequest>,
}

/// A persistent handler holding the shared manifest cache.
pub struct Worker {
    cache: Arc<Mutex<ManifestCache>>,
}

impl Default for Worker {
    fn default() -> Self {
        Self::new(CACHE_ENTRIES)
    }
}

impl Worker {
    pub fn new(cache_entries: usize) -> Self {
        Self {
            cache: Arc::new(Mutex::new(ManifestCache::new(cache_entries))),
        }
    }

    /// Handles one request line and returns exactly one response line.
    pub fn handle_line(&self, line: &str) -> String {
        let envelope: Envelope = match serde_json::from_str(line) {
            Ok(envelope) => envelope,
            Err(err) => return error_response(&format!("invalid request: {err}")),
        };
        match envelope.operation.as_str() {
            "health" => json!({
                "ok": true,
                "schema_version": SCHEMA_VERSION,
                "entity_index_version": semantic_core::entity_linker::entity_index_version(),
                "entity_index_entries": semantic_core::entity_linker::entity_index().entry_count(),
            })
            .to_string(),
            "analyze_script" => self.analyze(line),
            "split_sentences" => split_sentences(line),
            "select_phrases" => select_phrases_line(line),
            "align_phrases" => align_phrases_line(line),
            "compile_editorial_plan" => compile_editorial_plan_line(line),
            "resolve_map_overlaps" => resolve_map_overlaps_line(line),
            "trim_map_windows_before_budget" => trim_map_windows_before_budget_line(line),
            "rank_entity_overlays" => rank_entity_overlays_line(line),
            "cascade_clustered_entrances" => cascade_clustered_entrances_line(line),
            other => error_response(&format!("unknown operation: {other}")),
        }
    }

    fn analyze(&self, line: &str) -> String {
        let request: ScriptRequest = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(err) => return error_response(&format!("invalid analyze_script request: {err}")),
        };
        let key = ManifestCache::request_key(&request, &analyzer_versions());
        if let Some(manifest) = self.cache.lock().ok().and_then(|mut cache| cache.get(&key)) {
            let completeness = manifest_completeness(&manifest);
            return json!({
                "ok": true,
                "manifest": manifest,
                "cached": true,
                "completeness": completeness
            })
            .to_string();
        }
        match analyze_script(&request) {
            Ok(manifest) => {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.insert(key, manifest.clone());
                }
                let completeness = manifest_completeness(&manifest);
                json!({
                    "ok": true,
                    "manifest": manifest,
                    "cached": false,
                    "completeness": completeness
                })
                .to_string()
            }
            Err(message) => error_response(&message),
        }
    }

    /// Serves requests from stdin until end of input. Independent requests run
    /// on a bounded pool; the shared cache stays behind its mutex and the
    /// response lines keep input order. `SEMANTIC_ENGINE_WORKERS=1` restores
    /// serial processing.
    pub fn run_stdio(&self) -> io::Result<()> {
        ndjson_stdio::run_stdio_parallel(
            |line| self.handle_line(line),
            ndjson_stdio::workers_from_env("SEMANTIC_ENGINE_WORKERS"),
        )
    }
}

fn select_phrases_line(line: &str) -> String {
    let request: SelectPhrasesRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return error_response(&format!("invalid select_phrases request: {err}")),
    };
    if request.text.trim().is_empty() {
        return error_response("select_phrases: text is required");
    }
    let profile = request.profile.map(|raw| {
        let policy = PhraseExtractionPolicy {
            min_words: raw.min_words.unwrap_or(2),
            max_words: raw.max_words.unwrap_or(16),
            max_results: raw.max_results.unwrap_or(5),
            reject_verbs_when_all: raw.reject_verbs_when_all.unwrap_or(true),
        };
        PhraseProfile {
            stop_words: raw.stop_words.into_iter().collect(),
            function_words: raw.function_words.into_iter().collect(),
            visual_verbs: raw.visual_verbs.into_iter().collect(),
            policy,
        }
    });
    let blocked: Vec<(usize, usize)> = request
        .blocked_rune_spans
        .into_iter()
        .filter_map(|span| (span[0] < span[1]).then_some((span[0], span[1])))
        .collect();
    let phrases: Vec<SelectedPhrase> = select_phrases(
        &request.text,
        &blocked,
        request.limit,
        profile.as_ref(),
        &request.corpus,
        request.min_words,
        request.max_words,
    );
    json!({"ok": true, "phrases": phrases}).to_string()
}

fn align_phrases_line(line: &str) -> String {
    let request: AlignPhrasesRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return error_response(&format!("invalid align_phrases request: {err}")),
    };
    if request.phrases.is_empty() {
        return error_response("align_phrases: phrases are required");
    }
    let mut alignments: Vec<LocatedPhrase> = Vec::new();
    for (index, phrase) in request.phrases.iter().enumerate() {
        match locate_phrase(&request.words, phrase) {
            Ok(found) => {
                if request.all_occurrences {
                    alignments.extend(found);
                } else {
                    alignments.push(found[0].clone());
                }
            }
            Err(_) if request.allow_missing => {}
            Err(err) => {
                return error_response(&format!("align_phrases: phrase {index} {err}"));
            }
        }
    }
    json!({"ok": true, "alignments": alignments}).to_string()
}

fn compile_editorial_plan_line(line: &str) -> String {
    match declared_schema_version(line) {
        Some(version) if version == EDITORIAL_PLAN_V2_SCHEMA_VERSION => {
            compile_full_editorial_plan_line(line)
        }
        Some(version) => error_response(&format!(
            "unsupported compile_editorial_plan schema_version {version:?}, want {EDITORIAL_PLAN_V2_SCHEMA_VERSION:?}"
        )),
        None => compile_legacy_editorial_plan_line(line),
    }
}

/// Reads the declared envelope version before deserializing. A v2 request must
/// never be interpreted as the sparse legacy batch: the legacy request defaults
/// every section, so that mismatch would silently return an empty plan instead
/// of failing closed.
fn declared_schema_version(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value
        .get("schema_version")
        .and_then(|version| version.as_str())
        .map(ToOwned::to_owned)
}

fn compile_full_editorial_plan_line(line: &str) -> String {
    let request: EditorialPlanV2Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return error_response(&format!("invalid editorial_plan.v2 request: {err}")),
    };
    match compile_overlay_plan_v2(&request) {
        Ok(plan) => match serde_json::to_string(&plan) {
            Ok(response) => response,
            Err(err) => error_response(&format!("encode editorial_plan.v2 response: {err}")),
        },
        Err(err) => error_response(&err.to_string()),
    }
}

fn compile_legacy_editorial_plan_line(line: &str) -> String {
    let request: CompileEditorialPlanRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(&format!("invalid compile_editorial_plan request: {err}"))
        }
    };
    let plan = compile_editorial_plan(request);
    json!({ "ok": true, "plan": plan }).to_string()
}

fn resolve_map_overlaps_line(line: &str) -> String {
    let request: ResolveMapOverlapsRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return error_response(&format!("invalid resolve_map_overlaps request: {err}")),
    };
    let edits = resolve_map_overlaps(request);
    json!({ "ok": true, "edits": edits }).to_string()
}

fn trim_map_windows_before_budget_line(line: &str) -> String {
    let request: ResolveMapOverlapsRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(&format!("invalid map-window budget request: {err}"));
        }
    };
    let edits = trim_map_windows_before_budget(request);
    json!({ "ok": true, "edits": edits }).to_string()
}

fn rank_entity_overlays_line(line: &str) -> String {
    let request: RankEntityOverlaysRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return error_response(&format!("invalid entity overlay rank request: {err}")),
    };
    let selected_indices = rank_entity_overlays(request);
    json!({ "ok": true, "selected_indices": selected_indices }).to_string()
}

fn cascade_clustered_entrances_line(line: &str) -> String {
    let request: CascadeEntrancesRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return error_response(&format!("invalid entrance cascade request: {err}")),
    };
    let edits = cascade_clustered_entrances(request);
    json!({ "ok": true, "edits": edits }).to_string()
}

fn split_sentences(line: &str) -> String {
    let request: SplitRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => return error_response(&format!("invalid split_sentences request: {err}")),
    };
    let language = request.language.trim();
    if language.is_empty() {
        return error_response("split_sentences language is required");
    }
    let sentences: Vec<_> =
        semantic_core::phrase_impact::split_sentences(&request.transcript, language)
            .into_iter()
            .map(|sentence| {
                json!({
                    "text": sentence.text,
                    "start_byte": sentence.start_byte,
                    "end_byte": sentence.end_byte
                })
            })
            .collect();
    json!({ "ok": true, "sentences": sentences }).to_string()
}

fn error_response(message: &str) -> String {
    json!({ "ok": false, "error": message }).to_string()
}

/// Aggregates scene-level rejection into a response-level contract. `ok` only
/// says the request was processed; this field says whether every scene was
/// analysed or whether the caller received a partial manifest.
fn manifest_completeness(manifest: &SemanticManifest) -> serde_json::Value {
    let rejected: Vec<_> = manifest
        .scenes
        .iter()
        .filter(|scene| scene.status == "rejected")
        .collect();
    let status = if rejected.is_empty() {
        "complete"
    } else if rejected.len() == manifest.scenes.len() {
        "rejected"
    } else {
        "partial"
    };
    json!({
        "status": status,
        "rejected_scenes": rejected.len(),
        "rejections": rejected
            .iter()
            .map(|scene| json!({
                "scene_id": scene.scene_id,
                "reason": scene.reason.as_deref().unwrap_or("")
            }))
            .collect::<Vec<_>>(),
    })
}

/// Handles one request line with a fresh worker (stateless convenience for
/// callers that do not keep a worker around; caching still applies within the
/// returned call only through the shared cache held by [`Worker`]).
pub fn handle_line(line: &str) -> String {
    Worker::default().handle_line(line)
}

/// Serves requests from stdin until end of input, with the shared cache warm
/// across the whole process.
pub fn run_stdio() -> io::Result<()> {
    Worker::default().run_stdio()
}

#[cfg(test)]
mod tests {
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
            .insert(key, manifest);

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
    fn different_text_is_not_served_from_cache() {
        let worker = Worker::new(8);
        let a: serde_json::Value =
            serde_json::from_str(&worker.handle_line(&request("Primo testo."))).unwrap();
        let b: serde_json::Value =
            serde_json::from_str(&worker.handle_line(&request("Secondo testo."))).unwrap();
        assert_eq!(a["cached"], false);
        assert_eq!(b["cached"], false, "different text must miss the cache");
    }
}
