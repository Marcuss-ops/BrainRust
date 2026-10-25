//! Benchmark: VisualNER extraction.
//!
//! Two scenarios, in this order (the caches are process-wide, so the cold
//! burst must run before any warm-up):
//!
//! 1. **cold burst** — N threads start at a barrier and extract in three
//!    different languages at once. This is what the NDJSON worker pool does
//!    on the first requests, and it is where per-thread model loading pays
//!    its cost. Reports wall time and process peak RSS.
//! 2. **steady state** — `extract` / `analyze` on 1k/3k/10k-word inputs
//!    with the model already loaded (the per-request cost).
//!
//! Reports p50/p95/p99 like the other workspace benches; performance limits
//! are set against this measured baseline, never invented beforehand.
//!
//! Run:
//! ```sh
//! cargo bench -p visualner --bench extract
//! ```

use std::fs;
use std::hint::black_box;
use std::thread;
use std::time::Instant;
use visualner::{analyze, extract, ExtractOptions};

const WARMUP_RUNS: usize = 3;
const MEASURED_RUNS: usize = 15;
const COLD_THREADS: usize = 8;
const COLD_LANGUAGES: &[&str] = &["it", "en", "es"];

const SENTENCE_POOL: &[(&str, &str)] = &[
    (
        "it",
        "Nel 2020 Siemens annunciò investimenti da 120 milioni mentre i costi energetici riducevano i margini.",
    ),
    (
        "it",
        "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini industriali del settore.",
    ),
    (
        "it",
        "L'aumento dei costi energetici ha contribuito alla riduzione dei margini aziendali.",
    ),
    (
        "it",
        "Volkswagen collabora con Bosch sulle batterie mentre BMW compete con Mercedes in Europa.",
    ),
    (
        "en",
        "In 2020 Siemens announced 120 million in investments while energy costs reduced margins.",
    ),
    (
        "en",
        "Ford grows with hybrids while Toyota slows on industrial margins across the sector.",
    ),
    (
        "en",
        "The rise in energy costs contributed to the reduction of corporate profit margins.",
    ),
    (
        "es",
        "En 2020 Siemens anunció inversiones de 120 millones mientras los costos energéticos reducían los márgenes.",
    ),
    (
        "es",
        "Ford crece con los híbridos mientras Toyota frena en los márgenes industriales del sector.",
    ),
];

struct Percentiles {
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

fn percentiles(mut runs: Vec<f64>) -> Percentiles {
    runs.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    let at = |q: f64| {
        let index = ((runs.len() as f64 - 1.0) * q).round() as usize;
        runs[index]
    };
    Percentiles {
        p50_ms: at(0.50),
        p95_ms: at(0.95),
        p99_ms: at(0.99),
    }
}

fn peak_rss_mb() -> Option<f64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kib: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024.0)
}

/// Deterministic fixture: `target_words` words of mixed sentences.
fn fixture(language: &str, target_words: usize) -> String {
    let sentences: Vec<&str> = SENTENCE_POOL
        .iter()
        .filter(|(lang, _)| *lang == language)
        .map(|(_, text)| *text)
        .collect();
    let mut text = String::new();
    let mut words = 0usize;
    let mut index = 0usize;
    while words < target_words {
        text.push_str(sentences[index % sentences.len()]);
        text.push(' ');
        words += sentences[index % sentences.len()].split_whitespace().count();
        index += 1;
    }
    text
}

fn run_extract(text: &str, language: &str) -> usize {
    let options = ExtractOptions {
        language: language.to_string(),
        entity_count: 32,
    };
    extract(text, &options).expect("extract").len()
}

fn cold_burst() {
    let languages: Vec<String> = COLD_LANGUAGES
        .iter()
        .cycle()
        .take(COLD_THREADS)
        .map(|language| (*language).to_string())
        .collect();
    let texts: Vec<String> = languages
        .iter()
        .map(|language| fixture(language, 200))
        .collect();
    let barrier = std::sync::Barrier::new(languages.len());
    let started = Instant::now();
    thread::scope(|scope| {
        for (language, text) in languages.iter().zip(texts.iter()) {
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                let options = ExtractOptions {
                    language: language.clone(),
                    entity_count: 32,
                };
                black_box(extract(text, &options).expect("cold extract"));
            });
        }
    });
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "cold_burst threads={COLD_THREADS} languages={} wall_ms={wall_ms:.1} peak_rss_mb={:.1}",
        languages.len(),
        peak_rss_mb().unwrap_or(0.0)
    );
}

fn steady_state() {
    println!("── visualner extract bench ────────────────────────────");
    for (language, target_words) in [("it", 1000), ("it", 3000), ("it", 10000), ("en", 3000)] {
        let text = fixture(language, target_words);
        let actual_words = text.split_whitespace().count();
        for _ in 0..WARMUP_RUNS {
            black_box(run_extract(&text, language));
        }
        let mut runs = Vec::with_capacity(MEASURED_RUNS);
        for _ in 0..MEASURED_RUNS {
            let started = Instant::now();
            black_box(run_extract(&text, language));
            runs.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let p = percentiles(runs);
        println!(
            "extract lang={language} words={actual_words} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} runs={MEASURED_RUNS}",
            p.p50_ms, p.p95_ms, p.p99_ms
        );
    }
    for _ in 0..WARMUP_RUNS {
        black_box(analyze(&fixture("it", 3000), &ExtractOptions {
            language: "it".to_string(),
            entity_count: 32,
        })
        .expect("analyze warmup"));
    }
    let text = fixture("it", 3000);
    let mut runs = Vec::with_capacity(MEASURED_RUNS);
    for _ in 0..MEASURED_RUNS {
        let started = Instant::now();
        black_box(analyze(&text, &ExtractOptions {
            language: "it".to_string(),
            entity_count: 32,
        })
        .expect("analyze"));
        runs.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let p = percentiles(runs);
    println!(
        "analyze lang=it words=3000 p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} runs={MEASURED_RUNS}",
        p.p50_ms, p.p95_ms, p.p99_ms
    );
}

fn main() {
    cold_burst();
    steady_state();
    println!(
        "final_peak_rss_mb={:.1}",
        peak_rss_mb().unwrap_or(0.0)
    );
}
