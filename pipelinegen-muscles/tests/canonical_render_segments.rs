//! End-to-end tests for the canonical timeline executor: mid-asset input
//! seek and parallel per-segment rendering followed by a stream-copy concat.
//!
//! These tests drive the production dispatcher (`process`) with a sealed
//! `render-plan.v2` over a synthetic lavfi fixture, then inspect the real
//! output with ffprobe/ffmpeg. The seek test compares decoded pixels against
//! the expected source frame, so a regression that silently renders from
//! frame 0 (or off by one frame) fails instead of merely changing duration.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FPS: i64 = 30;

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
        .join("canonical-seek-tests")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test directory");
    dir
}

/// 60 deterministic frames (2 s at 30 fps) with forced keyframes every 15
/// frames, so an input seek at frame 30 lands inside a GOP.
fn make_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("fixture.mp4");
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={WIDTH}x{HEIGHT}:rate={FPS}:duration=2"),
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-g",
            "15",
            // Match the canonical render's bt709/tv VUI. Without this the
            // fixture and the re-encoded output are interpreted with different
            // YUV->RGB matrices, which adds a constant pixel offset unrelated
            // to seek or frame-selection correctness.
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
            &path.to_string_lossy(),
        ])
        .output()
        .expect("spawn ffmpeg for the fixture");
    assert!(
        output.status.success(),
        "fixture generation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    path
}

fn sha256(path: &Path) -> String {
    pipelinegen_muscles::hash::sha256_file(&path.to_string_lossy()).expect("sha256sum")
}

fn request(fixture: &Path, output: &Path, segments: Value, duration_frames: i64) -> Value {
    let fixture = fixture.to_string_lossy().to_string();
    let fixture_sha = sha256(Path::new(&fixture));
    json!({
        "version": "mediaexec.v1",
        "operation": "render_stock",
        "input_paths": [fixture],
        "output_path": output.to_string_lossy(),
        "codec": "libx264",
        "preset": "ultrafast",
        "crf": 18,
        "width": WIDTH,
        "height": HEIGHT,
        "fps_num": FPS,
        "fps_den": 1,
        "keyframe_interval": 15,
        "audio_codec": "aac",
        "audio_bitrate": "128k",
        "sample_rate": 48000,
        "channels": 2,
        "no_audio": true,
        "no_transitions": true,
        "no_effects": true,
        "render_plan": {
            "version": "render-plan.v2",
            "fps_numerator": FPS,
            "fps_denominator": 1,
            "duration_frames": duration_frames,
            "timeline_hash": "b".repeat(64),
            "manifest_sha256": format!("{fixture_sha}{fixture_sha}")[..64].to_string(),
            "plan_sha256": "c".repeat(64),
            "manifest": [{
                "asset_id": "fixture",
                "path": fixture,
                "sha256": fixture_sha,
                "frame_count": 60,
            }],
            "video_tracks": [{
                "index": 0,
                "segments": segments,
            }],
        },
    })
}

fn segment(source_start: i64, timeline_start: i64, frames: i64, freeze: bool) -> Value {
    json!({
        "asset_id": "fixture",
        "source": {"start_frame": source_start, "frame_count": if freeze { 1 } else { frames }},
        "timeline": {"start_frame": timeline_start, "frame_count": frames},
        "z_index": 0,
        "freeze": freeze,
    })
}

fn render(payload: Value) -> Result<(), String> {
    let request: pipelinegen_muscles::Request =
        serde_json::from_value(payload).expect("request must parse");
    let response = pipelinegen_muscles::process(request);
    if response.ok {
        Ok(())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "unknown error".to_string()))
    }
}

fn frame_count(path: &Path) -> i64 {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
            &path.to_string_lossy(),
        ])
        .output()
        .expect("ffprobe");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("frame count")
}

fn rgb_frame(path: &Path, seconds: f64) -> Vec<u8> {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{seconds:.6}"),
            "-i",
            &path.to_string_lossy(),
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .expect("ffmpeg frame extraction");
    assert!(
        output.status.success(),
        "frame extraction failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn mean_abs_diff(left: &[u8], right: &[u8]) -> f64 {
    assert_eq!(left.len(), right.len(), "frame sizes must match");
    let total: u64 = left
        .iter()
        .zip(right)
        .map(|(a, b)| a.abs_diff(*b) as u64)
        .sum();
    total as f64 / left.len().max(1) as f64
}

#[test]
fn mid_asset_segments_input_seek_is_frame_exact() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let dir = work_dir("mid_asset_seek");
    let fixture = make_fixture(&dir);
    let output = dir.join("out.mp4");
    // 10 frames starting at source frame 30 (inside the second GOP), so a
    // missing input seek renders frame 0 and a wrong residual trim shifts by
    // one frame — both are visible in the pixel comparison below.
    render(request(
        &fixture,
        &output,
        json!([segment(30, 0, 10, false)]),
        10,
    ))
    .expect("render failed");
    assert_eq!(frame_count(&output), 10);

    let rendered = rgb_frame(&output, 0.0);
    let expected = rgb_frame(&fixture, 30.0 / FPS as f64);
    let first_frame = rgb_frame(&fixture, 0.0);
    let diff_expected = mean_abs_diff(&rendered, &expected);
    let diff_first = mean_abs_diff(&rendered, &first_frame);
    assert!(
        diff_expected < 8.0,
        "rendered frame must match source frame 30 (mean abs diff {diff_expected:.2})"
    );
    // Frame 30 is only a few GOPs away, so absolute colour distance is small;
    // the rendered frame must still be clearly closer to the seeked frame than
    // to the asset's first frame.
    assert!(
        diff_first > diff_expected * 2.0 + 3.0,
        "rendered frame must not be source frame 0 (seeked diff {diff_expected:.2}, first-frame diff {diff_first:.2})"
    );
}

