// render_clip.rs — the single-pass clip render operation (feature spec §6/§7).
//
// Executes a sealed ClipRenderPlanV1 with ONE FFmpeg invocation: background
// (none | blur_source | asset), aspect-fitted foreground, watermark overlay,
// optional libass subtitle burn, and the encoder from the canonical
// encoder/media policy (encoder.rs — the only builder allowed to emit video
// encoder arguments). Rust makes NO business selections: every value arrives
// resolved in the plan (geometry, watermark position/opacity/margin, subtitle
// mode, audio policy); the media config carries only the encoder policy.
//
// Audio copy policy (§9): copy_if_compatible probes the source audio stream
// and copies it verbatim when it already satisfies the plan's audio contract;
// otherwise exactly one certified conversion runs. The outcome is reported in
// the response metadata (audio_copy_eligible, audio_encode_passes).
//
// Honest GPU accounting (§7): burn subtitles rasterize via libass (CPU); the
// response reports subtitle_raster_cpu=true — the operation never claims a
// 100% GPU path when libass is in the graph.

use crate::artifact::{failed_response, part_path, publish_output};
use crate::config::VideoProfile;
use crate::encoder::append_video_args;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use crate::probe;
use std::fs;
use std::path::Path;

#[path = "render_clip_plan.rs"]
mod plan;

use plan::{
    ClipPlanAudio, ClipPlanWatermark, ClipRenderPlan, AUDIO_COPY_IF_COMPATIBLE, BACKGROUND_ASSET,
    BACKGROUND_NONE, SUBTITLE_BURN,
};

