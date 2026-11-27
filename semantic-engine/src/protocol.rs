//! Newline-delimited JSON protocol over stdin/stdout, one response per request.
//!
//! Requests: `{"operation":"health"}` and
//! `{"operation":"analyze_script","language":"it","scenes":[{"scene_id":"s1","text":"..."}]}`.
//! Responses: `{"ok":true,...}` or `{"ok":false,"error":"..."}`.
//!
//! The worker is persistent, so it keeps one [`ManifestCache`] across requests:
//! re-analysing the same script text (a replay, or a repeated job) is served
//! from the cache instead of re-running the extractor.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Condvar, Mutex};

use semantic_core::cue_timing::{self, ShortFormPolicy, TimedCue};
use semantic_core::phrase_align::{locate_phrase, LocatedPhrase, SpeechWord};
use semantic_core::phrase_select::{
    select_phrases, PhraseExtractionPolicy, PhraseProfile, SelectedPhrase,
};
use semantic_core::SemanticManifest;
use serde::Deserialize;
use serde_json::json;

use crate::cache::disk::DiskCache;
use crate::cache::scene::SceneCache;
use crate::cache::ManifestCache;
use crate::{analyzer_versions, request_fingerprint, ScriptRequest, SCHEMA_VERSION};
use ndjson_stdio::ErrorCode;
use overlay_compiler::{
    compile_editorial_plan, compile_overlay_plan_v2_owned, CompileEditorialPlanRequest,
    EditorialPlanV2Request, EDITORIAL_PLAN_V2_SCHEMA_VERSION,
};

/// Entries kept per persistent worker. Large enough that a full script batch
/// plus its replay stay warm, small enough to bound RSS.
const CACHE_ENTRIES: usize = 256;

#[derive(Deserialize)]
struct Envelope {
    operation: String,
}

/// `analyze_script` request envelope. The wire version is optional and defaults
/// to v3, so every existing caller keeps the current contract. The field lives
/// on the envelope, not on [`ScriptRequest`], because it selects the response
/// encoding and is not an analysis input.
#[derive(Deserialize)]
struct AnalyzeScriptRequest {
    #[serde(flatten)]
    request: ScriptRequest,
    #[serde(default)]
    manifest_version: Option<String>,
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
    /// Per-scene cache owned by the worker, initialized once from the
    /// environment. Reopening it per request would run `create_dir_all` and the
    /// retention scan on every analysis.
    scene_cache: Option<Arc<SceneCache>>,
    /// Identical requests that are already being analysed. A waiter blocks on
    /// the flight instead of running the same extractor a second time.
    in_flight: Mutex<HashMap<String, Arc<Flight>>>,
}

/// Manifest plus its canonical encoding, shared by the leader and every
/// waiter so a cache hit and a cold response use the same bytes.
type FlightResult = Result<(Arc<SemanticManifest>, Arc<[u8]>), String>;

/// One analysis in progress, keyed by the request fingerprint.
struct Flight {
    result: Mutex<Option<FlightResult>>,
    done: Condvar,
}

impl Flight {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            done: Condvar::new(),
        }
    }

    fn complete(&self, result: FlightResult) {
        let mut slot = self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            *slot = Some(result);
            self.done.notify_all();
        }
    }

    fn wait(&self) -> FlightResult {
        let mut slot = self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while slot.is_none() {
            slot = self
                .done
                .wait(slot)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        slot.as_ref().expect("flight completed").clone()
    }
}

/// Completes a leader's flight with an error if the analysis unwinds, so a
/// waiter can never block forever on a panicking analysis.
struct FlightGuard<'a> {
    worker: &'a Worker,
    key: String,
    flight: Arc<Flight>,
    completed: bool,
}

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        self.flight.complete(Err("analysis aborted".to_string()));
        if let Ok(mut map) = self.worker.in_flight.lock() {
            map.remove(&self.key);
        }
    }
}

impl Default for Worker {
    fn default() -> Self {
        Self::from_env()
    }
}

mod worker;
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
    match compile_overlay_plan_v2_owned(request) {
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
/// Builds the response from the manifest's canonical bytes, avoiding a second
/// `serde_json::Value` tree for the whole manifest. Every path (cold, memory
/// hit, disk hit) uses this builder, so their bodies are byte-identical apart
/// from the `cached` flag; the protocol test enforces that.
fn manifest_response_bytes(encoded: &[u8], manifest: &SemanticManifest, cached: bool) -> String {
    let completeness = manifest_completeness(manifest);
    let manifest = std::str::from_utf8(encoded).unwrap_or("null");
    let mut response = String::with_capacity(encoded.len() + 128);
    response.push_str("{\"cached\":");
    response.push_str(if cached { "true" } else { "false" });
    response.push_str(",\"completeness\":");
    response.push_str(&completeness.to_string());
    response.push_str(",\"manifest\":");
    response.push_str(manifest);
    response.push_str(",\"ok\":true}");
    response
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
