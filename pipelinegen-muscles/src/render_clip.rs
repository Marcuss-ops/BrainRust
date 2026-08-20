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
use crate::encoder::{append_video_args, append_video_args_cuda};
use crate::native;
use crate::probe;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
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
    let fps_num = clip_plan.output.fps_num;
    let fps_den = clip_plan.output.fps_den;
    // Geometry and audio contract come from the sealed plan; the media
    // config supplies the encoder policy + audio bitrate. Fail-closed drift
    // check: the transport profile must agree with the audited plan. The
    // framerate is a rational pair so NTSC rates (30000/1001) survive the
    // boundary losslessly instead of being rounded to a scalar integer.
    if request.media.width != Some(clip_plan.output.width as u32)
        || request.media.height != Some(clip_plan.output.height as u32)
        || request.media.fps_num != Some(fps_num as u32)
        || request.media.fps_den != Some(fps_den as u32)
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
        // Scalar fps is a nominal projection for encoder validation only;
        // the filter graph and response metadata use the exact rational pair.
        fps: (fps_num as f64 / fps_den as f64).round() as u32,
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
    let (audio_copy_eligible, audio_encode_passes, audio_args) =
        audio_policy(&clip_plan.audio, &source_metadata, audio_bitrate);

    // The render backend is resolved by the Go RenderBackendResolver and
    // transported here — Rust never derives hardware usage from the codec
    // string. Only the CUDA native backend enables NVDEC decode; the
    // software FFmpeg fallback decodes on the CPU regardless of encoder.
    let backend = request
        .render_backend
        .as_deref()
        .unwrap_or("ffmpeg_fallback");
    let use_hw_decode = backend == "cuda_native";
    // A CUDA frame must not pass through a CPU-only filter. This strict path
    // is intentionally limited to a source-only clip: subtitles, alpha
    // watermarking, backgrounds and frame-rate conversion still use the
    // correctness fallback below until their GPU implementations are wired.
    let requested_codec = encoder.codec.trim().to_ascii_lowercase();
    let nvenc_encoder = requested_codec == "nvenc" || requested_codec.ends_with("_nvenc");
    let gpu_native = use_hw_decode && nvenc_encoder && gpu_native_eligible(&clip_plan);
    let graph = build_filter_graph_with_hw_decode(&clip_plan, &profile, use_hw_decode, gpu_native);
    let subtitle_raster_cpu = clip_plan
        .subtitles
        .as_ref()
        .map(|subtitles| subtitles.mode == SUBTITLE_BURN)
        .unwrap_or(false);

    let part = part_path(output);

    // Native libavcodec/libavformat path. It is deliberately narrower than
    // the CUDA filter path: only an already-sized source-only clip with
    // compatible copied audio can use it. This reuses the native decoder and
    // encoder setup inside the bridge and avoids spawning FFmpeg for this
    // class of render. Any unsupported media shape falls through to the
    // audited FFmpeg graph rather than changing output semantics.
    let native_eligible = gpu_native
        && audio_copy_eligible
        && clip_plan.subtitles.is_none()
        && clip_plan.watermark.is_none()
        && clip_plan.background.is_none()
        && source_metadata.width == profile.width
        && source_metadata.height == profile.height;
    if native_eligible {
        let native_started = std::time::Instant::now();
        if native::render_source_only(source, &part, profile.width, profile.height).is_ok() {
            let native_ms = native_started.elapsed().as_millis() as i64;
            return match publish_output(&part, output) {
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
                        fps: fps_num as f64 / fps_den as f64,
                        video_codec: Some("h264_nvenc".to_string()),
                        pixel_format: Some("cuda".to_string()),
                        format_name: Some("native_libav_cuda".to_string()),
                        stream_count: 0,
                        video_stream_count: 1,
                        audio_stream_count: u32::from(source_metadata.has_audio),
                        fps_num: fps_num as u32,
                        fps_den: fps_den as u32,
                        audio_codec: source_metadata.audio_codec.clone(),
                        audio_profile: source_metadata.audio_profile.clone(),
                        sample_rate: source_metadata.sample_rate,
                        channels: source_metadata.channels,
                        start_pts: source_metadata.start_pts,
                        has_video: true,
                        has_audio: source_metadata.has_audio,
                        mix_ms: None,
                        aac_encode_ms: None,
                        probe_ms: None,
                        hash_ms: None,
                        ffmpeg_ms: Some(native_ms.max(1)),
                        final_audio_sha256: None,
                        audio_copy_eligible: Some(true),
                        audio_encode_passes: Some(0),
                        subtitle_raster_cpu: Some(false),
                        native_media: Some(true),
                        gpu_copy_bytes: Some(0),
                    }),
                    error: None,
                },
                Err(error) => failed_response(None, error),
            };
        }
        let _ = fs::remove_file(&part);
    }

    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    // Inputs: [0] source, optional background asset, and for the CUDA path a
    // transparent overlay canvas supplied by lavfi.
    if use_hw_decode {
        // Decode source frames through NVDEC. CPU filters use the explicit
        // hwdownload fallback; the source-only path keeps the CUDA frames
        // device-local through scale_cuda and NVENC.
        command.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
    }
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
    if clip_plan
        .watermark
        .as_ref()
        .map(|wm| wm.text.trim().is_empty())
        .unwrap_or(false)
    {
        command.args([
            "-i",
            clip_plan
                .watermark
                .as_ref()
                .map(|wm| wm.path.as_str())
                .unwrap_or(""),
        ]);
    }
    if gpu_native && (clip_plan.watermark.is_some() || clip_plan.subtitles.is_some()) {
        command.args([
            "-f", "lavfi", "-i",
            &format!("color=c=black@0.0:s={}x{}:r={}/{}", profile.width, profile.height, fps_num, fps_den),
        ]);
    }
    command.args([
        "-filter_complex",
        &graph,
        "-map",
        "[vfinal]",
        "-map",
        "0:a?",
    ]);
    for argument in audio_args {
        command.arg(argument);
    }
    let encoder_result = if gpu_native {
        append_video_args_cuda(&mut command, &encoder, &profile, None)
    } else {
        append_video_args(&mut command, &encoder, &profile, None)
    };
    if let Err(error) = encoder_result {
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
                    fps: fps_num as f64 / fps_den as f64,
                    video_codec: None,
                    pixel_format: None,
                    format_name: None,
                    stream_count: 0,
                    video_stream_count: 0,
                    audio_stream_count: 0,
                    fps_num: fps_num as u32,
                    fps_den: fps_den as u32,
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
                    native_media: Some(false),
                    gpu_copy_bytes: None,
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
#[cfg(test)]
fn build_filter_graph(plan: &ClipRenderPlan, profile: &VideoProfile) -> String {
    build_filter_graph_with_hw_decode(plan, profile, false, false)
}

fn build_filter_graph_with_hw_decode(
    plan: &ClipRenderPlan,
    profile: &VideoProfile,
    hw_decode: bool,
    gpu_native: bool,
) -> String {
    let w = profile.width;
    let h = profile.height;
    // Rational framerate pair from the sealed plan: NTSC rates (30000/1001)
    // are passed to the fps filter losslessly instead of as a rounded int.
    let fps = format!("{}/{}", plan.output.fps_num, plan.output.fps_den);
    let scale = plan.output.foreground_scale_percent.clamp(1, 100) as u64;
    let fg_w = (w as u64 * scale / 100).max(2) as u32;
    let fg_h = (h as u64 * scale / 100).max(2) as u32;
    let mut graph = String::new();

    if gpu_native {
        if plan.watermark.is_none() && plan.subtitles.is_none() {
            graph.push_str(&format!("[0:v]scale_cuda={w}:{h}[vfinal]"));
            return graph;
        }
        graph.push_str(&format!("[0:v]scale_cuda={w}:{h}[base];"));
        if plan.watermark.is_some() || plan.subtitles.is_some() {
            // Rasterize only the transparent overlay layer on CPU, then
            // upload that small layer and composite it over the CUDA video.
            graph.push_str("[1:v]format=rgba");
            if let Some(watermark) = &plan.watermark {
                let (x, y) = watermark_text_position(watermark);
                let text = escape_filter_text(&watermark.text);
                graph.push_str(&format!(
                    ",drawtext=text='{text}':fontcolor=white@{}:fontsize=48:borderw=2:bordercolor=black@{}:{x}:{y}",
                    watermark.opacity, watermark.opacity
                ));
            }
            if let Some(subtitles) = &plan.subtitles {
                if subtitles.mode == SUBTITLE_BURN {
                    let escaped = escape_filter_path(&subtitles.path);
                    graph.push_str(&format!(",subtitles=filename='{escaped}'"));
                }
            }
            graph.push_str(",hwupload_cuda[overlay];[base][overlay]overlay_cuda=x=0:y=0:format=nv12[vfinal]");
        }
        return graph;
    }

    let bg_mode = plan
        .background
        .as_ref()
        .map(|bg| bg.mode.as_str())
        .unwrap_or(BACKGROUND_NONE);
    let blur_source = bg_mode != BACKGROUND_NONE && bg_mode != BACKGROUND_ASSET;
    // Conform the source rate once before branching the blur and foreground
    // paths. This avoids running an independent fps filter on both branches.
    let source_prefix = if hw_decode {
        "[0:v]hwdownload,format=nv12,".to_string()
    } else {
        "[0:v]".to_string()
    };
    if blur_source {
        graph.push_str(&format!(
            "{source_prefix}fps={fps},split=2[src_bg][src_fg];"
        ));
    } else {
        graph.push_str(&format!("{source_prefix}fps={fps}[src_fg];"));
    }
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
            // blur_source: build the blurred plate at a small fixed size,
            // then upscale it. The foreground remains sharp and overlays it
            // below. Keeping the expensive blur at 180x320 avoids doing a
            // full-resolution CPU blur for every output frame.
            graph.push_str(&format!(
                "[src_bg]scale=180:320:force_original_aspect_ratio=increase,crop=180:320,gblur=sigma=5,scale={w}:{h}:flags=bilinear,setsar=1[bg];"
            ));
        }
    }

    // Fitted foreground, centered on the background canvas.
    graph.push_str(&format!(
        "[src_fg]scale={fg_w}:{fg_h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[fg];"
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
        if watermark.text.trim().is_empty() {
            let input_index = if bg_mode == BACKGROUND_ASSET { 2 } else { 1 };
            let (x, y) = watermark_position(watermark, w, h);
            graph.push_str(&format!(
                "[{input_index}:v]format=rgba,colorchannelmixer=aa={}[wm];",
                watermark.opacity
            ));
            graph.push_str(&format!(
                "{final_label}[wm]overlay={x}:{y}:format=auto[v1];"
            ));
        } else {
            let (x, y) = watermark_text_position(watermark);
            let text = escape_filter_text(&watermark.text);
            graph.push_str(&format!(
                "{final_label}drawtext=text='{text}':fontcolor=white@{}:fontsize=48:borderw=2:bordercolor=black@{}:{x}:{y}[v1];",
                watermark.opacity, watermark.opacity
            ));
        }
        final_label = "[v1]".to_string();
    }

    if let Some(subtitles) = &plan.subtitles {
        if subtitles.mode == SUBTITLE_BURN {
            let escaped = escape_filter_path(&subtitles.path);
            graph.push_str(&format!(
                "{final_label}subtitles=filename='{escaped}'[vfinal]"
            ));
            return graph;
        }
    }
    graph.push_str(&format!("{final_label}null[vfinal]"));
    graph
}

