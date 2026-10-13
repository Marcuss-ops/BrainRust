use mediasampler::{sample_scene, BoundAssets, Candidate, SampleOptions, SampleResult, Scene};
use ndjson_stdio::{check_envelope, ErrorCode, WorkerError};
use serde::{Deserialize, Serialize};

/// One ledger entry binding an asset to the scene that won it. The worker
/// itself stays stateless across stdin lines: the persistent-process pool on
/// the Go side multiplexes unrelated scenes/videos onto the same worker, so
/// in-process cross-line state would leak bindings between unrelated runs.
/// Cross-scene reuse is therefore supplied per request by the caller (which
/// owns the composition), mirroring the `BoundAssets` ledger the library
/// threads through `sample_scene`.
#[derive(Debug, Deserialize)]
struct BoundEntry {
    asset_id: String,
    scene_id: String,
}

#[derive(Debug, Deserialize)]
struct SampleRequest {
    // Envelope tolerated for forward compatibility with the Go adapter,
    // which sends version/operation alongside the selection payload.
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    operation: Option<String>,
    scene: Scene,
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default)]
    allow_reuse: bool,
    /// Passthrough for `SampleOptions.images_per_scene` (0 disables).
    #[serde(default)]
    images_per_scene: usize,
    /// Per-request cross-scene ledger (see `BoundEntry`). Empty (the
    /// default) preserves the historical single-scene behavior.
    #[serde(default)]
    bound_assets: Vec<BoundEntry>,
}

#[derive(Debug, Serialize)]
struct SampleResponse {
    results: Vec<SampleResult>,
    winner_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn error_response(error: &WorkerError) -> String {
    error.log();
    serde_json::json!({
        "results": [],
        "winner_id": null,
        "error": error.message,
        "error_code": error.code,
    })
    .to_string()
}

fn main() {
    if let Err(error) = ndjson_stdio::run_stdio(process_line) {
        eprintln!("mediasampler read: {error}");
        std::process::exit(1);
    }
}

/// Maps one request line to one response line. Never exits on a malformed
/// line: the Go side drives this binary as a persistent worker, so killing
/// the process would drop every queued request behind it.
fn process_line(line: &str) -> String {
    let request: SampleRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return error_response(&WorkerError::new(
                ErrorCode::Decode,
                format!("mediasampler decode: {error}"),
            ));
        }
    };
    // Fail closed on unknown envelope values instead of silently ignoring
    // them: a contract drift between Go and Rust must surface as an error,
    // not as a mis-sampled scene.
    if let Err(error) = check_envelope(
        "mediasampler",
        request.version.as_deref(),
        "mediasampler.v1",
        request.operation.as_deref(),
        "select",
    ) {
        return error_response(&error);
    }
    let mut bound = BoundAssets::new();
    for entry in &request.bound_assets {
        bound.bind(&entry.asset_id, &entry.scene_id);
    }
    let options = SampleOptions {
        allow_reuse: request.allow_reuse,
        images_per_scene: request.images_per_scene,
    };
    let (results, winner_id) =
        sample_scene(&request.scene, &request.candidates, &options, &mut bound);
    match serde_json::to_string(&SampleResponse {
        results,
        winner_id,
        error: None,
    }) {
        Ok(line) => line,
        Err(error) => error_response(&WorkerError::new(
            ErrorCode::Encode,
            format!("mediasampler encode: {error}"),
        )),
    }
}
