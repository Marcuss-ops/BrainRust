//! Video render benchmark (harness = false, custom loop like
//! `benches/phrase_impact.rs`).
//!
//! Measures the `render_stock` canonical timeline executor end-to-end over a
//! SYNTHETIC fixture: `ffmpeg -f lavfi testsrc2` sources (no network, no
//! copyrighted content), a sealed `render-plan.v2` with three contiguous
//! segments, and the production dispatcher path
//! (`pipelinegen_muscles::dispatcher::process`). It reports per-run wall
//! latencies (p50/p95), the ffmpeg wall reported by the response, ms/frame,
//! and effective output fps. The JSON report lands in the ignored
//! `.cache/video-render/rust_video_benchmark.json` (override with
//! `VIDEO_RENDER_BENCH_OUTPUT`). The fixture itself is cached in
//! `.cache/video-render/fixtures/` across runs.
//!
//! Honest scope: this is the software baseline by default (rust never
//! selects GPU compositing). Hardware encode runs use the SAME harness with
//! `VIDEO_RENDER_BENCH_ENCODER=h264_nvenc` (the Go-resolved encoder policy
//! of the payload) and are reported under their codec, so a libx264 vs
//! NVENC comparison compares like with like. Decode stays CPU in both runs
//! (the canonical executor never hwaccel-decodes): an NVENC number is an
//! ENCODE-only speedup, never claimed as a full-GPU pipeline.
//!
//! Run:
//! ```sh
//! cargo bench -p pipelinegen-muscles --bench video_render
//! ```

#![recursion_limit = "256"]

use pipelinegen_muscles::process;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const FPS_NUM: u64 = 30;
const FPS_DEN: u64 = 1;
const FIXTURE_FRAMES: i64 = 120; // 4 s at 30 fps
const SEGMENT_FRAMES: i64 = 30; // 1 s per timeline segment
const SEGMENTS: i64 = 3;
const WARMUPS: usize = 2;
const DEFAULT_MEASURED_RUNS: usize = 10;

#[derive(Serialize)]
struct RunReport {
    run: usize,
    wall_ms: f64,
    ffmpeg_ms: i64,
    encode_ms: i64,
    ms_per_frame: f64,
    effective_fps: f64,
}

#[derive(Serialize)]
struct Report {
    description: &'static str,
    encoder: String,
    fixture: Value,
    encoder_note: &'static str,
    warmups: usize,
    measured_runs: usize,
    duration_frames: i64,
    runs: Vec<RunReport>,
    wall_p50_ms: f64,
    wall_p95_ms: f64,
    ms_per_frame_p50: f64,
    effective_fps_p50: f64,
    output_sha256: Option<String>,
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((fraction * (sorted.len() as f64 - 1.0)).round() as usize).min(sorted.len() - 1);
    sorted[index]
}

fn workspace_bin(name: &str) -> PathBuf {
    // cargo bench runs from the package directory; the muscles binary is
    // directly available from the dep build tree.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(manifest)
        .join("target")
        .join("debug")
        .join(name);
    if path.is_file() {
        return path;
    }
    PathBuf::from(name)
}

fn sha256_file(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .expect("sha256sum is required by the canonical plan validator");
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum digest")
        .to_string()
}

fn fixture_path(cache: &Path) -> PathBuf {
    cache
        .join("fixtures")
        .join(format!("testsrc2_{WIDTH}x{HEIGHT}_{FPS_NUM}fps.mp4"))
}

/// Generates (once per bench invocation) a synthetic 4-second source with
/// ffmpeg lavfi: deterministic shapes + timestamps, yuv420p, closed GOP.
fn ensure_fixture(ffmpeg: &Path, fixture: &Path) {
    let metadata = match fs::metadata(fixture) {
        Ok(metadata) => metadata,
        Err(_) => {
            let _ = fs::remove_file(fixture);
            generate_fixture(ffmpeg, fixture);
            return;
        }
    };
    if metadata.len() == 0 {
        generate_fixture(ffmpeg, fixture);
    }
}

