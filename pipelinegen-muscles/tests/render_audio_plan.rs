//! End-to-end tests for the combined-audio render (`render_audio_plan`) on
//! the parallel stem fan-out path: the compiled plan is split into stems,
//! mixed concurrently to lossless PCM and encoded to the canonical AAC-LC
//! master in ONE encode.
//!
//! These tests drive the production dispatcher (`process`) over synthetic
//! lavfi fixtures, then inspect the real output with ffprobe/ffmpeg and the
//! certified response metadata: exactly one AAC encode, measured stage
//! timings, a deterministic master, a pinned silent master for a
//! silence-only plan, and fail-closed cleanup with no temporaries left.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn work_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".cache")
        .join("render-audio-plan-tests")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test directory");
    dir
}

/// Mono 24 kHz voiceover (non-canonical: the graph must normalize it) and a
/// stereo 44.1 kHz BGM bed — the two source shapes the mix policy supports.
fn make_assets(dir: &Path) -> (String, String) {
    let vo = dir.join("vo.m4a");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:duration=3",
            "-ar",
            "24000",
            "-ac",
            "1",
            "-c:a",
            "aac",
            vo.to_str().unwrap(),
        ])
        .status()
        .expect("generate voiceover fixture");
    assert!(status.success());
    let bgm = dir.join("bgm.mp3");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=110:duration=6",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-c:a",
            "libmp3lame",
            bgm.to_str().unwrap(),
        ])
        .status()
        .expect("generate BGM fixture");
    assert!(status.success());
    (
        vo.to_string_lossy().into_owned(),
        bgm.to_string_lossy().into_owned(),
    )
}

/// The canonical compiled plan: [SILENCE 1s | VOICEOVER 2s | SILENCE 1s]
/// plus a 4s BGM event ducked under the voiceover track, 26 automations
/// semantics covered by the unit tests — here one triggered duck and one
/// fade keep the real graph on the automation path.
fn compiled_plan() -> Value {
    json!({
        "audio_plan_version": "compiled-audio-plan.v2",
        "duration_us": 4_000_000i64,
        "master_duration_us": 4_000_000i64,
        "tracks": [
            {
                "track_id": "voiceover",
                "role": "VOICEOVER",
                "events": [
                    {"event_id": "sil-1", "type": "SILENCE", "asset_id": null,
                     "timeline_start_us": 0, "duration_us": 1_000_000,
                     "source_in_us": 0, "source_duration_us": 0,
                     "use_original_audio": false, "gain_db": 0.0},
                    {"event_id": "vo-1", "type": "VOICEOVER", "asset_id": "vo",
                     "timeline_start_us": 1_000_000, "duration_us": 2_000_000,
                     "source_in_us": 0, "source_duration_us": 2_000_000,
                     "use_original_audio": false, "gain_db": 0.0},
                    {"event_id": "sil-2", "type": "SILENCE", "asset_id": null,
                     "timeline_start_us": 3_000_000, "duration_us": 1_000_000,
                     "source_in_us": 0, "source_duration_us": 0,
                     "use_original_audio": false, "gain_db": 0.0}
                ]
            },
            {
                "track_id": "bed",
                "role": "BGM",
                "events": [
                    {"event_id": "bgm-1", "type": "BGM", "asset_id": "bgm",
                     "timeline_start_us": 0, "duration_us": 4_000_000,
                     "source_in_us": 0, "source_duration_us": 4_000_000,
                     "use_original_audio": false, "gain_db": -18.0}
                ]
            }
        ],
        "background_music": [],
        "sfx": [],
        "automation": [
            {"target_track_id": "bed", "target_layer": "",
             "trigger_track_id": "voiceover",
             "start_us": 0, "end_us": 4_000_000,
             "gain_db": -12.0, "attack_us": 120_000, "release_us": 350_000},
            {"target_track_id": "bed", "target_layer": "", "trigger_track_id": "",
             "start_us": 0, "end_us": 2_000_000,
             "gain_db": -18.0, "attack_us": 200_000, "release_us": 200_000}
        ],
        "canonical_audio_profile": {
            "codec": "aac", "profile": "lc", "sample_rate": 48000,
            "channels": 2, "channel_layout": "stereo", "bitrate": "128k"
        }
    })
}

