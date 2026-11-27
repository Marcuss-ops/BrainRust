//! Optional local GLiNER2.5 named-entity recognition.
//!
//! UDPipe remains responsible for lemmas and syntax. GLiNER contributes
//! source-grounded entity spans; if its model files are absent or fail to
//! load, the deterministic extractor continues to run.

use super::values::Candidate;
use gliner25_rs::{BoundaryConfig, BoundaryEngine, BoundaryParams, ExecutionMode, SchemaTask};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;

const ENTITY_LABELS: [&str; 11] = [
    "person",
    "organization",
    "location",
    "book",
    "television program",
    "play",
    "award",
    "historical event",
    "facility",
    "product",
    "brand",
];

static NER_WORKER: OnceLock<Result<Sender<ModelRequest>, String>> = OnceLock::new();

struct ModelRequest {
    source: String,
    language: String,
    response: Sender<Result<Vec<Candidate>, String>>,
}

/// Returns NER candidates only when a local model is installed or explicitly
/// requested. This keeps CI and model-free deployments deterministic and
/// avoids downloading weights as a side effect of ordinary extraction.
pub(super) fn candidates(source: &str, language: &str) -> Result<(bool, Vec<Candidate>), String> {
    if model_disabled() {
        return Ok((false, Vec::new()));
    }
    let model_dir = model_dir();
    let explicit = std::env::var("BRAINRUST_NER_MODEL")
        .is_ok_and(|value| value.eq_ignore_ascii_case("gliner25"));
    if !explicit && !has_local_model(&model_dir) {
        return Ok((false, Vec::new()));
    }
    if !has_local_model(&model_dir) {
        return Err(format!(
            "GLiNER2.5 is enabled but no model manifest exists under {}; set GLINER25_MODEL_DIR or install the model",
            model_dir.display()
        ));
    }
    let worker = worker(&model_dir)?;
    let (response, result) = mpsc::channel();
    worker
        .send(ModelRequest {
            source: source.to_string(),
            language: language.to_string(),
            response,
        })
        .map_err(|_| "GLiNER2.5 worker stopped unexpectedly".to_string())?;
    Ok((
        true,
        result
            .recv()
            .map_err(|_| "GLiNER2.5 worker dropped its response".to_string())??,
    ))
}

/// Non-invasive NER state for the worker health response. Reading it never
/// spawns the model thread and never loads weights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NerStatus {
    /// `disabled`, `enabled` (explicitly requested) or `auto`.
    pub mode: &'static str,
    /// A GLiNER manifest exists under the resolved model directory.
    pub model_installed: bool,
    /// Resolved model directory (for deployment diagnostics).
    pub model_dir: String,
    /// The model thread has been started successfully in this process.
    pub worker_started: bool,
}

pub fn status() -> NerStatus {
    let model_dir = model_dir();
    NerStatus {
        mode: requested_mode(),
        model_installed: has_local_model(&model_dir),
        model_dir: model_dir.to_string_lossy().into_owned(),
        worker_started: matches!(NER_WORKER.get(), Some(Ok(_))),
    }
}

fn requested_mode() -> &'static str {
    match std::env::var("BRAINRUST_NER_MODEL") {
        Ok(value) if is_disabled_value(&value) => "disabled",
        Ok(value) if value.eq_ignore_ascii_case("gliner25") => "enabled",
        _ => "auto",
    }
}

fn is_disabled_value(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "off" | "none" | "disabled"
    )
}

fn worker(model_dir: &std::path::Path) -> Result<&'static Sender<ModelRequest>, String> {
    let model_dir = model_dir.to_path_buf();
    // The whole initialization (channel + thread spawn) runs inside
    // `get_or_init`, so concurrent callers cannot spawn two model loaders.
    get_or_init_result(&NER_WORKER, || spawn_worker(model_dir))
}

/// Starts one worker thread and returns its request sender. Extracted from
/// `worker()` so the error path can be tested without touching the global.
fn spawn_worker(model_dir: PathBuf) -> Result<Sender<ModelRequest>, String> {
    let (requests, incoming) = mpsc::channel::<ModelRequest>();
    std::thread::Builder::new()
        .name("visualner-gliner25".to_string())
        .spawn(move || {
            let mut model = load_model(&model_dir);
            while let Ok(request) = incoming.recv() {
                let candidates = model
                    .as_mut()
                    .map_err(|error| error.clone())
                    .and_then(|model| {
                        extract_candidates(model, &request.source, &request.language)
                    });
                let _ = request.response.send(candidates);
            }
        })
        .map(|_| requests)
        .map_err(|error| format!("could not start GLiNER2.5 worker: {error}"))
}

/// Runs `init` once and returns either the initialized value or the cached
/// initialization error to every caller. Extracted so the single-execution
/// guarantee is testable without touching the process-global worker.
fn get_or_init_result<T>(
    cell: &OnceLock<Result<T, String>>,
    init: impl FnOnce() -> Result<T, String>,
) -> Result<&T, String> {
    match cell.get_or_init(init) {
        Ok(value) => Ok(value),
        Err(error) => Err(error.clone()),
    }
}

