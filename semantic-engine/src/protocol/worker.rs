use super::*;

impl Worker {
    /// A memory-only worker. Tests and embedders keep this behaviour.
    pub fn new(cache_entries: usize) -> Self {
        Self::with_disk(cache_entries, None)
    }

    /// A worker with an optional persistent store behind the memory cache.
    pub fn with_disk(cache_entries: usize, disk: Option<Arc<DiskCache>>) -> Self {
        Self {
            cache: Arc::new(Mutex::new(ManifestCache::new(cache_entries))),
            disk,
        }
    }

    /// The shipping worker: memory cache plus a disk cache when the operator
    /// configured `SEMANTIC_CACHE_DIR`.
    pub fn from_env() -> Self {
        Self::with_disk(CACHE_ENTRIES, DiskCache::from_env().map(Arc::new))
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
            "resolve_map_overlaps" => resolve_map_overlaps_line(line),
            "trim_map_windows_before_budget" => trim_map_windows_before_budget_line(line),
            "rank_entity_overlays" => rank_entity_overlays_line(line),
            "cascade_clustered_entrances" => cascade_clustered_entrances_line(line),
            other => error_response(
                ErrorCode::UnsupportedOperation,
                &format!("unknown operation: {other}"),
            ),
        }
    }

    fn analyze(&self, line: &str) -> String {
        let request: ScriptRequest = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(err) => {
                return error_response(
                    ErrorCode::Decode,
                    &format!("invalid analyze_script request: {err}"),
                )
            }
        };
        let analyzers = analyzer_versions();
        let key = ManifestCache::request_key(&request, &analyzers);
        if let Some(manifest) = self.cache.lock().ok().and_then(|mut cache| cache.get(&key)) {
            return manifest_response(manifest.as_ref(), true);
        }

        // The disk key is a wider hash of the same bytes; the fingerprint below
        // is the value recorded on the manifest, so a hit still proves that the
        // stored result belongs to this exact request.
        let fingerprint = request_fingerprint(&request, &analyzers);
        let disk_key = self
            .disk
            .as_ref()
            .map(|_| DiskCache::key(&request, &analyzers));
        if let (Some(disk), Some(disk_key)) = (self.disk.as_ref(), disk_key.as_deref()) {
            if let Some(manifest) = disk.load(disk_key, &fingerprint) {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.insert(key, Arc::clone(&manifest));
                }
                return manifest_response(manifest.as_ref(), true);
            }
        }

        match analyze_script(&request) {
            Ok(manifest) => {
                let manifest = Arc::new(manifest);
                // One encoding serves both caches: the disk payload wraps
                // these bytes and the memory budget uses their length. The
                // previous flow serialized the manifest twice, then cloned it
                // again inside `store`.
                let encoded = serde_json::to_vec(manifest.as_ref()).ok();
                if let (Some(disk), Some(disk_key), Some(encoded)) =
                    (self.disk.as_ref(), disk_key.as_deref(), encoded.as_deref())
                {
                    disk.store_bytes(disk_key, encoded);
                }
                if let (Ok(mut cache), Some(encoded)) = (self.cache.lock(), encoded.as_ref()) {
                    cache.insert_with_bytes(key, Arc::clone(&manifest), encoded.len());
                }
                manifest_response(manifest.as_ref(), false)
            }
            Err(message) => error_response(ErrorCode::Execution, &message),
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