#[test]
fn mid_asset_freeze_holds_the_seeked_frame() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let dir = work_dir("mid_asset_freeze");
    let fixture = make_fixture(&dir);
    let output = dir.join("freeze.mp4");
    // One source frame at index 42 held for 12 timeline frames.
    render(request(
        &fixture,
        &output,
        json!([segment(42, 0, 12, true)]),
        12,
    ))
    .expect("render failed");
    assert_eq!(frame_count(&output), 12);
    let expected = rgb_frame(&fixture, 42.0 / FPS as f64);
    for seconds in [0.0, 6.0 / FPS as f64, 11.0 / FPS as f64] {
        let rendered = rgb_frame(&output, seconds);
        let diff = mean_abs_diff(&rendered, &expected);
        assert!(
            diff < 8.0,
            "freeze at {seconds:.3}s must hold source frame 42 (mean abs diff {diff:.2})"
        );
    }
}

#[test]
fn parallel_segments_concatenate_into_the_full_timeline() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let dir = work_dir("parallel_concat");
    let fixture = make_fixture(&dir);
    let output = dir.join("concat.mp4");
    let segments = json!([
        segment(0, 0, 10, false),
        segment(20, 10, 10, false),
        segment(40, 20, 10, false),
    ]);
    render(request(&fixture, &output, segments.clone(), 30)).expect("render failed");
    assert_eq!(frame_count(&output), 30);

    // Determinism: the same plan rendered again must produce identical bytes,
    // so a parallel run can be cached/deduped exactly like a serial one.
    let second = dir.join("concat_second.mp4");
    render(request(&fixture, &second, segments, 30)).expect("second render failed");
    assert_eq!(
        sha256(&output),
        sha256(&second),
        "parallel segment concat must be byte-deterministic"
    );

    // Each third of the output comes from its own source offset.
    for (timeline_seconds, source_frame) in [
        (0.0, 0.0),
        (10.0 / FPS as f64, 20.0),
        (20.0 / FPS as f64, 40.0),
    ] {
        let rendered = rgb_frame(&output, timeline_seconds);
        let expected = rgb_frame(&fixture, source_frame / FPS as f64);
        let diff = mean_abs_diff(&rendered, &expected);
        assert!(
            diff < 8.0,
            "segment at {timeline_seconds:.3}s must match source frame {source_frame} (diff {diff:.2})"
        );
    }
}

#[test]
fn multi_manifest_projection_matches_single_artifact_projection() {
    assert!(
        ffmpeg_available(),
        "ffmpeg is required for projection parity"
    );
    let dir = work_dir("multi-manifest-projection");
    let fixture = make_fixture(&dir);
    let digest = sha256(&fixture);
    let size = fs::metadata(&fixture).unwrap().len();
    let entry = |id: &str, start: i64, end: i64, hash: &str| {
        json!({
            "item_id": id, "track": 0, "in_us": start, "out_us": end,
            "content_hash": hash.repeat(64)
        })
    };
    let manifest = |entries: Value| {
        json!({
            "schema_version": "pipelinegen.overlay-manifest.v1", "plan_id": "plan",
            "video_id": "video", "timeline_sha256": digest,
            "asset": {"asset_id": "fixture", "sha256": digest, "size_bytes": size,
                      "frame_count": 60, "duration_ms": 2000}, "entries": entries
        })
    };
    let single = manifest(json!([
        entry("a", 0, 500000, "a"),
        entry("b", 1000000, 1500000, "b")
    ]));
    let set = json!({"schema_version": "pipelinegen.overlay-manifest-set.v1", "manifests": [
        manifest(json!([entry("a", 0, 500000, "a")])),
        manifest(json!([entry("b", 1000000, 1500000, "b")]))
    ]});
    let project = |wire: Value, inputs: Vec<String>, output: &Path| -> Result<(), String> {
        render(
            json!({"version":"mediaexec.v1", "operation":"project_overlay_manifest",
            "source_path": fixture, "input_paths": inputs, "output_path": output,
            "overlay_manifest": wire, "codec":"libx264", "preset":"ultrafast", "crf":18,
            "width":WIDTH, "height":HEIGHT, "fps_num":FPS, "fps_den":1,
            "keyframe_interval":15, "audio_codec":"aac", "audio_bitrate":"128k",
            "sample_rate":48000, "channels":2 }),
        )
    };
    let path = fixture.to_string_lossy().to_string();
    let single_out = dir.join("single.mp4");
    let multi_out = dir.join("multi.mp4");
    project(single, vec![path.clone()], &single_out).unwrap();
    project(set.clone(), vec![path.clone(), path.clone()], &multi_out).unwrap();
    assert_eq!(frame_count(&single_out), 60);
    assert_eq!(frame_count(&multi_out), 60);
    assert_eq!(sha256(&single_out), sha256(&multi_out));
    let mut tampered = set;
    tampered["manifests"][1]["asset"]["sha256"] = json!("f".repeat(64));
    tampered["manifests"][1]["timeline_sha256"] = json!("f".repeat(64));
    assert!(project(tampered, vec![path.clone(), path], &dir.join("bad.mp4")).is_err());
}
