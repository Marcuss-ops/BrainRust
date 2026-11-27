use super::*;
use crate::analyze_script_with_scene_cache;
use crate::cache::scene::SceneCache;
use std::collections::HashMap;

impl Worker {
    /// A memory-only worker. Tests and embedders keep this behaviour.
    pub fn new(cache_entries: usize) -> Self {
        Self::with_disk(cache_entries, None)
    }

    /// A worker with an optional persistent store behind the memory cache.
    pub fn with_disk(cache_entries: usize, disk: Option<Arc<DiskCache>>) -> Self {
        Self::with_caches(cache_entries, disk, None)
    }

    fn with_caches(
        cache_entries: usize,
        disk: Option<Arc<DiskCache>>,
        scene_cache: Option<Arc<SceneCache>>,
    ) -> Self {
        Self {
            cache: Arc::new(Mutex::new(ManifestCache::new(cache_entries))),
            disk,
            scene_cache,
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    /// The shipping worker: memory cache plus the persistent caches when the
    /// operator configured `SEMANTIC_CACHE_DIR`. The scene cache is opened
    /// once here, not on every request.
    pub fn from_env() -> Self {
        Self::with_caches(
            CACHE_ENTRIES,
            DiskCache::from_env().map(Arc::new),
            SceneCache::from_env().map(Arc::new),
        )
    }

    /// Handles one request line and returns exactly one response line.
    pub fn handle_line(&self, line: &str) -> String {
        let envelope: Envelope = match serde_json::from_str(line) {
            Ok(envelope) => envelope,
            Err(err) => {
                return error_response(ErrorCode::Decode, &format!("invalid request: {err}"))
            }
        };
        match envelope.operation.as_str() {
            "health" => {
                let lexicon = semantic_core::lexicon::provenance();
                let models = visualner::model_status();
                let ner = visualner::ner_status();
                // The cache lock is only taken for this read: health never holds
                // it across analysis work, so it cannot delay a request.
                let cache = self
                    .cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let (hits, misses) = cache.stats();
                let (max_entries, max_bytes) = cache.capacity();
                json!({
                    "ok": true,
                    "schema_version": SCHEMA_VERSION,
                    "manifest_versions": semantic_core::PRODUCIBLE_SCHEMA_VERSIONS,
                    "entity_index_version": semantic_core::entity_linker::entity_index_version(),
                    "entity_index_entries": semantic_core::entity_linker::entity_index().entry_count(),
                    "entity_linker_version": semantic_core::entity_linker::linker_version(),
                    "entity_linker_weights_sha256": semantic_core::entity_linker::linker_weights_sha256(),
                    "lexicon": {
                        "version": lexicon.version,
                        "source": lexicon.source(),
                        "override_dir": lexicon.override_dir,
                        "tables_from_override": lexicon.overrides_loaded,
                        "tables_failed": lexicon.overrides_failed,
                    },
                    // Kept for older probes; `udpipe` is the diagnostic view.
                    "udpipe_model_slots": models.cached_slots,
                    "udpipe": {
                        "cached_slots": models.cached_slots,
                        "loaded_languages": models.loaded_languages,
                        "failed_languages": models.failed_languages,
                        "installed_languages": models.installed_languages,
                        "missing_languages": models.missing_languages,
                    },
                    "ner": {
                        "mode": ner.mode,
                        "model_installed": ner.model_installed,
                        "model_dir": ner.model_dir,
                        "worker_started": ner.worker_started,
                    },
                    "cache": {
                        "entries": cache.len(),
                        "bytes": cache.bytes(),
                        "hits": hits,
                        "misses": misses,
                        "max_entries": max_entries,
                        "max_bytes": max_bytes,
                        "disk_enabled": self.disk.is_some(),
                        "scene_cache_enabled": self.scene_cache.is_some(),
                    },
                })
                .to_string()
            }
            "analyze_script" => self.analyze(line),
            "lexicon_profile" => lexicon_profile_line(line),
            "quality_gate" => script_qa::protocol::handle_line(line),
            "localize_identities" => localize_identities_line(line),
            "split_sentences" => split_sentences(line),
            "select_phrases" => select_phrases_line(line),
            "align_phrases" => align_phrases_line(line),
            "subtitles_kernel" => subtitles_kernel_line(line),
            "compile_editorial_plan" => compile_editorial_plan_line(line),
            other => error_response(
                ErrorCode::UnsupportedOperation,
                &format!("unknown operation: {other}"),
            ),
        }
    }

    fn analyze(&self, line: &str) -> String {
        let envelope: AnalyzeScriptRequest = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(err) => {
                return error_response(
                    ErrorCode::Decode,
                    &format!("invalid analyze_script request: {err}"),
                )
            }
        };
        // Version routing is explicit: an unknown version is a request error,
        // never a silent fallback to the v3 shape.
        let version =
            match semantic_core::ManifestVersion::parse(envelope.manifest_version.as_deref()) {
                Ok(version) => version,
                Err(err) => return error_response(ErrorCode::Decode, &err),
            };
        let request = envelope.request;
        let analyzers = analyzer_versions();
        // The fingerprint identifies the analysis; the version suffix keeps the
        // in-memory cache from serving v3 bytes to a v4 caller. v3 keeps the
        // historical key (empty suffix), so existing entries stay valid.
        let fingerprint = request_fingerprint(&request, &analyzers);
        let key = format!("{fingerprint}{}", version.cache_suffix());
        if let Some((manifest, encoded)) = self
            .cache
            .lock()
            .ok()
            .and_then(|mut cache| cache.get_encoded(&key))
        {
            return manifest_response_bytes(&encoded, manifest.as_ref(), true);
        }

        // The disk key is a wider hash of the same request bytes and stays
        // version-independent: the store always holds the canonical v3
        // manifest, and a v4 hit is re-projected before it is served.
        let disk_key = self
            .disk
            .as_ref()
            .map(|_| DiskCache::key(&request, &analyzers));
        if let (Some(disk), Some(disk_key)) = (self.disk.as_ref(), disk_key.as_deref()) {
            if let Some(manifest) = disk.load(disk_key, &fingerprint) {
                let encoded = match Self::encode_manifest(manifest.as_ref(), version) {
                    Ok(encoded) => encoded,
                    Err(err) => return error_response(ErrorCode::Encode, &err),
                };
                if let Ok(mut cache) = self.cache.lock() {
                    cache.insert_encoded(key.clone(), Arc::clone(&manifest), Arc::clone(&encoded));
                }
                return manifest_response_bytes(&encoded, manifest.as_ref(), true);
            }
        }

        // The closure runs only for the leader. It stores the result before
        // completing the flight, so a waiter wakes after the caches are
        // populated and a subsequent request is a normal cache hit.
        let reused = std::cell::Cell::new(false);
        let (result, leader) = self.run_single_flight(&key, || {
            // A caller can miss the cache, then reach the flight map only after
            // the previous leader stored its result and removed the entry. Re-check
            // under the leader slot so that window cannot duplicate the analysis.
            if let Ok(cache) = self.cache.lock() {
                if let Some((manifest, encoded)) = cache.peek_encoded(&key) {
                    reused.set(true);
                    return Ok((manifest, encoded));
                }
            }
            let manifest = Arc::new(analyze_script_with_scene_cache(
                &request,
                self.scene_cache.as_deref(),
            )?);
            // The disk store always keeps the canonical v3 encoding; the
            // memory entry and the response body use the requested version.
            let v3_encoded = serde_json::to_vec(manifest.as_ref())
                .map_err(|err| format!("encode manifest: {err}"))?;
            let encoded: Arc<[u8]> = match version {
                semantic_core::ManifestVersion::V3 => Arc::from(v3_encoded.clone()),
                semantic_core::ManifestVersion::V4 => {
                    Self::encode_manifest(manifest.as_ref(), version)?
                }
            };
            if let (Some(disk), Some(disk_key)) = (self.disk.as_ref(), disk_key.as_deref()) {
                disk.store_bytes(disk_key, &v3_encoded);
            }
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert_encoded(key.clone(), Arc::clone(&manifest), Arc::clone(&encoded));
            }
            Ok((manifest, encoded))
        });
        match result {
            Ok((manifest, encoded)) => {
                let cached = !leader || reused.get();
                manifest_response_bytes(&encoded, manifest.as_ref(), cached)
            }
            Err(message) => error_response(ErrorCode::Execution, &message),
        }
    }

