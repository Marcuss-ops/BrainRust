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

use semantic_core::cue_timing::{self, ShortFormPolicy, TimedCue};
use semantic_core::phrase_align::{locate_phrase, LocatedPhrase, SpeechWord};
use semantic_core::phrase_select::{
    select_phrases, PhraseExtractionPolicy, PhraseProfile, SelectedPhrase,
};
use semantic_core::SemanticManifest;
use serde::Deserialize;
use serde_json::json;

use crate::cache::disk::DiskCache;
use crate::cache::ManifestCache;
use crate::{
    analyze_script, analyzer_versions, request_fingerprint, ScriptRequest, SCHEMA_VERSION,
};
use ndjson_stdio::ErrorCode;
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
struct SubtitlesKernelRequest {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    cues: Vec<TimedCue>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    style_id: String,
    #[serde(default)]
    clip_duration_ms: i64,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    policy: Option<ShortFormPolicy>,
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
    /// Optional persistent store. `None` keeps the worker memory-only, which is
    /// the default until `SEMANTIC_CACHE_DIR` is set.
    disk: Option<Arc<DiskCache>>,
}

impl Default for Worker {
    fn default() -> Self {
        Self::from_env()
    }
}

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

fn subtitles_kernel_line(line: &str) -> String {
    let request: SubtitlesKernelRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid subtitles_kernel request: {err}"),
            )
        }
    };
    let policy = request.policy.unwrap_or_default();
    match request.mode.trim() {
        "normalize" => {
            let cues = cue_timing::normalize_short_form_cues(&request.cues, policy);
            json!({"ok": true, "cues": cues}).to_string()
        }
        "distribute" => {
            if request.cues.is_empty() {
                return error_response(
                    ErrorCode::Execution,
                    "subtitles_kernel: distribute requires cues",
                );
            }
            let cues = cue_timing::cues_with_text(&request.cues, &request.text);
            json!({"ok": true, "cues": cues}).to_string()
        }
        "compile" => match cue_timing::compile_ass_content(&request.cues, &request.style_id) {
            Ok(content) => json!({
                "ok": true,
                "ass_content": content,
                "cue_count": request.cues.len(),
                "last_cue_end_ms": request.cues.last().map(|c| c.end_ms).unwrap_or(0),
            })
            .to_string(),
            Err(err) => error_response(ErrorCode::Execution, &err),
        },
        "validate" => {
            match cue_timing::validate_ass_content(&request.content, request.clip_duration_ms) {
                Ok(()) => json!({"ok": true}).to_string(),
                Err(err) => error_response(ErrorCode::Execution, &err),
            }
        }
        "parse" => match cue_timing::parse_subtitle_text(&request.filename, &request.content) {
            Ok((plain_text, cues)) => json!({
                "ok": true,
                "plain_text": plain_text,
                "cues": cues,
            })
            .to_string(),
            Err(err) => error_response(ErrorCode::Execution, &err),
        },
        other => error_response(
            ErrorCode::UnsupportedOperation,
            &format!("subtitles_kernel: unsupported mode {other:?}"),
        ),
    }
}

fn lexicon_profile_line(line: &str) -> String {
    let request: LexiconProfileRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid lexicon_profile request: {err}"),
            )
        }
    };
    let language = request.language.trim();
    if language.is_empty() {
        let profiles: Vec<&semantic_core::language_profiles::LanguageProfile> =
            semantic_core::language_profiles::supported_languages()
                .into_iter()
                .filter_map(semantic_core::language_profiles::get)
                .collect();
        return json!({
            "ok": true,
            "schema_version": semantic_core::language_profiles::schema_version(),
            "profiles": profiles,
        })
        .to_string();
    }
    match semantic_core::language_profiles::get(language) {
        Some(profile) => json!({
            "ok": true,
            "schema_version": semantic_core::language_profiles::schema_version(),
            "profile": profile,
        })
        .to_string(),
        None => error_response(
            ErrorCode::Execution,
            &format!("lexicon_profile: unknown language {language:?}"),
        ),
    }
}

