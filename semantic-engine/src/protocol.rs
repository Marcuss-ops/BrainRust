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

use semantic_core::SemanticManifest;
use serde::Deserialize;
use serde_json::json;

use crate::cache::ManifestCache;
use crate::{analyze_script, analyzer_versions, ScriptRequest, SCHEMA_VERSION};

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
            "health" => json!({ "ok": true, "schema_version": SCHEMA_VERSION }).to_string(),
            "analyze_script" => self.analyze(line),
            "split_sentences" => split_sentences(line),
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
