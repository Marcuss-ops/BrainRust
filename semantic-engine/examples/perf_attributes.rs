//! Latency probe for the TICKET-52 attribute/coreference lane.
//!
//! Runs the same generic analyzer over one attribute sentence per certified
//! language, `runs` times, and prints one JSON line with mean/p50/p95/max
//! milliseconds and the process RSS. Used to compare a change against a
//! stashed baseline build on the same machine; it asserts nothing.
//!
//! ```text
//! VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models \
//!   cargo run --release --example perf_attributes -- 20
//! ```

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};
use std::time::Instant;

const LANGUAGES: &[(&str, &str)] = &[
    (
        "it",
        "Marco Rossi ha 45 anni. È alto 188 centimetri. Pesa 80 chilogrammi. È un medico.",
    ),
    (
        "en",
        "Marco Rossi is aged 45. He is 188 centimeters tall. He weighs 80 kilograms.",
    ),
    (
        "es",
        "Marco Rossi tiene 45 años. Mide 188 centímetros. Pesa 80 kilogramos.",
    ),
    (
        "pt",
        "Marco Rossi tem 45 anos. Mede 188 centímetros. Pesa 80 quilos.",
    ),
    (
        "fr",
        "Marco Rossi a 45 ans. Il mesure 188 centimètres. Il pèse 80 kilogrammes.",
    ),
    (
        "de",
        "Marco Rossi ist 45 Jahre alt. Er ist 188 Zentimeter groß. Er wiegt 80 Kilogramm.",
    ),
    (
        "nl",
        "Marco Rossi is 45 jaar oud. Hij is 188 centimeter lang. Hij weegt 80 kilogram.",
    ),
    (
        "pl",
        "Marco Rossi ma 45 lat. Ma 188 centymetrów wzrostu. Waży 80 kilogramów.",
    ),
    (
        "ru",
        "Марко Росси 45 лет. Его рост 188 сантиметров. Он весит 80 килограммов.",
    ),
    (
        "tr",
        "Marco Rossi 45 yaşında. Boyu 188 santimetre. 80 kilogram ağırlığında.",
    ),
    (
        "id",
        "Marco Rossi berusia 45 tahun. Tingginya 188 sentimeter. Beratnya 80 kilogram.",
    ),
];

fn request(language: &str, text: &str) -> ScriptRequest {
    ScriptRequest {
        language: language.to_string(),
        scenes: vec![SceneInput {
            scene_id: "s1".to_string(),
            text: text.to_string(),
        }],
        editorial: None,
        entity_safety_cap: 0,
        reference_date: None,
    }
}

fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse().ok())
    })
}

fn main() {
    let runs: usize = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);

    let requests: Vec<ScriptRequest> = LANGUAGES
        .iter()
        .map(|(language, text)| request(language, text))
        .collect();

    // Warm the models once so the first run does not carry model loading.
    for request in &requests {
        analyze_script(request).expect("analysis must succeed");
    }

    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        for request in &requests {
            analyze_script(request).expect("analysis must succeed");
        }
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }

    samples.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let at = |fraction: f64| samples[((samples.len() as f64 - 1.0) * fraction).round() as usize];
    println!(
        "{{\"runs\":{},\"mean_ms\":{:.2},\"p50_ms\":{:.2},\"p95_ms\":{:.2},\"max_ms\":{:.2},\"rss_kib\":{:?}}}",
        samples.len(),
        mean,
        at(0.5),
        at(0.95),
        samples[samples.len() - 1],
        rss_kib(),
    );
}