fn localize_identities_line(line: &str) -> String {
    let request: LocalizeIdentitiesRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid localize_identities request: {err}"),
            )
        }
    };
    let language = request.language.trim();
    if language.is_empty() {
        return error_response(
            ErrorCode::Execution,
            "localize_identities: language is required",
        );
    }
    let mode = if request.mode.trim().is_empty() {
        "match"
    } else {
        request.mode.trim()
    };
    let matches = match mode {
        "match" => semantic_core::localized_identity::match_localized_sources(
            &request.text,
            language,
            &request.sources,
        ),
        "complete" => semantic_core::localized_identity::complete_ordered_person_matches(
            &request.text,
            language,
            &request.sources,
            &request.visual,
            &request.existing,
        ),
        other => {
            return error_response(
                ErrorCode::UnsupportedOperation,
                &format!("localize_identities: unsupported mode {other:?}"),
            )
        }
    };
    json!({"ok": true, "matches": matches}).to_string()
}

fn select_phrases_line(line: &str) -> String {
    let request: SelectPhrasesRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid select_phrases request: {err}"),
            )
        }
    };
    if request.text.trim().is_empty() {
        return error_response(ErrorCode::Execution, "select_phrases: text is required");
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
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid align_phrases request: {err}"),
            )
        }
    };
    if request.phrases.is_empty() {
        return error_response(ErrorCode::Execution, "align_phrases: phrases are required");
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
                return error_response(
                    ErrorCode::Execution,
                    &format!("align_phrases: phrase {index} {err}"),
                );
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
        Some(version) => error_response(ErrorCode::UnsupportedVersion, &format!(
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
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid editorial_plan.v2 request: {err}"),
            )
        }
    };
    match compile_overlay_plan_v2(&request) {
        Ok(plan) => match serde_json::to_string(&plan) {
            Ok(response) => response,
            Err(err) => error_response(
                ErrorCode::Encode,
                &format!("encode editorial_plan.v2 response: {err}"),
            ),
        },
        Err(err) => error_response(ErrorCode::Execution, &err.to_string()),
    }
}

fn compile_legacy_editorial_plan_line(line: &str) -> String {
    let request: CompileEditorialPlanRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid compile_editorial_plan request: {err}"),
            )
        }
    };
    let plan = compile_editorial_plan(request);
    json!({ "ok": true, "plan": plan }).to_string()
}

fn resolve_map_overlaps_line(line: &str) -> String {
    let request: ResolveMapOverlapsRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid resolve_map_overlaps request: {err}"),
            )
        }
    };
    let edits = resolve_map_overlaps(request);
    json!({ "ok": true, "edits": edits }).to_string()
}

fn trim_map_windows_before_budget_line(line: &str) -> String {
    let request: ResolveMapOverlapsRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid map-window budget request: {err}"),
            );
        }
    };
    let edits = trim_map_windows_before_budget(request);
    json!({ "ok": true, "edits": edits }).to_string()
}

fn rank_entity_overlays_line(line: &str) -> String {
    let request: RankEntityOverlaysRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid entity overlay rank request: {err}"),
            )
        }
    };
    let selected_indices = rank_entity_overlays(request);
    json!({ "ok": true, "selected_indices": selected_indices }).to_string()
}

fn cascade_clustered_entrances_line(line: &str) -> String {
    let request: CascadeEntrancesRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid entrance cascade request: {err}"),
            )
        }
    };
    let edits = cascade_clustered_entrances(request);
    json!({ "ok": true, "edits": edits }).to_string()
}

fn split_sentences(line: &str) -> String {
    let request: SplitRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                ErrorCode::Decode,
                &format!("invalid split_sentences request: {err}"),
            )
        }
    };
    let language = request.language.trim();
    if language.is_empty() {
        return error_response(ErrorCode::Execution, "split_sentences language is required");
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

/// One rejection envelope for every operation. `ok` and `error` stay exactly
/// as before (existing clients read the message); `error_code` is the additive
/// machine-readable category `ndjson-stdio` already defines for the other
/// workers, so a caller no longer has to parse English to distinguish a
/// malformed line from a failed operation.
fn error_response(code: ErrorCode, message: &str) -> String {
    json!({ "ok": false, "error_code": code, "error": message }).to_string()
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

/// One `analyze_script` response, shared by the memory hit, the disk hit and
/// the fresh-analysis paths so all three report the same completeness contract.
fn manifest_response(manifest: &SemanticManifest, cached: bool) -> String {
    let completeness = manifest_completeness(manifest);
    json!({
        "ok": true,
        "manifest": manifest,
        "cached": cached,
        "completeness": completeness
    })
    .to_string()
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
