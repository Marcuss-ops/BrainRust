use pipelinegen_muscles::phrase_impact::{run, split_sentences, Options, Request, SummaryLength};
use serde::Serialize;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

const WARMUP_RUNS: usize = 10;
const MEASURED_RUNS: usize = 100;
const EMBEDDING_DIMENSIONS: usize = 384;

#[derive(Serialize)]
struct Percentiles {
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

#[derive(Serialize)]
struct PhaseReport {
    split_ms: Percentiles,
    embedding_ms: Percentiles,
    embedding_measured_by_runner: bool,
    similarity_ms: Percentiles,
    ranking_ms: Percentiles,
    summary_ms: Percentiles,
    bullet_ms: Percentiles,
    total_ms: Percentiles,
    words: usize,
    sentences: usize,
    measured_runs: usize,
}

#[derive(Serialize)]
struct ThroughputReport {
    videos: usize,
    threads: usize,
    elapsed_ms: f64,
    videos_per_second: f64,
    sentences_per_second: f64,
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
    transcript_sizes: Vec<PhaseReport>,
    throughput: Vec<ThroughputReport>,
    memory_stress_runs: usize,
}

fn fixture_text(target_words: usize) -> String {
    let mut words = Vec::with_capacity(target_words);
    for index in 0..target_words {
        let word = match index % 23 {
            0 => "Company",
            1 => "announced",
            2 => "revenue",
            3 => "growth",
            4 => "after",
            5 => "the",
            6 => "quarterly",
            7 => "meeting",
            8 => "investors",
            9 => "responded",
            10 => "positively",
            11 => "with",
            12 => "a",
            13 => "record",
            14 => "35%",
            15 => "increase",
            16 => "across",
            17 => "international",
            18 => "markets",
            19 => "during",
            20 => "2026",
            21 => "operations",
            _ => "continued",
        };
        let token = if index % 20 == 19 {
            format!("{word}.")
        } else {
            word.to_string()
        };
        words.push(token);
    }
    if let Some(last) = words.last_mut() {
        if !last.ends_with('.') {
            last.push('.');
        }
    }
    words.join(" ")
}

fn synthetic_embeddings(count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|index| {
            let mut vector = vec![0.0; EMBEDDING_DIMENSIONS];
            let topic = index % 12;
            vector[topic] = 1.0;
            vector[12 + (index % 64)] = 0.12;
            vector[128 + ((index.wrapping_mul(37)) % 256)] = 0.05;
            vector
        })
        .collect()
}