fn request(output: &str, plan: Value, assets: Value) -> pipelinegen_muscles::Request {
    serde_json::from_value(json!({
        "version": "mediaexec.v1",
        "operation": "render_audio_plan",
        "ffmpeg_path": "ffmpeg",
        "output_path": output,
        "audio_plan": plan,
        "audio_assets": assets,
    }))
    .expect("decode render_audio_plan request")
}

fn response_value(response: &impl serde::Serialize) -> Value {
    serde_json::to_value(response).expect("encode render_audio_plan response")
}

fn probe_field(path: &str, field: &str) -> String {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "a:0",
            "-show_entries",
            &format!("stream={field}"),
            "-of",
            "csv=p=0",
            path,
        ])
        .output()
        .expect("run ffprobe");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn mean_volume_db(path: &str) -> f64 {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostats",
            "-i",
            path,
            "-af",
            "volumedetect",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("run volumedetect");
    let text = String::from_utf8_lossy(&output.stderr);
    for line in text.lines() {
        if let Some(rest) = line.split("mean_volume: ").nth(1) {
            let value = rest.split_whitespace().next().unwrap_or("");
            if value == "-inf" {
                return f64::NEG_INFINITY;
            }
            return value.parse().expect("parse mean_volume");
        }
    }
    panic!("volumedetect reported no mean_volume for {path}: {text}");
}

fn assert_no_temporaries(dir: &Path) {
    for entry in fs::read_dir(dir).expect("list test directory") {
        let name = entry
            .expect("read entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert!(
            !name.contains(".stem") && !name.contains(".filter") && !name.contains(".part"),
            "temporary artifact left behind: {name}"
        );
    }
}

#[test]
fn render_audio_plan_fans_out_stems_and_encodes_the_master_once() {
    if !ffmpeg_available() {
        return;
    }
    let dir = work_dir("fanout");
    let (vo, bgm) = make_assets(&dir);
    let output = dir.join("master.m4a");
    let response = response_value(&pipelinegen_muscles::process(request(
        output.to_str().unwrap(),
        compiled_plan(),
        json!([
            {"asset_id": "vo", "path": vo},
            {"asset_id": "bgm", "path": bgm}
        ]),
    )));
    assert_eq!(response["ok"], json!(true), "error: {}", response["error"]);

    // Contract metadata: the master is AAC-encoded exactly ONCE; the stem
    // fan-out is a measured mix phase; every stage carries its wall time.
    let metadata = &response["metadata"];
    assert_eq!(metadata["audio_encode_passes"], json!(1), "{metadata}");
    assert!(
        metadata["mix_ms"].as_i64().unwrap_or(-1) >= 0,
        "mix_ms must be a measured phase: {metadata}"
    );
    assert!(
        metadata["aac_encode_ms"].as_i64().unwrap_or(0) > 0,
        "{metadata}"
    );
    assert!(metadata["probe_ms"].as_i64().unwrap_or(0) > 0, "{metadata}");
    assert!(metadata["hash_ms"].as_i64().unwrap_or(0) > 0, "{metadata}");
    let digest = metadata["final_audio_sha256"].as_str().unwrap_or("");
    assert_eq!(digest.len(), 64, "{metadata}");
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "sha256 must be lowercase hex: {digest}"
    );
    let duration = metadata["duration_sec"].as_f64().unwrap_or(0.0);
    assert!((duration - 4.0).abs() <= 0.040, "duration {duration}");

    // Canonical output contract.
    let path = output.to_str().unwrap();
    assert_eq!(probe_field(path, "codec_name"), "aac");
    assert_eq!(probe_field(path, "profile"), "LC");
    assert_eq!(probe_field(path, "sample_rate"), "48000");
    assert_eq!(probe_field(path, "channels"), "2");

    // The fan-out must actually mix audible material — a silent master would
    // pass duration/codec checks but betray a broken stem sum.
    let mean = mean_volume_db(path);
    assert!(mean > -60.0 && mean < 0.0, "master mean volume {mean} dB");

    assert_no_temporaries(&dir);
}