/// Returns true only for the graph that can remain entirely in CUDA memory.
/// The conservative boundary is deliberate: an accidental CPU filter here
/// would silently trigger a full-frame PCIe round trip.
fn gpu_native_eligible(plan: &ClipRenderPlan) -> bool {
    plan.background.is_none()
        && plan.watermark.as_ref().map(|wm| !wm.text.trim().is_empty()).unwrap_or(true)
        && plan.output.foreground_scale_percent == 100
}

/// watermark_position resolves the overlay x/y expressions for the requested
/// corner with the margin in output pixels.
fn watermark_position(
    watermark: &ClipPlanWatermark,
    _width: u32,
    _height: u32,
) -> (String, String) {
    let margin = watermark.margin_px;
    match watermark.position.as_str() {
        "top_left" => (format!("x={margin}"), format!("y={margin}")),
        "top_right" => (
            format!("x=main_w-overlay_w-{margin}"),
            format!("y={margin}"),
        ),
        "center" => (
            "x=(main_w-overlay_w)/2".to_string(),
            "y=(main_h-overlay_h)/2".to_string(),
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

fn watermark_text_position(watermark: &ClipPlanWatermark) -> (String, String) {
    let margin = watermark.margin_px;
    match watermark.position.as_str() {
        "top_left" => (format!("x={margin}"), format!("y={margin}")),
        "top_right" => (format!("x=w-text_w-{margin}"), format!("y={margin}")),
        "bottom_left" => (format!("x={margin}"), format!("y=h-text_h-{margin}")),
        "bottom_right" => (format!("x=w-text_w-{margin}"), format!("y=h-text_h-{margin}")),
        _ => ("x=(w-text_w)/2".to_string(), "y=(h-text_h)/2".to_string()),
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

fn escape_filter_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 8);
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            ':' | ',' | '\'' => {
                escaped.push('\\');
                escaped.push(character);
            }
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
        ClipPlanAudio, ClipPlanBackground, ClipPlanOutput, ClipPlanSource, ClipPlanSubtitles,
        ClipPlanWatermark, ClipRenderPlan,
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
                fps_num: 60,
                fps_den: 1,
                foreground_scale_percent: 100,
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
            native_media: None,
            gpu_copy_bytes: None,
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
        assert!(
            graph.contains("scale=1080:1920:flags=bilinear"),
            "graph: {graph}"
        );
        assert!(graph.contains("pad=1080:1920:(ow-iw)/2:(oh-ih)/2"));
        assert!(graph.contains("fps=60/1"));
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
        assert!(graph.starts_with("[0:v]fps="));
    }

    #[test]
    fn source_only_cuda_graph_has_no_cpu_boundary() {
        let mut p = plan();
        p.background = None;
        assert!(gpu_native_eligible(&p));
        let graph = build_filter_graph_with_hw_decode(&p, &profile(), true, true);
        assert_eq!(graph, "[0:v]scale_cuda=1080:1920[vfinal]");
        assert!(!graph.contains("hwdownload"));
        assert!(!graph.contains("fps="));
    }

    #[test]
    fn cuda_graph_rejects_alpha_and_burn_filters() {
        let mut p = plan();
        p.background = None;
        p.watermark = Some(ClipPlanWatermark {
			text: String::new(),
			asset_id: "wm-1".to_string(),
            path: "/tmp/wm.png".to_string(),
            sha256: "a".repeat(64),
            position: "top_left".to_string(),
            opacity: 0.8,
            margin_px: 10,
        });
        assert!(!gpu_native_eligible(&p));
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
}