fn generate_fixture(ffmpeg: &Path, fixture: &Path) {
    if let Some(parent) = fixture.parent() {
        fs::create_dir_all(parent).expect("create fixture directory");
    }
    let duration = FIXTURE_FRAMES as f64 / FPS_NUM as f64;
    let args = [
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-f".to_string(),
        "lavfi".to_string(),
        "-i".to_string(),
        format!("testsrc2=size={WIDTH}x{HEIGHT}:rate={FPS_NUM}/{FPS_DEN}:duration={duration}"),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        "ultrafast".to_string(),
        "-crf".to_string(),
        "28".to_string(),
        "-g".to_string(),
        "30".to_string(),
        "-r".to_string(),
        format!("{FPS_NUM}/{FPS_DEN}"),
        fixture.to_string_lossy().into_owned(),
    ];
    let output = Command::new(ffmpeg)
        .args(&args)
        .output()
        .expect("spawn ffmpeg for fixture generation");
    if !output.status.success() {
        panic!(
            "fixture generation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
}

/// Builds one sealed canonical render plan request over the fixture.
fn build_request_value(fixture: &str, fixture_sha: &str, output: &Path, encoder: &str) -> Value {
    let segment = |index: i64| {
        json!({
            "asset_id": "fixture",
            "source": {"start_frame": 15, "frame_count": SEGMENT_FRAMES},
            "timeline": {"start_frame": index * SEGMENT_FRAMES, "frame_count": SEGMENT_FRAMES},
            "z_index": 0,
            "freeze": false,
        })
    };
    json!({
        "version": "mediaexec.v1",
        "operation": "render_stock",
        "input_paths": [fixture],
        "output_path": output.to_string_lossy(),
        // The media policy travels FLATTENED on the wire (serde flatten on
        // MediaConfig), exactly like the Go boundary emits it. The encoder
        // is the ONLY policy lever the harness overrides: for NVENC the
        // executor itself translates fast presets to the p1..p7 scale.
        "codec": encoder,
        "preset": "ultrafast",
        "crf": 28,
        "width": WIDTH,
        "height": HEIGHT,
        "fps_num": FPS_NUM,
        "fps_den": FPS_DEN,
        "keyframe_interval": 30,
        "audio_codec": "aac",
        "audio_bitrate": "128k",
        "sample_rate": 48000,
        "channels": 2,
        "no_audio": true,
        "no_transitions": true,
        "no_effects": true,
        "render_plan": {
            "version": "render-plan.v2",
            "fps_numerator": FPS_NUM,
            "fps_denominator": FPS_DEN,
            "duration_frames": SEGMENTS * SEGMENT_FRAMES,
            "timeline_hash": "b".repeat(64),
            "manifest_sha256": fixture_sha_repeat(fixture_sha),
            "plan_sha256": "c".repeat(64),
            "manifest": [
                {
                    "asset_id": "fixture",
                    "path": fixture,
                    "sha256": fixture_sha,
                    "frame_count": FIXTURE_FRAMES,
                }
            ],
            "video_tracks": [
                {
                    "index": 0,
                    "segments": (0..SEGMENTS).map(segment).collect::<Vec<_>>(),
                }
            ],
        },
    })
}

fn fixture_sha_repeat(sha: &str) -> String {
    // Manifest hash must be a valid lowercase SHA256 value; the plan
    // validator checks shape only for this field, the asset sha256 IS
    // verified against the file bytes.
    format!("{sha}{sha}")[..64].to_string()
}

fn main() {
    let bench_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".cache")
        .join("video-render");
    fs::create_dir_all(&bench_dir).expect("create report directory");
    let ffmpeg_path =
        std::env::var("VIDEO_RENDER_BENCH_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let encoder =
        std::env::var("VIDEO_RENDER_BENCH_ENCODER").unwrap_or_else(|_| "libx264".to_string());
    let ffmpeg = workspace_bin(&ffmpeg_path);
    let fixture = fixture_path(&bench_dir);
    ensure_fixture(&ffmpeg, &fixture);
    let fixture_str = fixture.to_string_lossy().into_owned();
    let fixture_sha = sha256_file(&fixture);

    let runs = std::env::var("VIDEO_RENDER_BENCH_RUNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MEASURED_RUNS);

    let mut run_reports = Vec::with_capacity(runs);
    for run_index in 0..(WARMUPS + runs) {
        let measured = run_index >= WARMUPS;
        // Unique output per run so every run pays the full publication path.
        let output = bench_dir.join(format!("bench_render_{run_index}.mp4"));
        let payload = build_request_value(&fixture_str, &fixture_sha, &output, &encoder);
        let request: pipelinegen_muscles::Request =
            serde_json::from_value(payload).expect("bench request must parse");
        let started = Instant::now();
        let response = process(request);
        let wall = started.elapsed();
        if !response.ok {
            panic!(
                "bench render failed on run {run_index}: {:?}",
                response.error
            );
        }
        if measured {
            let metadata = response.metadata.as_ref().expect("success metadata");
            let ffmpeg_ms = metadata.ffmpeg_ms.unwrap_or(0);
            let encode_ms = metadata.encode_ms.unwrap_or(0);
            run_reports.push(RunReport {
                run: run_index - WARMUPS + 1,
                wall_ms: wall.as_secs_f64() * 1000.0,
                ffmpeg_ms,
                encode_ms,
                ms_per_frame: wall.as_secs_f64() * 1000.0 / (SEGMENTS * SEGMENT_FRAMES) as f64,
                effective_fps: (SEGMENTS * SEGMENT_FRAMES) as f64 / wall.as_secs_f64(),
            });
        }
        let _ = fs::remove_file(&output);
    }

    let mut walls: Vec<f64> = run_reports.iter().map(|run| run.wall_ms).collect();
    walls.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut per_frame: Vec<f64> = run_reports.iter().map(|run| run.ms_per_frame).collect();
    per_frame.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut fps_list: Vec<f64> = run_reports.iter().map(|run| run.effective_fps).collect();
    fps_list.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let report = Report {
        description: "render_stock canonical timeline executor over a synthetic lavfi fixture (honest-scope bench)",
        encoder: encoder.clone(),
        fixture: json!({
            "path": fixture.to_string_lossy(),
            "sha256": fixture_sha,
            "width": WIDTH,
            "height": HEIGHT,
            "fps": format!("{FPS_NUM}/{FPS_DEN}"),
            "frames": FIXTURE_FRAMES,
            "segments": SEGMENTS,
            "segment_frames": SEGMENT_FRAMES,
        }),
        encoder_note: "decode stays CPU for every encoder; hardware runs set VIDEO_RENDER_BENCH_ENCODER=h264_nvenc and are reported under their codec",
        warmups: WARMUPS,
        measured_runs: runs,
        duration_frames: SEGMENTS * SEGMENT_FRAMES,
        runs: run_reports,
        wall_p50_ms: percentile(&walls, 0.50),
        wall_p95_ms: percentile(&walls, 0.95),
        ms_per_frame_p50: percentile(&per_frame, 0.50),
        effective_fps_p50: percentile(&fps_list, 0.50),
        output_sha256: None,
    };

    let report_path = std::env::var("VIDEO_RENDER_BENCH_OUTPUT").unwrap_or_else(|_| {
        bench_dir
            .join("rust_video_benchmark.json")
            .to_string_lossy()
            .into_owned()
    });
    let serialized = serde_json::to_string_pretty(&report).expect("serialize report");
    fs::write(&report_path, &serialized).expect("write report");

    println!("── video_render bench ──────────────────────────────");
    println!(
        "runs={} warmups={} fixture={}x{}@{} encoder={}",
        runs, WARMUPS, WIDTH, HEIGHT, FPS_NUM, encoder
    );
    println!(
        "wall p50 = {:.1} ms | p95 = {:.1} ms | ms/frame p50 = {:.3} | fps p50 = {:.1}",
        report.wall_p50_ms, report.wall_p95_ms, report.ms_per_frame_p50, report.effective_fps_p50
    );
    println!("report: {report_path}");
}