pub(super) fn render_clip(request: Request) -> Response {
    let raw_plan = match request.clip_plan.clone() {
        Some(value) => value,
        None => return failed_response(None, "clip_plan is required".to_string()),
    };
    let clip_plan = match plan::decode_and_validate(raw_plan) {
        Ok(plan) => plan,
        Err(error) => return failed_response(None, error),
    };
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    let source = match request.source_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "source_path is required".to_string()),
    };
    let encoder = match request.media.encoder() {
        Ok(policy) => policy,
        Err(error) => return failed_response(None, error),
    };
    let keyframe_interval = match request.media.keyframe_interval {
        Some(value) if value > 0 => value,
        _ => {
            return failed_response(
                None,
                "ENCODER_POLICY_REQUIRED: keyframe_interval is required for encoded media"
                    .to_string(),
            )
        }
    };
    // Geometry and audio contract come from the sealed plan; the media
    // config supplies the encoder policy + audio bitrate. Fail-closed drift
    // check: the transport profile must agree with the audited plan.
    if request.media.width != Some(clip_plan.output.width as u32)
        || request.media.height != Some(clip_plan.output.height as u32)
        || request.media.fps != Some(clip_plan.output.fps as u32)
    {
        return failed_response(
            None,
            "clip_plan geometry disagrees with the resolved media profile".to_string(),
        );
    }
    let audio_bitrate = request
        .media
        .audio_bitrate
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("128k");
    let profile = VideoProfile {
        width: clip_plan.output.width as u32,
        height: clip_plan.output.height as u32,
        fps: clip_plan.output.fps as u32,
        keyframe_interval,
        audio_codec: clip_plan.audio.codec.clone(),
        audio_bitrate: audio_bitrate.to_string(),
        sample_rate: clip_plan.audio.sample_rate as u32,
        channels: clip_plan.audio.channels as u32,
    };

    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let ffprobe = probe::ffprobe_path(ffmpeg);

    // Audio copy policy: probe the source stream; copy verbatim only when it
    // already satisfies the plan contract (never re-encode compatible audio).
    let source_metadata = match probe::probe_file(&ffprobe, source) {
        Ok(metadata) => metadata,
        Err(error) => return failed_response(Some(source.to_string()), error),
    };
    let (audio_copy_eligible, audio_encode_passes, audio_args) = audio_policy(
        &clip_plan.audio,
        &source_metadata,
        audio_bitrate,
    );

    let graph = build_filter_graph(&clip_plan, &profile);
    let subtitle_raster_cpu = clip_plan
        .subtitles
        .as_ref()
        .map(|subtitles| subtitles.mode == SUBTITLE_BURN)
        .unwrap_or(false);

    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    // Inputs: [0] source, [1] background asset (mode=asset), [2] watermark.
    command.args(["-i", source]);
    if clip_plan.background.as_ref().map(|bg| bg.mode.as_str()) == Some(BACKGROUND_ASSET) {
        command.args([
            "-i",
            clip_plan
                .background
                .as_ref()
                .and_then(|bg| bg.path.as_deref())
                .unwrap_or(""),
        ]);
    }
    if clip_plan.watermark.is_some() {
        command.args([
            "-i",
            clip_plan
                .watermark
                .as_ref()
                .map(|wm| wm.path.as_str())
                .unwrap_or(""),
        ]);
    }
    command.args(["-filter_complex", &graph, "-map", "[vfinal]", "-map", "0:a?"]);
    for argument in audio_args {
        command.arg(argument);
    }
    if let Err(error) = append_video_args(&mut command, &encoder, &profile, None) {
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);

    let encode_started = std::time::Instant::now();
    let encode_result = command.output();
    let ffmpeg_ms = encode_started.elapsed().as_millis() as i64;
    match encode_result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_clip".to_string(),
                source_path: Some(source.to_string()),
                items: Vec::new(),
                metadata: Some(MediaMetadata {
                    duration_sec: source_metadata.duration_sec,
                    bitrate: None,
                    width: profile.width,
                    height: profile.height,
                    fps: profile.fps as f64,
                    video_codec: None,
                    pixel_format: None,
                    format_name: None,
                    stream_count: 0,
                    video_stream_count: 0,
                    audio_stream_count: 0,
                    fps_num: 0,
                    fps_den: 0,
                    audio_codec: None,
                    audio_profile: None,
                    sample_rate: None,
                    channels: None,
                    start_pts: None,
                    has_video: true,
                    has_audio: source_metadata.has_audio,
                    mix_ms: None,
                    aac_encode_ms: None,
                    probe_ms: None,
                    hash_ms: None,
                    ffmpeg_ms: Some(ffmpeg_ms.max(1)),
                    final_audio_sha256: None,
                    audio_copy_eligible: Some(audio_copy_eligible),
                    audio_encode_passes: Some(audio_encode_passes),
                    subtitle_raster_cpu: Some(subtitle_raster_cpu),
                }),
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "clip render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("clip render failed to start: {error}"))
        }
    }
}

/// audio_policy decides copy vs exactly-one certified conversion and returns
/// the ffmpeg audio arguments plus the recorded outcome. copy_if_compatible
/// copies verbatim only when the source stream already satisfies the plan
/// contract; transcode always runs one conversion. A source without an audio
/// stream yields no stream (mapped 0:a?) and records the ineligible outcome.
fn audio_policy(
    audio: &ClipPlanAudio,
    source: &MediaMetadata,
    bitrate: &str,
) -> (bool, i32, Vec<String>) {
    let compatible = source.has_audio
        && source.audio_codec.as_deref() == Some(audio.codec.as_str())
        && source.sample_rate == Some(audio.sample_rate as u32)
        && source.channels == Some(audio.channels as u32);
    if audio.mode == AUDIO_COPY_IF_COMPATIBLE && compatible {
        return (true, 0, vec!["-c:a".to_string(), "copy".to_string()]);
    }
    (
        false,
        i32::from(source.has_audio),
        vec![
            "-c:a".to_string(),
            audio.codec.clone(),
            "-ar".to_string(),
            audio.sample_rate.to_string(),
            "-ac".to_string(),
            audio.channels.to_string(),
            "-b:a".to_string(),
            bitrate.to_string(),
        ],
    )
}

