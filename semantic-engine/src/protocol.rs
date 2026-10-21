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
            other => error_response(&format!("unknown operation: {other}")),
        }
    }

    fn analyze(&self, line: &str) -> String {
        let request: ScriptRequest = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(err) => return error_response(&format!("invalid analyze_script request: {err}")),
        };
        let key = ManifestCache::key(
            request.language.trim(),
            &request.scenes,
            &analyzer_versions(),
        );
        if let Some(manifest) = self.cache.lock().ok().and_then(|mut cache| cache.get(&key)) {
            return json!({ "ok": true, "manifest": manifest, "cached": true }).to_string();
        }
        match analyze_script(&request) {
            Ok(manifest) => {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.insert(key, manifest.clone());
                }
                json!({ "ok": true, "manifest": manifest, "cached": false }).to_string()
            }
            Err(message) => error_response(&message),
        }
    }

    /// Serves requests from stdin until end of input.
    pub fn run_stdio(&self) -> io::Result<()> {
        ndjson_stdio::run_stdio(|line| self.handle_line(line))
    }
}

fn error_response(message: &str) -> String {
    json!({ "ok": false, "error": message }).to_string()
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
