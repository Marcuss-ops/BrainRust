//! Manual performance probe for TICKET-12 WP4.
//!
//! Run with models:
//!   VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models \
//!     cargo test -p semantic-engine --test performance_probe --release -- \
//!     --ignored --nocapture
//!
//! Run model-free (the documented fallback):
//!   cargo test -p semantic-engine --test performance_probe --release -- \
//!     --ignored --nocapture
//!
//! The probe prints p50/p95/p99 for the full 20-scene analysis and for one
//! scene through `visualner::extract` (tokenization + optional UDPipe + NER).
//! It deliberately does not assert the original 25 ms gate: the measured
//! model-free baseline already exceeds it, so the revised SLO is a release
//! owner decision, not a test threshold to weaken silently.
//!
//! Set `BRAINRUST_PERF_ARCHIVE=<dir>` to persist the raw 100 samples as JSON.
//! `BRAINRUST_PERF_LABEL` names the artifacts (`udpipe` or `model-free`); when
//! absent, the label is inferred from `VISUALNER_MODEL_DIR`.

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WARM_RUNS: usize = 100;

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn archive_samples(probe: &str, fixture: &str, durations: &[Duration]) {
    let Ok(dir) = std::env::var("BRAINRUST_PERF_ARCHIVE") else {
        return;
    };
    let label = std::env::var("BRAINRUST_PERF_LABEL").unwrap_or_else(|_| {
        if std::env::var_os("VISUALNER_MODEL_DIR").is_some() {
            "udpipe".to_string()
        } else {
            "model-free".to_string()
        }
    });
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let millis = |duration: Duration| duration.as_secs_f64() * 1_000.0;
    let samples_ms: Vec<f64> = durations.iter().copied().map(millis).collect();
    let mean_ms = samples_ms.iter().sum::<f64>() / samples_ms.len() as f64;
    let payload = serde_json::json!({
        "schema": "ticket12.performance_probe.v1",
        "probe": probe,
        "fixture": fixture,
        "label": label,
        "warm_runs": durations.len(),
        "warmup_included": false,
        "model_dir": std::env::var("VISUALNER_MODEL_DIR").ok(),
        "generated_at_unix_s": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default(),
        "samples_ms": samples_ms,
        "min_ms": samples_ms.iter().copied().fold(f64::INFINITY, f64::min),
        "mean_ms": mean_ms,
        "p50_ms": millis(percentile(&sorted, 0.50)),
        "p95_ms": millis(percentile(&sorted, 0.95)),
        "p99_ms": millis(percentile(&sorted, 0.99)),
        "max_ms": millis(*sorted.last().unwrap_or(&Duration::ZERO)),
    });
    let dir = PathBuf::from(dir);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot create archive dir {}: {error}", dir.display());
        return;
    }
    let path = dir.join(format!("{label}-{probe}.json"));
    match serde_json::to_string_pretty(&payload) {
        Ok(json) => {
            if let Err(error) = std::fs::write(&path, json + "\n") {
                eprintln!("cannot write {}: {error}", path.display());
            } else {
                eprintln!("archived {} samples to {}", durations.len(), path.display());
            }
        }
        Err(error) => eprintln!("cannot serialize archive: {error}"),
    }
}

fn twenty_scene_request() -> ScriptRequest {
    let sentence = "NASA e ESA conducono missioni spaziali mentre SpaceX sviluppa razzi riutilizzabili e i ricercatori osservano Marte.";
    ScriptRequest {
        language: "it".to_string(),
        scenes: (0..20)
            .map(|index| SceneInput {
                scene_id: format!("scene-{index:02}"),
                text: format!("{sentence} {sentence}"),
            })
            .collect(),
        editorial: None,
        entity_safety_cap: 0,

        reference_date: None,
    }
}

#[test]
#[ignore = "manual performance probe; run explicitly in release"]
fn warm_twenty_scene_analysis_percentiles() {
    let request = twenty_scene_request();
    // Warm the lazy engine/model path before sampling steady-state latency.
    analyze_script(&request).expect("warmup analysis");

    let mut durations = Vec::with_capacity(WARM_RUNS);
    for _ in 0..WARM_RUNS {
        let start = Instant::now();
        analyze_script(&request).expect("analysis");
        durations.push(start.elapsed());
    }
    archive_samples(
        "twenty-scene",
        "twenty_scene_it_esa_nasa_spacex_mars",
        &durations,
    );
    durations.sort_unstable();

    let p50 = percentile(&durations, 0.50);
    let p95 = percentile(&durations, 0.95);
    let p99 = percentile(&durations, 0.99);
    let max = *durations.last().unwrap_or(&Duration::ZERO);
    eprintln!(
        "analyze_script 20 scenes: p50={p50:?} p95={p95:?} p99={p99:?} max={max:?} runs={WARM_RUNS} model_dir={:?}",
        std::env::var("VISUALNER_MODEL_DIR").ok()
    );
}

#[test]
#[ignore = "manual performance probe; run explicitly in release"]
fn warm_single_scene_extract_percentiles() {
    let sentence = "NASA e ESA conducono missioni spaziali mentre SpaceX sviluppa razzi riutilizzabili e i ricercatori osservano Marte.";
    let options = visualner::ExtractOptions {
        language: "it".to_string(),
        entity_count: 0,
    };
    // Warm the lazy UDPipe model once.
    visualner::extract(sentence, &options).expect("warmup extract");

    let mut durations = Vec::with_capacity(WARM_RUNS);
    for _ in 0..WARM_RUNS {
        let start = Instant::now();
        visualner::extract(sentence, &options).expect("extract");
        durations.push(start.elapsed());
    }
    archive_samples(
        "single-scene",
        "single_scene_it_esa_nasa_spacex_mars",
        &durations,
    );
    durations.sort_unstable();

    let p50 = percentile(&durations, 0.50);
    let p95 = percentile(&durations, 0.95);
    let p99 = percentile(&durations, 0.99);
    let max = *durations.last().unwrap_or(&Duration::ZERO);
    eprintln!(
        "visualner::extract one scene: p50={p50:?} p95={p95:?} p99={p99:?} max={max:?} runs={WARM_RUNS} model_dir={:?}",
        std::env::var("VISUALNER_MODEL_DIR").ok()
    );
}