/// build_filter_graph composes the single-pass filter chain:
/// background (none | blur_source | asset) + fitted foreground → overlay →
/// watermark (position/opacity/margin) → libass burn (mode=burn only).
/// Input indices are positional: [0] source, [1] background asset (only when
/// mode=asset), [2] watermark (next free index).
fn build_filter_graph(plan: &ClipRenderPlan, profile: &VideoProfile) -> String {
    let w = profile.width;
    let h = profile.height;
    let fps = profile.fps;
    let mut graph = String::new();

    let bg_mode = plan
        .background
        .as_ref()
        .map(|bg| bg.mode.as_str())
        .unwrap_or(BACKGROUND_NONE);
    match bg_mode {
        BACKGROUND_NONE => {}
        BACKGROUND_ASSET => {
            // Cover-crop the background asset to the full output frame. The
            // fps filter must match the foreground so the overlay's main
            // (background) input drives the composite at the contract rate.
            graph.push_str(&format!(
                "[1:v]scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1,fps={fps}[bg];"
            ));
        }
        _ => {
            // blur_source: cover-crop the source itself and blur it. The
            // sharp foreground overlays it below — one decode, one encode.
            graph.push_str(&format!(
                "[0:v]scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},gblur=sigma=25,setsar=1,fps={fps}[bg];"
            ));
        }
    }

    // Fitted foreground, centered on the background canvas.
    graph.push_str(&format!(
        "[0:v]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,fps={fps},setsar=1[fg];"
    ));

    let mut final_label: String;
    match bg_mode {
        BACKGROUND_NONE => final_label = "[fg]".to_string(),
        _ => {
            graph.push_str("[bg][fg]overlay=0:0:format=auto[v0];");
            final_label = "[v0]".to_string();
        }
    }

    if let Some(watermark) = &plan.watermark {
        let input_index = if bg_mode == BACKGROUND_ASSET { 2 } else { 1 };
        let (x, y) = watermark_position(watermark, w, h);
        graph.push_str(&format!(
            "[{input_index}:v]format=rgba,colorchannelmixer=aa={}[wm];",
            watermark.opacity
        ));
        graph.push_str(&format!("{final_label}[wm]overlay={x}:{y}:format=auto[v1];"));
        final_label = "[v1]".to_string();
    }

    if let Some(subtitles) = &plan.subtitles {
        if subtitles.mode == SUBTITLE_BURN {
            let escaped = escape_filter_path(&subtitles.path);
            graph.push_str(&format!("{final_label}subtitles=filename='{escaped}'[vfinal]"));
            return graph;
        }
    }
    graph.push_str(&format!("{final_label}null[vfinal]"));
    graph
}

/// watermark_position resolves the overlay x/y expressions for the requested
/// corner with the margin in output pixels.
fn watermark_position(watermark: &ClipPlanWatermark, _width: u32, _height: u32) -> (String, String) {
    let margin = watermark.margin_px;
    match watermark.position.as_str() {
        "top_left" => (format!("x={margin}"), format!("y={margin}")),
        "top_right" => (
            format!("x=main_w-overlay_w-{margin}"),
            format!("y={margin}"),
        ),
        "bottom_left" => (
            format!("x={margin}"),
            format!("y=main_h-overlay_h-{margin}"),
        ),
        _ => (
            format!("x=main_w-overlay_w-{margin}"),
            format!("y=main_h-overlay_h-{margin}"),
        ),
    }
}