fn extract_candidates(
    model: &mut BoundaryEngine,
    source: &str,
    language: &str,
) -> Result<Vec<Candidate>, String> {
    let labels = ENTITY_LABELS
        .iter()
        .map(|label| (*label).to_string())
        .collect();
    let tasks = [SchemaTask::Entities(labels)];
    let mut params = BoundaryParams::default();
    let (global_threshold, location_threshold, organization_threshold) = effective_thresholds();
    // The engine drops mentions below `params.threshold`, so it must see the
    // most permissive class threshold; the per-class filter below applies the
    // stricter value afterwards.
    params.threshold = global_threshold.min(organization_threshold);

    let output = model
        .extract_long_with(source, &tasks, &params, Default::default())
        .map_err(|error| format!("GLiNER2.5 inference failed: {error}"))?;

    Ok(output
        .mentions
        .into_iter()
        .filter_map(|mention| {
            let kind = entity_kind(&mention.field)?;
            let threshold = match kind {
                "LOCATION" => location_threshold,
                "ORG" => organization_threshold,
                _ => global_threshold,
            };
            if mention.score < threshold {
                return None;
            }
            let text = source.get(mention.char_start..mention.char_end)?;
            if !has_name_shape(text, language) {
                return None;
            }
            Some(Candidate {
                text: text.to_string(),
                start: mention.char_start,
                end: mention.char_end,
                entity_type: Some(kind.to_string()),
            })
        })
        .collect())
}

/// The thresholds the extractor actually applies. Organizations carry the
/// weakest zero-shot signal: synthetic company names such as "Büro Steinbach"
/// land around 0.5 while person and location mentions are usually well above
/// 0.8. A single flat threshold either loses those organizations or lets
/// common nouns through.
fn effective_thresholds() -> (f32, f32, f32) {
    let global = threshold_from_env("BRAINRUST_NER_THRESHOLD", 0.80);
    let location = threshold_from_env("BRAINRUST_NER_LOCATION_THRESHOLD", 0.90);
    let organization = threshold_from_env("BRAINRUST_NER_ORG_THRESHOLD", global.min(0.50));
    (global, location, organization)
}

/// Identity of the GLiNER runtime for the persistent cache key. It covers the
/// mode, whether a model is installed, the content digest of the selected
/// model variant and the effective thresholds. Cached once per process: the
/// model and the environment are fixed for the lifetime of the worker.
pub fn ner_cache_identity() -> String {
    static IDENTITY: OnceLock<String> = OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            let mode = requested_mode();
            let dir = model_dir();
            let installed = has_local_model(&dir);
            // Hash every installed variant, not just the first one: the
            // loader may select fp32/fp16 by hardware, and the identity must
            // follow the content that is actually available.
            let mut variants: Vec<String> = ["fp32_25", "fp16_25", "fp16"]
                .iter()
                .map(|variant| dir.join(variant))
                .filter(|path| path.join("boundary_manifest.json").is_file())
                .map(|path| {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    format!("{name}:{}", directory_fingerprint(&path))
                })
                .collect();
            variants.sort();
            let model = if variants.is_empty() {
                "absent".to_string()
            } else {
                variants.join(",")
            };
            let (global, location, organization) = effective_thresholds();
            format!(
                "mode={mode};installed={installed};model={model};thresholds={global:.6}/{location:.6}/{organization:.6}"
            )
        })
        .clone()
}

/// SHA-256 over the relative names and bytes of every file under `root`. The
/// path itself is not hashed, so two deployments with the same model content
/// share the identity. A read error is hashed as such instead of being ignored.
fn directory_fingerprint(root: &Path) -> String {
    fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                collect(root, &path, files);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative, path));
            }
        }
    }

    let mut files = Vec::new();
    collect(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    hasher.update(b"visualner-gliner-model");
    for (name, path) in files {
        hasher.update(name.as_bytes());
        hasher.update([0u8]);
        match std::fs::read(&path) {
            Ok(bytes) => {
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            Err(error) => {
                hasher.update(b"error");
                hasher.update(error.to_string().as_bytes());
            }
        }
    }
    semantic_core::text::lowercase_hex(&hasher.finalize())
}

/// Resolves the bundled ONNX Runtime for GLiNER and exports it for the
/// process. Call this once **before** starting worker threads.
///
/// `std::env::set_var` mutates process-global state; doing it from the model
/// thread could race with any other environment access. The binary calls this
/// from `main`, and library embedders that enable GLiNER must call it during
/// their single-threaded startup. When the variable is already set, or no
/// bundled library exists, this is a no-op.
pub fn configure_runtime() {
    static CONFIGURED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CONFIGURED.get_or_init(|| {
        if std::env::var_os("ORT_DYLIB_PATH").is_some() {
            return;
        }
        let model_dir = model_dir();
        for name in ["libonnxruntime.so", "libonnxruntime.so.1.23.2"] {
            let candidate = model_dir.join(name);
            if candidate.is_file() {
                std::env::set_var("ORT_DYLIB_PATH", candidate);
                break;
            }
        }
    });
}

fn load_model(model_dir: &std::path::Path) -> Result<BoundaryEngine, String> {
    gliner25_rs::init("visualner-gliner25");
    let config = BoundaryConfig::new(model_dir).with_execution(ExecutionMode::Standard);
    BoundaryEngine::new(config).map_err(|error| {
        let message = format!("GLiNER2.5 initialization failed: {error}");
        eprintln!("visualner: {message}");
        message
    })
}

fn threshold_from_env(name: &str, fallback: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| (0.0..=1.0).contains(value))
        .unwrap_or(fallback)
}

