use super::*;
use super::tests_support::*;
use crate::render_clip::plan::{
    ClipPlanAudio, ClipPlanBackground, ClipPlanSubtitles, ClipPlanWatermark,
};
use std::fs;

#[test]
fn blur_source_graph_is_single_pass_with_cpu_subtitle_stage() {
    let mut p = plan();
    let wm_path = std::env::temp_dir().join("cliprender-wm.png");
    fs::write(&wm_path, b"wm").unwrap();
    let sub_path = std::env::temp_dir().join("cliprender-sub.ass");
    fs::write(&sub_path, b"[Script Info]").unwrap();
    p.watermark = Some(ClipPlanWatermark {
			text: String::new(),
			asset_id: "wm-1".to_string(),
        path: wm_path.to_string_lossy().into_owned(),
        sha256: "a".repeat(64),
        position: "top_right".to_string(),
        opacity: 0.85,
        margin_px: 40,
    });
    p.subtitles = Some(ClipPlanSubtitles {
        mode: "burn".to_string(),
        style_id: Some("shorts-v1".to_string()),
        path: sub_path.to_string_lossy().into_owned(),
        sha256: "a".repeat(64),
    });
    let graph = build_filter_graph(&p, &profile());
    // One graph, one encode: background + foreground + watermark + burn.
    assert!(graph.contains("scale=180:320"), "graph: {graph}");
    assert!(graph.contains("gblur=sigma=5"), "graph: {graph}");
    assert!(graph.contains("force_original_aspect_ratio=decrease"));
    assert!(graph.contains("fps=60/1"));
    assert!(graph.contains("overlay=(W-w)/2:(H-h)/2:shortest=1:format=auto"));
    assert!(graph.contains("colorchannelmixer=aa=0.85"));
    assert!(graph.contains("x=main_w-overlay_w-40"));
    assert!(graph.contains("subtitles=filename="));
    assert!(graph.ends_with("[vfinal]"));
}

#[test]
fn sidecar_subtitles_never_burn_into_the_graph() {
    let mut p = plan();
    p.subtitles = Some(ClipPlanSubtitles {
        mode: "sidecar".to_string(),
        style_id: None,
        path: "/tmp/sub.ass".to_string(),
        sha256: "a".repeat(64),
    });
    let graph = build_filter_graph(&p, &profile());
    assert!(!graph.contains("subtitles="), "graph: {graph}");
    assert!(graph.ends_with("null[vfinal]"));
}

#[test]
fn no_background_skips_bg_chain() {
    let mut p = plan();
    p.background = Some(ClipPlanBackground {
        mode: "none".to_string(),
        asset_id: None,
        path: None,
        sha256: None,
    });
    let graph = build_filter_graph(&p, &profile());
    assert!(!graph.contains("gblur"), "graph: {graph}");
    assert!(!graph.contains("[bg]"), "graph: {graph}");
    assert!(graph.starts_with("[0:v]fps="));
}

#[test]
fn asset_background_uses_second_input() {
    let mut p = plan();
    p.background = Some(ClipPlanBackground {
        mode: "asset".to_string(),
        asset_id: Some("bg-1".to_string()),
        path: Some("/tmp/bg.png".to_string()),
        sha256: Some("a".repeat(64)),
    });
    let graph = build_filter_graph(&p, &profile());
    assert!(graph.contains("[1:v]scale="), "graph: {graph}");
    assert!(!graph.contains("gblur"), "graph: {graph}");
}

#[test]
fn watermark_positions_cover_all_four_corners() {
    let wm = |position: &str| ClipPlanWatermark {
			text: String::new(),
			asset_id: "wm-1".to_string(),
        path: "/tmp/wm.png".to_string(),
        sha256: "a".repeat(64),
        position: position.to_string(),
        opacity: 1.0,
        margin_px: 24,
    };
    let (x, y) = watermark_position(&wm("top_left"), 1080, 1920);
    assert_eq!((x.as_str(), y.as_str()), ("x=24", "y=24"));
    let (x, y) = watermark_position(&wm("top_right"), 1080, 1920);
    assert_eq!((x.as_str(), y.as_str()), ("x=main_w-overlay_w-24", "y=24"));
    let (x, y) = watermark_position(&wm("bottom_left"), 1080, 1920);
    assert_eq!((x.as_str(), y.as_str()), ("x=24", "y=main_h-overlay_h-24"));
    let (x, y) = watermark_position(&wm("bottom_right"), 1080, 1920);
    assert_eq!(
        (x.as_str(), y.as_str()),
        ("x=main_w-overlay_w-24", "y=main_h-overlay_h-24")
    );
}