/// escape_filter_path makes a filesystem path safe inside an ffmpeg filter
/// argument: backslashes and colons are backslash-escaped, single quotes are
/// closed/reopened, and the result is single-quoted.
fn escape_filter_path(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len() + 8);
    for character in path.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            ':' => escaped.push_str("\\:"),
            '\'' => escaped.push_str("'\\''"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VideoProfile;
    use crate::protocol::MediaMetadata;
    use crate::render_clip::plan::{
        ClipPlanAudio, ClipPlanBackground, ClipPlanSource, ClipPlanSubtitles, ClipPlanWatermark,
        ClipRenderPlan, ClipPlanOutput,
    };
    use std::fs;

    fn plan() -> ClipRenderPlan {
        ClipRenderPlan {
            version: "clip-render-plan.v1".to_string(),
            run_id: "job-1".to_string(),
            source: ClipPlanSource {
                asset_id: "asset-src".to_string(),
                path: "/tmp/source.mp4".to_string(),
                sha256: "a".repeat(64),
            },
            background: Some(ClipPlanBackground {
                mode: "blur_source".to_string(),
                asset_id: None,
                path: None,
                sha256: None,
            }),
            watermark: None,
            subtitles: None,
            output: ClipPlanOutput {
                contract_id: "velox-editing-clip-v1".to_string(),
                container: "mp4".to_string(),
                video_codec: "h264".to_string(),
                video_profile: Some("high".to_string()),
                pixel_format: "yuv420p".to_string(),
                width: 1080,
                height: 1920,
                fps: 60,
            },
            audio: ClipPlanAudio {
                mode: "copy_if_compatible".to_string(),
                codec: "aac".to_string(),
                sample_rate: 48000,
                channels: 2,
            },
            output_path: "/tmp/out.mp4".to_string(),
            plan_sha256: "a".repeat(64),
        }
    }

    fn profile() -> VideoProfile {
        VideoProfile {
            width: 1080,
            height: 1920,
            fps: 60,
            keyframe_interval: 120,
            audio_codec: "aac".to_string(),
            audio_bitrate: "128k".to_string(),
            sample_rate: 48000,
            channels: 2,
        }
    }

    fn source_metadata(has_audio: bool) -> MediaMetadata {
        MediaMetadata {
            duration_sec: 30.0,
            bitrate: None,
            width: 1920,
            height: 1080,
            fps: 30.0,
            video_codec: Some("h264".to_string()),
            pixel_format: Some("yuv420p".to_string()),
            format_name: None,
            stream_count: 1,
            video_stream_count: 1,
            audio_stream_count: u32::from(has_audio),
            fps_num: 30,
            fps_den: 1,
            audio_codec: if has_audio {
                Some("aac".to_string())
            } else {
                None
            },
            audio_profile: None,
            sample_rate: if has_audio { Some(48000) } else { None },
            channels: if has_audio { Some(2) } else { None },
            start_pts: None,
            has_video: true,
            has_audio,
            mix_ms: None,
            aac_encode_ms: None,
            probe_ms: None,
            hash_ms: None,
            ffmpeg_ms: None,
            final_audio_sha256: None,
            audio_copy_eligible: None,
            audio_encode_passes: None,
            subtitle_raster_cpu: None,
        }
    }

    #[test]
    fn blur_source_graph_is_single_pass_with_cpu_subtitle_stage() {
        let mut p = plan();
        let wm_path = std::env::temp_dir().join("cliprender-wm.png");
        fs::write(&wm_path, b"wm").unwrap();
        let sub_path = std::env::temp_dir().join("cliprender-sub.ass");
        fs::write(&sub_path, b"[Script Info]").unwrap();
        p.watermark = Some(ClipPlanWatermark {
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
        assert!(graph.contains("gblur=sigma=25"), "graph: {graph}");
        assert!(graph.contains("pad=1080:1920:(ow-iw)/2:(oh-ih)/2"));
        assert!(graph.contains("fps=60"));
        assert!(graph.contains("overlay=0:0:format=auto"));
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
        assert!(graph.starts_with("[0:v]scale="));
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
        assert_eq!(
            (x.as_str(), y.as_str()),
            ("x=main_w-overlay_w-24", "y=24")
        );
        let (x, y) = watermark_position(&wm("bottom_left"), 1080, 1920);
        assert_eq!(
            (x.as_str(), y.as_str()),
            ("x=24", "y=main_h-overlay_h-24")
        );
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
}
