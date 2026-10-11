//! Benchmark: Scene Intelligence lexical analysis (1k / 3k / 10k words).
//!
//! Lexical-only by design: this module uses no embeddings (no E5 path
//! exists to measure — entity surfaces arrive from the existing NLP /
//! semantic context). The E5 mode of the certification matrix therefore
//! applies to the phrase-impact pipeline, not here; model inference RAM
//! and throughput stay external and unmeasured.
//!
//! Reports p50/p95/p99 for context build + full analysis, CPU ticks,
//! RSS high-water mark, sentences and throughput. Performance limits must
//! be set against this measured baseline, never invented beforehand.

use pipelinegen_muscles::scene_intelligence::{analyze, SceneText, SharedContext};
use serde::Serialize;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

const WARMUP_RUNS: usize = 5;
const MEASURED_RUNS: usize = 20;

#[derive(Serialize)]
struct Percentiles {
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

#[derive(Serialize)]
struct SizeReport {
    words: usize,
    scenes: usize,
    sentences: usize,
    context_ms: Percentiles,
    analyze_ms: Percentiles,
    total_ms: Percentiles,
    sentences_per_second: f64,
    measured_runs: usize,
}

#[derive(Serialize)]
struct Report {
    description: &'static str,
    embedding_note: &'static str,
    warmup_runs: usize,
    measured_runs: usize,
    peak_rss_mb: Option<f64>,
    initial_rss_mb: Option<f64>,
    final_rss_mb: Option<f64>,
    process_cpu_percent: Option<f64>,
    sizes: Vec<SizeReport>,
}

const SENTENCE_POOL: &[&str] = &[
    "Nel 2020 Siemens annunciò investimenti da 120 milioni mentre i costi energetici riducevano i margini.",
    "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini industriali.",
    "L'aumento dei costi energetici ha contribuito alla riduzione dei margini aziendali.",
    "Le vendite sono diminuite dopo l'aumento dei prezzi del terzo trimestre.",
    "Volkswagen collabora con Bosch sulle batterie mentre BMW compete con Mercedes.",
    "Tre anni dopo gli investimenti crollarono e il progetto fu abbandonato.",
    "Il giorno era bello e calmo senza particolari novità da segnalare.",
];

fn fixture_scenes(target_words: usize) -> (Vec<SceneText>, Vec<Vec<String>>) {
    let mut scenes = Vec::new();
    let mut entities = Vec::new();
    let mut words = 0usize;
    let mut scene_idx = 0usize;
    while words < target_words {
        let mut text = String::new();
        for k in 0..8 {
            text.push_str(SENTENCE_POOL[(scene_idx + k) % SENTENCE_POOL.len()]);
            text.push(' ');
        }
        words += text.split_whitespace().count();
        scenes.push(SceneText {
            scene_id: format!("scene-{:03}", scene_idx),
            text,
            language: "it".to_string(),
            video_start_ms: (scene_idx as i64) * 20_000,
            video_end_ms: (scene_idx as i64 + 1) * 20_000,
        });
        entities.push(vec![
            "Siemens".to_string(),
            "Ford".to_string(),
            "Toyota".to_string(),
            "Volkswagen".to_string(),
            "Bosch".to_string(),
        ]);
        scene_idx += 1;
    }
    (scenes, entities)
}

fn percentiles(samples: &[f64]) -> Percentiles {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |p: f64| {
        let index = ((sorted.len().saturating_sub(1)) as f64 * p).ceil() as usize;
        sorted[index.min(sorted.len() - 1)]
    };
    Percentiles {
        p50_ms: at(0.50),
        p95_ms: at(0.95),
        p99_ms: at(0.99),
    }
}

fn benchmark_size(target_words: usize) -> SizeReport {
    let (scenes, entities) = fixture_scenes(target_words);
    let sentence_count = SharedContext::build(&scenes).sentences.len();
    let words: usize = scenes
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum();
    for _ in 0..WARMUP_RUNS {
        black_box(analyze(black_box(&scenes), black_box(&entities)));
    }
    let mut context = Vec::with_capacity(MEASURED_RUNS);
    let mut analyze_ms = Vec::with_capacity(MEASURED_RUNS);
    let mut total = Vec::with_capacity(MEASURED_RUNS);
    for _ in 0..MEASURED_RUNS {
        let t0 = Instant::now();
        let ctx = SharedContext::build(&scenes);
        let c_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = Instant::now();
        black_box(analyze(black_box(&scenes), black_box(&entities)));
        let a_ms = t1.elapsed().as_secs_f64() * 1000.0;
        let _ = black_box(ctx);
        context.push(c_ms);
        analyze_ms.push(a_ms);
        total.push(c_ms + a_ms);
    }
    let total_p = percentiles(&total);
    SizeReport {
        words,
        scenes: scenes.len(),
        sentences: sentence_count,
        context_ms: percentiles(&context),
        analyze_ms: percentiles(&analyze_ms),
        sentences_per_second: sentence_count as f64 / (total_p.p50_ms / 1000.0),
        total_ms: total_p,
        measured_runs: MEASURED_RUNS,
    }
}

fn process_memory_mb() -> (Option<f64>, Option<f64>) {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    let mut high_water_kb = None;
    let mut current_kb = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmHWM:") {
            high_water_kb = value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok());
        }
        if let Some(value) = line.strip_prefix("VmRSS:") {
            current_kb = value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok());
        }
    }
    (
        high_water_kb.map(|value| value / 1024.0),
        current_kb.map(|value| value / 1024.0),
    )
}

fn main() {
    let started = Instant::now();
    let (initial_peak_rss_mb, initial_rss_mb) = process_memory_mb();
    let sizes = [1_000, 3_000, 10_000]
        .into_iter()
        .map(benchmark_size)
        .collect();
    let stat = fs::read_to_string("/proc/self/stat").ok();
    let cpu_ticks: Option<f64> = stat.as_deref().and_then(|st| {
        let close = st.rfind(')')?;
        let fields: Vec<&str> = st[close + 1..].split_whitespace().collect();
        Some(fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?)
    });
    let (final_peak_rss_mb, final_rss_mb) = process_memory_mb();
    let cpu_percent = cpu_ticks.map(|after| {
        let elapsed = started.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            after / 100.0 / elapsed * 100.0
        } else {
            0.0
        }
    });
    let report = Report {
        description: "Lexical-only benchmark of scene_intelligence; no embeddings, no model inference, no service is invoked.",
        embedding_note: "No E5 mode exists for this module by design: entity surfaces arrive from the existing NLP/semantic context. The certification E5 column applies to the phrase-impact pipeline.",
        warmup_runs: WARMUP_RUNS,
        measured_runs: MEASURED_RUNS,
        peak_rss_mb: final_peak_rss_mb.or(initial_peak_rss_mb),
        initial_rss_mb,
        final_rss_mb,
        process_cpu_percent: cpu_percent,
        sizes,
    };
    let json = serde_json::to_string_pretty(&report).expect("benchmark report serializes");
    let output_path = std::env::var("SCENE_INTELLIGENCE_BENCH_OUTPUT")
        .unwrap_or_else(|_| ".cache/scene-intelligence/rust_benchmark.json".to_string());
    if let Some(parent) = Path::new(&output_path).parent() {
        fs::create_dir_all(parent).expect("create benchmark output directory");
    }
    fs::write(&output_path, &json).expect("write benchmark report");
    println!("{json}");
}