#[test]
fn filter_path_escaping_handles_colons_quotes_and_backslashes() {
    let escaped = escape_filter_path("/data/clip:1 'final'.ass\\x");
    assert_eq!(escaped, "/data/clip\\:1 '\\''final'\\''.ass\\\\x");
}

#[test]
fn audio_copy_when_compatible() {
    let audio = ClipPlanAudio {
        mode: "copy_if_compatible".to_string(),
        codec: "aac".to_string(),
        sample_rate: 48000,
        channels: 2,
    };
    let (eligible, passes, args) = audio_policy(&audio, &source_metadata(true), "128k");
    assert!(eligible);
    assert_eq!(passes, 0);
    assert_eq!(args, vec!["-c:a", "copy"]);
}

#[test]
fn audio_converts_once_when_incompatible() {
    let audio = ClipPlanAudio {
        mode: "copy_if_compatible".to_string(),
        codec: "aac".to_string(),
        sample_rate: 48000,
        channels: 2,
    };
    let mut metadata = source_metadata(true);
    metadata.sample_rate = Some(44100);
    let (eligible, passes, args) = audio_policy(&audio, &metadata, "128k");
    assert!(!eligible);
    assert_eq!(passes, 1);
    assert!(args.contains(&"-ar".to_string()));
    assert!(args.contains(&"48000".to_string()));
    assert!(!args.contains(&"copy".to_string()));
}

#[test]
fn transcode_mode_always_converts() {
    let audio = ClipPlanAudio {
        mode: "transcode".to_string(),
        codec: "aac".to_string(),
        sample_rate: 48000,
        channels: 2,
    };
    let (eligible, passes, args) = audio_policy(&audio, &source_metadata(true), "128k");
    assert!(!eligible);
    assert_eq!(passes, 1);
    assert!(args.contains(&"-c:a".to_string()));
    assert!(args.contains(&"aac".to_string()));
}

#[test]
fn silent_source_never_reports_copy() {
    let audio = ClipPlanAudio {
        mode: "copy_if_compatible".to_string(),
        codec: "aac".to_string(),
        sample_rate: 48000,
        channels: 2,
    };
    let (eligible, passes, _) = audio_policy(&audio, &source_metadata(false), "128k");
    assert!(!eligible);
    assert_eq!(passes, 0);
}

#[test]
fn bench_accumulator_sums_decode_and_encode_real_time() {
    // Real ffmpeg 4.4/6.x -benchmark_all lines (usec): decode_video,
    // encode_video and flush_video are the only task labels emitted.
    let mut acc = BenchAccumulator::default();
    acc.handle_line("bench:      129 user      114 sys       18 real decode_video 0.0 ");
    acc.handle_line("bench:    28557 user    39220 sys     3924 real decode_video 0.0 ");
    acc.handle_line("bench:      291 user      340 sys      632 real encode_video 0.0 ");
    acc.handle_line("bench:      168 user        0 sys       56 real flush_video 0.0 ");
    acc.handle_line("bench:     1061 user      953 sys      995 real flush_video 0.0 ");
    assert!(acc.saw_bench);
    // The REAL column (microseconds) is what the parser sums — never the
    // user/sys columns, which are CPU-time deltas of the whole process.
    assert_eq!(acc.decode_us, 18 + 3924);
    assert_eq!(acc.encode_us, 632 + 56 + 995);
}

#[test]
fn bench_accumulator_ignores_non_bench_output() {
    let mut acc = BenchAccumulator::default();
    // Non-bench chatter, the maxrss summary line, and malformed bench
    // lines must never count as measured work.
    acc.handle_line("Stream mapping:");
    acc.handle_line("frame=   24 fps=0.0 q=-1.0 Lsize=      29kB time=00:00:00.98");
    acc.handle_line("bench: maxrss=1234KiB");
    acc.handle_line("bench:     129 user      114 sys       18 real");
    assert!(!acc.saw_bench);
    assert_eq!(acc.decode_us, 0);
    assert_eq!(acc.encode_us, 0);
}