#[test]
fn render_audio_plan_master_is_deterministic_across_renders() {
    if !ffmpeg_available() {
        return;
    }
    let dir = work_dir("deterministic");
    let (vo, bgm) = make_assets(&dir);
    let assets = json!([
        {"asset_id": "vo", "path": vo},
        {"asset_id": "bgm", "path": bgm}
    ]);
    let mut digests = Vec::new();
    for name in ["one.m4a", "two.m4a"] {
        let output = dir.join(name);
        let response = response_value(&pipelinegen_muscles::process(request(
            output.to_str().unwrap(),
            compiled_plan(),
            assets.clone(),
        )));
        assert_eq!(response["ok"], json!(true), "error: {}", response["error"]);
        digests.push(
            response["metadata"]["final_audio_sha256"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
    }
    assert_eq!(
        digests[0], digests[1],
        "fan-out must replay deterministically"
    );
    assert_eq!(digests[0].len(), 64);
    assert_no_temporaries(&dir);
}

#[test]
fn render_audio_plan_silence_only_plan_renders_pinned_silence() {
    if !ffmpeg_available() {
        return;
    }
    let dir = work_dir("silence");
    let (vo, _) = make_assets(&dir);
    let output = dir.join("silent.m4a");
    let plan = json!({
        "audio_plan_version": "compiled-audio-plan.v2",
        "duration_us": 2_000_000i64,
        "tracks": [
            {
                "track_id": "voiceover",
                "role": "VOICEOVER",
                "events": [
                    {"event_id": "sil-1", "type": "SILENCE", "asset_id": null,
                     "timeline_start_us": 0, "duration_us": 2_000_000,
                     "source_in_us": 0, "source_duration_us": 0,
                     "use_original_audio": false, "gain_db": 0.0}
                ]
            }
        ],
        "canonical_audio_profile": {
            "codec": "aac", "profile": "lc", "sample_rate": 48000,
            "channels": 2, "channel_layout": "stereo", "bitrate": "128k"
        }
    });
    let response = response_value(&pipelinegen_muscles::process(request(
        output.to_str().unwrap(),
        plan,
        json!([{"asset_id": "vo", "path": vo}]),
    )));
    assert_eq!(response["ok"], json!(true), "error: {}", response["error"]);
    assert_eq!(response["metadata"]["audio_encode_passes"], json!(1));
    let duration = response["metadata"]["duration_sec"].as_f64().unwrap_or(0.0);
    assert!((duration - 2.0).abs() <= 0.040, "duration {duration}");
    // Digital silence through AAC sits at the encoder's noise floor
    // (about -91 dB, the same sentinel the loudness fixtures use), not at
    // -inf: a silence-only plan must land there and nowhere near audible.
    let mean = mean_volume_db(output.to_str().unwrap());
    assert!(
        mean <= -80.0,
        "a silence-only plan must render silence: {mean} dB"
    );
    assert_no_temporaries(&dir);
}

#[test]
fn render_audio_plan_fails_closed_on_a_short_source_without_temporaries() {
    if !ffmpeg_available() {
        return;
    }
    let dir = work_dir("failclosed");
    let (_, bgm) = make_assets(&dir);
    let output = dir.join("master.m4a");
    let plan = json!({
        "audio_plan_version": "compiled-audio-plan.v2",
        "duration_us": 4_000_000i64,
        "tracks": [
            {
                "track_id": "bed",
                "role": "BGM",
                "events": [
                    {"event_id": "bgm-1", "type": "BGM", "asset_id": "bgm",
                     "timeline_start_us": 0, "duration_us": 4_000_000,
                     "source_in_us": 0, "source_duration_us": 8_000_000,
                     "use_original_audio": false, "gain_db": -18.0}
                ]
            }
        ],
        "canonical_audio_profile": {
            "codec": "aac", "profile": "lc", "sample_rate": 48000,
            "channels": 2, "channel_layout": "stereo", "bitrate": "128k"
        }
    });
    let response = response_value(&pipelinegen_muscles::process(request(
        output.to_str().unwrap(),
        plan,
        json!([{"asset_id": "bgm", "path": bgm}]),
    )));
    assert_eq!(response["ok"], json!(false));
    let error = response["error"].as_str().unwrap_or("");
    assert!(
        error.contains("shorter than required source range"),
        "unexpected error: {error}"
    );
    assert!(
        !output.exists(),
        "a failed render must not publish an output"
    );
    assert_no_temporaries(&dir);
}
