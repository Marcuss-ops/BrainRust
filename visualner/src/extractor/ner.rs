//! Optional local GLiNER2.5 named-entity recognition.
//!
//! UDPipe remains responsible for lemmas and syntax. GLiNER contributes
//! source-grounded entity spans; if its model files are absent or fail to
//! load, the deterministic extractor continues to run.

use super::values::Candidate;
use gliner25_rs::{BoundaryConfig, BoundaryEngine, BoundaryParams, ExecutionMode, SchemaTask};
use std::path::PathBuf;
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

static NER_WORKER: OnceLock<Sender<ModelRequest>> = OnceLock::new();

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
    let worker = worker(&model_dir);
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

fn worker(model_dir: &std::path::Path) -> &'static Sender<ModelRequest> {
    NER_WORKER.get_or_init(|| {
        let (requests, incoming) = mpsc::channel::<ModelRequest>();
        let model_dir = model_dir.to_path_buf();
        std::thread::Builder::new()
            .name("visualner-gliner25".to_string())
            .spawn(move || {
                let mut model = load_model(&model_dir);
                while let Ok(request) = incoming.recv() {
                    let candidates = model
                        .as_mut()
                        .map_err(|error| error.clone())
                        .map(|model| extract_candidates(model, &request.source, &request.language));
                    let _ = request.response.send(candidates);
                }
            })
            .expect("could not start GLiNER2.5 worker");
        requests
    })
}

fn extract_candidates(model: &mut BoundaryEngine, source: &str, language: &str) -> Vec<Candidate> {
    let labels = ENTITY_LABELS
        .iter()
        .map(|label| (*label).to_string())
        .collect();
    let tasks = [SchemaTask::Entities(labels)];
    let mut params = BoundaryParams::default();
    let global_threshold = threshold_from_env("BRAINRUST_NER_THRESHOLD", 0.80);
    let location_threshold = threshold_from_env("BRAINRUST_NER_LOCATION_THRESHOLD", 0.90);
    // Organizations carry the weakest zero-shot signal: synthetic company names
    // such as "Büro Steinbach" land around 0.5 while person and location
    // mentions are usually well above 0.8. A single flat threshold either
    // loses those organizations or lets common nouns through.
    let organization_threshold =
        threshold_from_env("BRAINRUST_NER_ORG_THRESHOLD", global_threshold.min(0.50));
    // The engine drops mentions below `params.threshold`, so it must see the
    // most permissive class threshold; the per-class filter below applies the
    // stricter value afterwards.
    params.threshold = global_threshold.min(organization_threshold);

    let Ok(output) = model.extract_long_with(source, &tasks, &params, Default::default()) else {
        return Vec::new();
    };

    output
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
        .collect()
}

fn load_model(model_dir: &std::path::Path) -> Result<BoundaryEngine, String> {
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        for name in ["libonnxruntime.so", "libonnxruntime.so.1.23.2"] {
            let candidate = model_dir.join(name);
            if candidate.is_file() {
                std::env::set_var("ORT_DYLIB_PATH", candidate);
                break;
            }
        }
    }
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
    std::env::var("BRAINRUST_NER_MODEL").is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "off" | "none" | "disabled"
        )
    })
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
