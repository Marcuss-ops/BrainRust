//! Real-media integration tests for the preflight engine.
//!
//! The engine must judge actual files, not a mock: these tests render a tiny
//! clip with ffmpeg, probe it through the same `probe_file` path the render
//! operations use, and assert that a check writes nothing at all.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use pipelinegen_muscles::preflight::{
    check, PreflightAsset, PreflightCue, PreflightOverlay, PreflightRequest, Severity,
};

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "muscles-preflight-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn render_clip(path: &Path) {
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x64:rate=24:duration=1",
            "-c:v",
            "mpeg4",
        ])
        .arg(path)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg failed to render the fixture");
}

fn request(assets: Vec<PreflightAsset>) -> PreflightRequest {
    PreflightRequest {
        video_duration_ms: Some(1_000),
        assets,
        subtitles: Vec::new(),
        overlays: Vec::new(),
        ffmpeg_path: None,
    }
}

fn asset(id: &str, path: &Path) -> PreflightAsset {
    PreflightAsset {
        asset_id: id.to_string(),
        path: path.to_string_lossy().into_owned(),
    }
}

#[test]
fn a_real_video_passes_and_the_check_writes_nothing() {
    let dir = temp_dir("pass");
    let clip = dir.join("clip.mp4");
    render_clip(&clip);

    let mut before = std::fs::read_dir(&dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    before.sort();

    let mut request = request(vec![asset("clip", &clip)]);
    request.subtitles = vec![PreflightCue {
        item_id: "c1".to_string(),
        in_us: 0,
        out_us: 1_000_000,
        text: "Testo leggibile.".to_string(),
    }];
    request.overlays = vec![PreflightOverlay {
        item_id: "o1".to_string(),
        track: 0,
        in_us: 100_000,
        out_us: 900_000,
    }];

    let report = check(&request);
    assert!(report.ok, "unexpected issues: {:?}", report.issues);
    assert!(report.issues.is_empty());

    let mut after = std::fs::read_dir(&dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    after.sort();
    assert_eq!(before, after, "preflight must not create or delete files");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_asset_is_a_blocking_error() {
    let dir = temp_dir("missing");
    let report = check(&request(vec![asset("ghost", &dir.join("nope.mp4"))]));
    assert!(!report.ok);
    assert_eq!(report.issues[0].code, "asset_missing");
    assert_eq!(report.issues[0].severity, Severity::Error);
    assert_eq!(report.issues[0].item_id.as_deref(), Some("ghost"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_non_media_file_is_unreadable() {
    let dir = temp_dir("not-media");
    let fake = dir.join("fake.mp4");
    std::fs::write(&fake, b"this is not a video").expect("write fake asset");

    let report = check(&request(vec![asset("fake", &fake)]));
    assert!(!report.ok);
    assert_eq!(report.issues[0].code, "asset_unreadable");
    let expected = fake.to_string_lossy().into_owned();
    assert_eq!(report.issues[0].path.as_deref(), Some(expected.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_audio_only_file_has_no_video_stream() {
    let dir = temp_dir("audio-only");
    let audio = dir.join("tone.m4a");
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:a",
            "aac",
        ])
        .arg(&audio)
        .status()
        .expect("run ffmpeg");
    assert!(
        status.success(),
        "ffmpeg failed to render the audio fixture"
    );

    let report = check(&request(vec![asset("tone", &audio)]));
    assert!(!report.ok);
    assert_eq!(report.issues[0].code, "asset_no_video");
    let _ = std::fs::remove_dir_all(dir);
}