fn request(text: String) -> Request {
    let sentence_count = split_sentences(&text, "en").len();
    Request {
        transcript: text,
        language: "en".to_string(),
        embeddings: synthetic_embeddings(sentence_count),
        timings: Vec::new(),
        options: Options {
            summary_length: SummaryLength::Medium,
            ..Options::default()
        },
        embedding_ms: 0.0,
    }
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

fn benchmark_size(words: usize) -> PhaseReport {
    let text = fixture_text(words);
    let input = request(text);
    let sentence_count = input.embeddings.len();
    for _ in 0..WARMUP_RUNS {
        let mut warmup_request = input.clone();
        warmup_request.embedding_ms = 0.0;
        black_box(run(warmup_request).expect("benchmark fixture is valid"));
    }
    let mut split = Vec::with_capacity(MEASURED_RUNS);
    let embedding = vec![0.0; MEASURED_RUNS];
    let mut similarity = Vec::with_capacity(MEASURED_RUNS);
    let mut ranking = Vec::with_capacity(MEASURED_RUNS);
    let mut summary = Vec::with_capacity(MEASURED_RUNS);
    let mut bullet = Vec::with_capacity(MEASURED_RUNS);
    let mut total = Vec::with_capacity(MEASURED_RUNS);
    for _ in 0..MEASURED_RUNS {
        let mut measured_request = input.clone();
        measured_request.embedding_ms = 0.0;
        let result = black_box(run(measured_request).expect("benchmark fixture is valid"));
        split.push(result.timings.split_ms);
        similarity.push(result.timings.similarity_ms);
        ranking.push(result.timings.ranking_ms);
        summary.push(result.timings.summary_ms);
        bullet.push(result.timings.bullet_ms);
        total.push(result.timings.total_ms);
    }
    PhaseReport {
        split_ms: percentiles(&split),
        embedding_ms: percentiles(&embedding),
        embedding_measured_by_runner: false,
        similarity_ms: percentiles(&similarity),
        ranking_ms: percentiles(&ranking),
        summary_ms: percentiles(&summary),
        bullet_ms: percentiles(&bullet),
        total_ms: percentiles(&total),
        words,
        sentences: sentence_count,
        measured_runs: MEASURED_RUNS,
    }
}

fn throughput_case(videos: usize, threads: usize, input: &Request) -> ThroughputReport {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let sentence_count = input.embeddings.len();
    let start = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if index >= videos {
                    break;
                }
                black_box(run(input.clone()).expect("throughput fixture is valid"));
            });
        }
    });
    let elapsed = start.elapsed().as_secs_f64();
    ThroughputReport {
        videos,
        threads,
        elapsed_ms: elapsed * 1000.0,
        videos_per_second: videos as f64 / elapsed,
        sentences_per_second: (videos * sentence_count) as f64 / elapsed,
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

fn process_cpu_ticks() -> Option<f64> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    let close = stat.rfind(')')?;
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    let user_ticks = fields.get(11)?.parse::<f64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<f64>().ok()?;
    Some(user_ticks + system_ticks)
}

fn main() {
    let started = Instant::now();
    let cpu_start = process_cpu_ticks();
    let (initial_peak_rss_mb, initial_rss_mb) = process_memory_mb();
    let transcript_sizes = [250, 500, 1_000, 2_500, 5_000, 10_000]
        .into_iter()
        .map(benchmark_size)
        .collect();
    let throughput_input = request(fixture_text(100));
    let mut throughput = Vec::new();
    for videos in [1, 10, 100, 1_000] {
        for threads in [1, 2, 4, 8] {
            throughput.push(throughput_case(videos, threads, &throughput_input));
        }
    }
    let memory_stress_runs = 10_000;
    let memory_input = request(fixture_text(250));
    for _ in 0..memory_stress_runs {
        black_box(run(memory_input.clone()).expect("memory stress fixture is valid"));
    }
    let (final_peak_rss_mb, final_rss_mb) = process_memory_mb();
    let cpu_percent = cpu_start
        .zip(process_cpu_ticks())
        .and_then(|(before, after)| {
            let elapsed = started.elapsed().as_secs_f64();
            (elapsed > 0.0).then_some((after - before) / 100.0 / elapsed * 100.0)
        });
    let report = Report {
        description: "Synthetic benchmark of the isolated Rust phrase-impact library; no production inference or service is invoked.",
        embedding_note: "Embeddings are deterministic synthetic 384D vectors. Rust embedding_ms is zero; model inference, its RAM and its throughput are external and not measured here.",
        warmup_runs: WARMUP_RUNS,
        measured_runs: MEASURED_RUNS,
        peak_rss_mb: final_peak_rss_mb.or(initial_peak_rss_mb),
        initial_rss_mb,
        final_rss_mb,
        process_cpu_percent: cpu_percent,
        transcript_sizes,
        throughput,
        memory_stress_runs,
    };
    let json = serde_json::to_string_pretty(&report).expect("benchmark report serializes");
    let output_path = std::env::var("PHRASE_IMPACT_BENCH_OUTPUT")
        .unwrap_or_else(|_| ".cache/phrase-impact/rust_benchmark.json".to_string());
    if let Some(parent) = Path::new(&output_path).parent() {
        fs::create_dir_all(parent).expect("create benchmark output directory");
    }
    fs::write(&output_path, &json).expect("write benchmark report");
    println!("{json}");
}