    /// Encode one manifest for the requested wire version. v3 is the canonical
    /// serde encoding; v4 is the normalized projection from
    /// `semantic_core::to_v4_value`.
    fn encode_manifest(
        manifest: &SemanticManifest,
        version: semantic_core::ManifestVersion,
    ) -> Result<Arc<[u8]>, String> {
        match version {
            semantic_core::ManifestVersion::V3 => serde_json::to_vec(manifest)
                .map(Arc::from)
                .map_err(|err| format!("encode manifest: {err}")),
            semantic_core::ManifestVersion::V4 => {
                let value = semantic_core::to_v4_value(manifest)?;
                serde_json::to_vec(&value)
                    .map(Arc::from)
                    .map_err(|err| format!("encode v4 manifest: {err}"))
            }
        }
    }

    /// Runs `analyze` once per distinct `key`. Concurrent callers with the same
    /// key wait for the leader and receive the same result. The returned bool
    /// is `true` for the caller that actually ran the analysis.
    fn run_single_flight<F>(&self, key: &str, analyze: F) -> (FlightResult, bool)
    where
        F: FnOnce() -> FlightResult,
    {
        let (flight, leader) = {
            let mut map = self
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match map.get(key) {
                Some(flight) => (Arc::clone(flight), false),
                None => {
                    let flight = Arc::new(Flight::new());
                    map.insert(key.to_string(), Arc::clone(&flight));
                    (flight, true)
                }
            }
        };
        if !leader {
            return (flight.wait(), false);
        }
        let mut guard = FlightGuard {
            worker: self,
            key: key.to_string(),
            flight: Arc::clone(&flight),
            completed: false,
        };
        let result = analyze();
        flight.complete(result.clone());
        if let Ok(mut map) = self.in_flight.lock() {
            map.remove(key);
        }
        guard.completed = true;
        (result, true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn identical_concurrent_requests_share_one_analysis() {
        let worker = Worker::new(8);
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        // Release every thread together: without the barrier a thread can start
        // after the leader already finished and legitimately become a new leader.
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let start = &start;
                    let worker = &worker;
                    scope.spawn(move || {
                        start.wait();
                        worker
                            .run_single_flight("same-key", || {
                                CALLS.fetch_add(1, Ordering::SeqCst);
                                std::thread::sleep(Duration::from_millis(500));
                                Err::<(Arc<SemanticManifest>, Arc<[u8]>), String>(
                                    "forced".to_string(),
                                )
                            })
                            .0
                    })
                })
                .collect();
            for handle in handles {
                match handle.join().expect("thread") {
                    Err(message) => assert_eq!(message, "forced"),
                    Ok(_) => panic!("expected the leader's error"),
                }
            }
        });
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}