fn model_disabled() -> bool {
    std::env::var("BRAINRUST_NER_MODEL").is_ok_and(|value| is_disabled_value(&value))
}

fn model_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("GLINER25_MODEL_DIR") {
        return PathBuf::from(path);
    }
    if let Some(root) = std::env::var_os("VISUALNER_MODEL_DIR") {
        return PathBuf::from(root).join("gliner2.5-multi-v1-onnx");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/visualner/models/gliner2.5-multi-v1-onnx")
}

fn has_local_model(path: &std::path::Path) -> bool {
    ["fp32_25", "fp16_25", "fp16"]
        .iter()
        .any(|variant| path.join(variant).join("boundary_manifest.json").is_file())
}

fn entity_kind(label: &str) -> Option<&'static str> {
    match label {
        "person" => Some("PERSON"),
        "organization" => Some("ORG"),
        "location" => Some("LOCATION"),
        "book" | "television program" | "play" => Some("WORK"),
        "award" => Some("AWARD"),
        "historical event" => Some("EVENT"),
        "facility" => Some("FACILITY"),
        "product" => Some("PRODUCT"),
        "brand" => Some("BRAND"),
        "animal" | "species" => Some("ANIMAL"),
        "food" | "dish" => Some("FOOD"),
        "vehicle" => Some("VEHICLE"),
        "law" => Some("LAW"),
        "court case" => Some("COURT_CASE"),
        "norp" | "political group" => Some("NORP"),
        "disease" => Some("DISEASE"),
        "technology" | "software" => Some("TECHNOLOGY"),
        "artwork" => Some("ARTWORK"),
        "celestial body" => Some("CELESTIAL"),
        _ => None,
    }
}

/// Reject lowercase common nouns returned by a zero-shot prompt while
/// retaining uncased writing systems and mixed-case names such as iPhone.
fn has_name_shape(text: &str, language: &str) -> bool {
    let profiles = &semantic_core::lexicon::lexicon().language_profiles;
    !profiles.is_cased(language) || text.chars().any(char::is_uppercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn concurrent_initialization_runs_the_closure_once() {
        static CELL: OnceLock<Result<usize, String>> = OnceLock::new();
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        get_or_init_result(&CELL, || {
                            CALLS.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(20));
                            Ok(42)
                        })
                        .copied()
                    })
                })
                .collect();
            for handle in handles {
                assert_eq!(handle.join().expect("thread"), Ok(42));
            }
        });
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn worker_reports_a_model_load_failure_instead_of_an_empty_success() {
        let root = std::env::temp_dir().join(format!(
            "brainrust-ner-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("fp32_25")).expect("model dir");
        std::fs::write(root.join("fp32_25/boundary_manifest.json"), b"not-json").expect("manifest");
        let sender = spawn_worker(root.clone()).expect("worker starts");
        let (response, result) = mpsc::channel();
        sender
            .send(ModelRequest {
                source: "Alice met Bob".to_string(),
                language: "en".to_string(),
                response,
            })
            .expect("request");
        match result.recv().expect("response") {
            Err(message) => assert!(
                message.contains("GLiNER2.5"),
                "unexpected error message: {message}"
            ),
            Ok(candidates) => {
                panic!("a model load failure must not become an empty success: {candidates:?}")
            }
        }
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn model_directory_fingerprint_changes_with_content() {
        let root = std::env::temp_dir().join(format!(
            "brainrust-ner-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("fp32_25")).expect("model dir");
        std::fs::write(root.join("fp32_25/boundary_manifest.json"), b"one").expect("manifest");
        let first = directory_fingerprint(&root);
        std::fs::write(root.join("fp32_25/boundary_manifest.json"), b"two").expect("manifest");
        let second = directory_fingerprint(&root);
        assert_ne!(first, second);
        assert_eq!(second, directory_fingerprint(&root));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn additive_gliner_labels_project_to_typed_entity_kinds() {
        for (label, expected) in [
            ("animal", "ANIMAL"),
            ("species", "ANIMAL"),
            ("food", "FOOD"),
            ("vehicle", "VEHICLE"),
            ("law", "LAW"),
            ("court case", "COURT_CASE"),
            ("norp", "NORP"),
            ("disease", "DISEASE"),
            ("technology", "TECHNOLOGY"),
            ("artwork", "ARTWORK"),
            ("celestial body", "CELESTIAL"),
        ] {
            assert_eq!(entity_kind(label), Some(expected), "{label}");
        }
        assert_eq!(entity_kind("untyped"), None);
    }
}
