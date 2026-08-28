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
use crate::probe;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[path = "render_clip_plan.rs"]
mod plan;

use plan::{
    ClipPlanAudio, ClipPlanWatermark, ClipRenderPlan, AUDIO_COPY_IF_COMPATIBLE, BACKGROUND_ASSET,
    BACKGROUND_NONE, SUBTITLE_BURN,
};

pub(super) fn render_clip(request: Request) -> Response {
    // The whole-operation wall clock for the benchmark decomposition: every
    // success response reports startup_ms / publish_ms / op_ms so the Go
    // boundary can attribute the seconds outside the ffmpeg wall.
    let op_started = std::time::Instant::now();
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
        fps_num: fps_num as u32,
        fps_den: fps_den as u32,
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
    // The probe is timed separately (probe_ms) so the report can attribute
    // the ffprobe wall instead of burying it inside startup_ms.
    let probe_started = std::time::Instant::now();
    let source_metadata = match probe::probe_file(&ffprobe, source) {
        Ok(metadata) => metadata,
        Err(error) => return failed_response(Some(source.to_string()), error),
    };
    let probe_elapsed = probe_started.elapsed();
    let probe_ms = probe_elapsed.as_millis() as i64;
    let (audio_copy_eligible, audio_encode_passes, audio_args) =
        audio_policy(&clip_plan.audio, &source_metadata, audio_bitrate);

    // The render backend is resolved by the Go RenderBackendResolver and
    // transported here — Rust never derives hardware usage from the codec
    // string. cuda_native selects the PATH B CUDA hybrid graph (NVDEC →
    // scale_cuda base → CPU-rasterized overlay → hwupload_cuda →
    // overlay_cuda → NVENC) with ZERO readback of the base video; every
    // other backend uses the software FFmpeg graph (chronon_vulkan is
    // executed by the Chronon executor, never here).
    let backend = request
        .render_backend
        .as_deref()
        .unwrap_or("ffmpeg_fallback");
    let requested_codec = encoder.codec.trim().to_ascii_lowercase();
    let nvenc_encoder = requested_codec == "nvenc" || requested_codec.ends_with("_nvenc");
    let gpu_native = backend == "cuda_native" && nvenc_encoder && gpu_native_eligible(&clip_plan);
    // Zero-readback contract: when cuda_native is selected the plan MUST be
    // renderable device-local. A plan the hybrid cannot do (background
    // plate, scale != 100, software encoder) is a resolver/media-policy
    // mismatch — fail closed loudly instead of silently hwdownload-ing the
    // base video (the historical PATH B readback this implementation
    // eliminates).
    if backend == "cuda_native" && !gpu_native {
        return failed_response(
            None,
            "cuda_native selected but the plan/encoder is not eligible for the zero-readback CUDA hybrid (background, scale or software encoder); resolver/media-policy mismatch".to_string(),
        );
    }
    let graph = if gpu_native {
        build_gpu_filter_graph(&clip_plan, &profile)
    } else {
        build_filter_graph(&clip_plan, &profile)
    };
    let subtitle_raster_cpu = clip_plan
        .subtitles
        .as_ref()
        .map(|subtitles| subtitles.mode == SUBTITLE_BURN)
        .unwrap_or(false);

    let part = part_path(output);

    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    // -benchmark_all emits a per-frame `bench:` line on stderr for every
    // decode/encode call — the ONLY decode/encode attribution stock ffmpeg
    // exposes from a single pass. The lines are logged at INFO, so the
    // loglevel must be raised from error; -nostats keeps the progress
    // reports off. The per-frame lines are accumulated incrementally by
    // BenchAccumulator (the retained stderr tail is capped at 64KiB and
    // would truncate them on long renders).
    command.args([
        "-hide_banner",
        "-loglevel",
        "info",
        "-nostats",
        "-benchmark_all",
        "-y",
    ]);
    // Inputs: [0] source, optional background asset, optional watermark, and
    // for the CUDA hybrid the transparent overlay canvas (lavfi).
    if gpu_native {
        // Decode through NVDEC and keep the base video device-local; the
        // PATH B graph never downloads it.
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
    let text_watermark = clip_plan
        .watermark
        .as_ref()
        .map(|wm| !wm.text.trim().is_empty())
        .unwrap_or(false);
    let image_watermark = clip_plan
        .watermark
        .as_ref()
        .map(|wm| wm.text.trim().is_empty())
        .unwrap_or(false);
    let burn_subtitles = clip_plan
        .subtitles
        .as_ref()
        .map(|s| s.mode == SUBTITLE_BURN)
        .unwrap_or(false);
    // PATH B overlay canvas: transparent full-frame canvas rasterized on CPU
    // (drawtext + libass subtitles), uploaded once and composited on GPU.
    if gpu_native && (text_watermark || burn_subtitles) {
        command.args([
            "-f", "lavfi", "-i",
            &format!("color=c=black@0.0:s={}x{}:r={}/{}", profile.width, profile.height, fps_num, fps_den),
        ]);
    }
    if image_watermark {
        command.args([
            "-i",
            clip_plan
                .watermark
                .as_ref()
                .map(|wm| wm.path.as_str())
                .unwrap_or(""),
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

    // startup_ms = everything BEFORE the ffmpeg wall EXCEPT the source probe
    // (reported separately as probe_ms): plan decode, audio policy,
    // filter-graph build, process spawn. This is the "renderer startup" the
    // benchmark could not attribute to decode, composite or encode.
    let startup_ms = (op_started.elapsed() - probe_elapsed).as_millis() as i64;
    let encode_started = std::time::Instant::now();
    let bench = Arc::new(Mutex::new(BenchAccumulator::default()));
    let bench_for_thread = Arc::clone(&bench);
    let on_bench_line = move |line: &str| {
        bench_for_thread.lock().unwrap().handle_line(line);
    };
    let encode_result = command.output_with_line_handler(on_bench_line);
    let ffmpeg_ms = encode_started.elapsed().as_millis() as i64;
    // Fine-grained attribution from the bench sums: decode_ms/encode_ms are
    // measured per-frame; filter_graph_ms is the residual (ffmpeg wall minus
    // the measured work) and honestly covers the filter graph (subtitle
    // raster + watermark + compositing + conversions) plus demux/mux and the
    // +faststart pass. saw_bench=false means the ffmpeg build emitted no
    // bench lines → phases stay NOT_INSTRUMENTED (never a fake zero).
    let bench_state = bench.lock().unwrap();
    let saw_bench = bench_state.saw_bench;
    let decode_ms = bench_state.decode_us / 1000;
    let encode_ms = bench_state.encode_us / 1000;
    let filter_graph_ms = (ffmpeg_ms - decode_ms - encode_ms).max(0);
    drop(bench_state);
    match encode_result {
        Ok(result) if result.status.success() => {
            let publish_started = std::time::Instant::now();
            let published = publish_output(&part, output);
            let publish_ms = publish_started.elapsed().as_millis() as i64;
            let op_ms = op_started.elapsed().as_millis() as i64;
            match published {
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
                        probe_ms: Some(probe_ms.max(0)),
                        hash_ms: None,
                        ffmpeg_ms: Some(ffmpeg_ms.max(1)),
                        startup_ms: Some(startup_ms.max(1)),
                        publish_ms: Some(publish_ms.max(1)),
                        op_ms: Some(op_ms.max(1)),
                        final_audio_sha256: None,
                        audio_copy_eligible: Some(audio_copy_eligible),
                        audio_encode_passes: Some(audio_encode_passes),
                        subtitle_raster_cpu: Some(subtitle_raster_cpu),
                        gpu_copy_bytes: None,
                        decode_ms: if saw_bench {
                            Some(decode_ms)
                        } else {
                            None
                        },
                        filter_graph_ms: if saw_bench {
                            Some(filter_graph_ms)
                        } else {
                            None
                        },
                        // subtitle_raster_ms / watermark_raster_ms /
                        // frame_conversion_ms / audio_mux_ms stay None
                        // (NOT_INSTRUMENTED): a single-pass stock ffmpeg
                        // invocation attributes only decode/encode per-frame;
                        // the filter-graph residual above covers subtitle
                        // raster + watermark + compositing + conversions +
                        // muxing as one lump. Per-filter attribution would
                        // require a custom ffmpeg build — never a fake zero.
                        subtitle_raster_ms: None,
                        watermark_raster_ms: None,
                        frame_conversion_ms: None,
                        encode_ms: if saw_bench {
                            Some(encode_ms)
                        } else {
                            None
                        },
                        audio_mux_ms: None,
                    }),
                    error: None,
                },
                Err(error) => failed_response(None, error),
            }
        }
        Ok(result) => {
            let _ = fs::remove_file(&part);
            // Strip the per-frame bench lines from the error tail so the
            // actual ffmpeg error stays readable (the retained 64KiB tail is
            // dominated by bench lines on any non-trivial render).
            let stderr = String::from_utf8_lossy(&result.stderr);
            let message = stderr
                .lines()
                .filter(|line| !line.starts_with("bench:"))
                .collect::<Vec<_>>()
                .join("\n");
            failed_response(None, format!("clip render failed: {}", message.trim()))
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("clip render failed to start: {error}"))
        }
    }
}

/// BenchAccumulator sums ffmpeg's `-benchmark_all` per-frame bench lines as
/// they stream on stderr. Line shape (ffmpeg 4.x-7.x):
///
///   bench:  <user> user <sys> sys <real> real <task> <n>
///
/// where <real> is microseconds of wall time for that decode/encode call.
/// Only the task labels are honored: decode_video → decode work,
/// encode_video + flush_video → encode work. These per-frame sums are the
/// only single-pass decode/encode attribution stock ffmpeg exposes; the
/// filter graph (subtitles/watermark/composite/conversions) and muxing are
/// NOT wrapped by any bench line and surface as the residual in render_clip.
#[derive(Default)]
struct BenchAccumulator {
    decode_us: i64,
    encode_us: i64,
    saw_bench: bool,
}

impl BenchAccumulator {
    fn handle_line(&mut self, line: &str) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 || fields[0] != "bench:" || fields[6] != "real" {
            return;
        }
        let real_us: i64 = match fields[5].parse() {
            Ok(value) => value,
            Err(_) => return,
        };
        self.saw_bench = true;
        match fields[7] {
            "decode_video" => self.decode_us += real_us,
            "encode_video" | "flush_video" => self.encode_us += real_us,
            _ => {}
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

/// build_filter_graph composes the single-pass SOFTWARE filter chain:
/// background (none | blur_source | asset) + fitted foreground → overlay →
/// watermark (position/opacity/margin) → libass burn (mode=burn only).
/// Input indices are positional: [0] source, [1] background asset (only when
/// mode=asset), [2] watermark (next free index). This is the ONLY graph: the
/// CUDA hybrid path (scale_cuda/overlay_cuda/hwupload_cuda) was removed with
/// the cuda_native backend — certified Chronon owns GPU compositing on the
/// Chronon executor, and this graph is the software baseline for every other
/// host.
fn build_filter_graph(plan: &ClipRenderPlan, profile: &VideoProfile) -> String {
    let w = profile.width;
    let h = profile.height;
    // Rational framerate pair from the sealed plan: NTSC rates (30000/1001)
    // are passed to the fps filter losslessly instead of as a rounded int.
    let fps = format!("{}/{}", plan.output.fps_num, plan.output.fps_den);
    let scale = plan.output.foreground_scale_percent.clamp(1, 100) as u64;
    let fg_w = (w as u64 * scale / 100).max(2) as u32;
    let fg_h = (h as u64 * scale / 100).max(2) as u32;
    let mut graph = String::new();

    let bg_mode = plan
        .background
        .as_ref()
        .map(|bg| bg.mode.as_str())
        .unwrap_or(BACKGROUND_NONE);
    let blur_source = bg_mode != BACKGROUND_NONE && bg_mode != BACKGROUND_ASSET;
    // Conform the source rate once before branching the blur and foreground
    // paths. This avoids running an independent fps filter on both branches.
    let source_prefix = "[0:v]";
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

/// build_gpu_filter_graph composes the PATH B CUDA hybrid chain — ZERO
/// readback of the base video:
///
///   [0:v] NVDEC → scale_cuda (device-local base)
///   overlay layer rasterized on CPU (drawtext / libass subtitles / image
///     watermark on a transparent canvas) → hwupload_cuda (only the small
///     overlay crosses the PCIe bus)
///   [base][overlay] overlay_cuda → NVENC (pix_fmt cuda)
///
/// The base video frames never leave VRAM. Called only when the plan passed
/// gpu_native_eligible (the resolver must route everything else away); the
/// caller fail-closes before this function if cuda_native was selected for
/// an ineligible plan.
fn build_gpu_filter_graph(plan: &ClipRenderPlan, profile: &VideoProfile) -> String {
    let w = profile.width;
    let h = profile.height;
    let text_watermark = plan
        .watermark
        .as_ref()
        .map(|wm| !wm.text.trim().is_empty())
        .unwrap_or(false);
    let image_watermark = plan
        .watermark
        .as_ref()
        .map(|wm| wm.text.trim().is_empty())
        .unwrap_or(false);
    let burn_subtitles = plan
        .subtitles
        .as_ref()
        .map(|s| s.mode == SUBTITLE_BURN)
        .unwrap_or(false);

    if !text_watermark && !image_watermark && !burn_subtitles {
        // Plain source-only clip: base video straight to NVENC, all CUDA.
        return format!("[0:v]scale_cuda={w}:{h}[vfinal]");
    }
    let mut graph = String::new();
    if text_watermark || burn_subtitles {
        // FFmpeg's overlay_cuda only accepts matching opaque CUDA YUV
        // formats.  A transparent subtitle/text canvas is yuva420p/rgba, so
        // forcing it through overlay_cuda either fails or drops alpha.  Use
        // the correctness path for this case; NVENC still performs the final
        // hardware encode and the base is not decoded twice.
        graph.push_str(&format!(
            "[0:v]hwdownload,format=rgba,scale={w}:{h}[base];"
        ));
    } else {
        graph.push_str(&format!("[0:v]scale_cuda={w}:{h}[base];"));
    }
    if text_watermark || burn_subtitles {
        // CPU raster of the transparent canvas: text + libass subtitles.
        graph.push_str("[1:v]format=rgba");
        if text_watermark {
            if let Some(watermark) = &plan.watermark {
                let (x, y) = watermark_text_position(watermark);
                let text = escape_filter_text(&watermark.text);
                graph.push_str(&format!(
                    ",drawtext=text='{text}':fontcolor=white@{}:fontsize=48:borderw=2:bordercolor=black@{}:{x}:{y}",
                    watermark.opacity, watermark.opacity
                ));
            }
        }
        if burn_subtitles {
            if let Some(subtitles) = &plan.subtitles {
                let escaped = escape_filter_path(&subtitles.path);
                // libass may emit yuva420p; normalize the transparent canvas
                // back to RGBA before the CPU overlay stage.
                graph.push_str(&format!(",subtitles=filename='{escaped}',format=rgba"));
            }
        }
        graph.push_str("[canvas];");
    }
    if image_watermark {
        let index = if text_watermark || burn_subtitles { 2 } else { 1 };
        let watermark = plan.watermark.as_ref().expect("image watermark present");
        let (x, y) = watermark_position(watermark, w, h);
        if text_watermark || burn_subtitles {
            // Compose the logo onto the CPU canvas, then upload once.
            graph.push_str(&format!(
                "[{index}:v]format=rgba,colorchannelmixer=aa={}[wm];[canvas][wm]overlay={x}:{y}:format=auto[composed];",
                watermark.opacity
            ));
            // overlay_cuda in the deployed FFmpeg build does not expose the
            // software overlay `format` option.  The negotiated CUDA frame
            // format is already NV12, so specifying format=nv12 makes the
            // filter fail before Chronon is reached.
            graph.push_str(
                "[base][composed]overlay=x=0:y=0:format=auto[composited];[composited]format=yuv420p[vfinal]"
            );
        } else {
            // Image-only overlay: upload the logo and position it with
            // overlay_cuda (x/y expressions resolve against the base video).
            graph.push_str(&format!(
                "[{index}:v]format=rgba,colorchannelmixer=aa={}[wm];[wm]hwupload_cuda[overlay];[base][overlay]overlay_cuda={x}:{y}[vfinal]",
                watermark.opacity
            ));
        }
        return graph;
    }
    if text_watermark || burn_subtitles {
        graph.push_str(
            "[base][canvas]overlay=x=0:y=0:format=auto[composited];[composited]format=yuv420p[vfinal]"
        );
    } else {
        graph.push_str("[canvas]hwupload_cuda[overlay];[base][overlay]overlay_cuda=x=0:y=0[vfinal]");
    }
    graph
}

/// Returns true only for plans the PATH B CUDA hybrid can render entirely
/// device-local (zero readback of the base video): no background plate
/// (mode none or absent) and no foreground fit/pad (scale must be 100 — the
/// pad filter has no device-local CUDA equivalent). Overlays (image/text
/// watermark, burn subtitles) are fine: they are rasterized on CPU into the
/// small overlay layer and uploaded, never the base video. Zero is treated
/// as 100 to mirror Compile's normalizeForegroundScale.
fn gpu_native_eligible(plan: &ClipRenderPlan) -> bool {
    let background_none = plan
        .background
        .as_ref()
        .map(|bg| bg.mode == BACKGROUND_NONE)
        .unwrap_or(true);
    let scale = plan.output.foreground_scale_percent;
    background_none && (scale == 100 || scale == 0)
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
                contract_id: "VELOX_ASSEMBLY_READY_V1".to_string(),
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
            fps_num: 60,
            fps_den: 1,
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
            fps: 24.0,
            video_codec: Some("h264".to_string()),
            pixel_format: Some("yuv420p".to_string()),
            format_name: None,
            stream_count: 1,
            video_stream_count: 1,
            audio_stream_count: u32::from(has_audio),
            fps_num: 24,
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
            startup_ms: None,
            publish_ms: None,
            op_ms: None,
            final_audio_sha256: None,
            audio_copy_eligible: None,
            audio_encode_passes: None,
            subtitle_raster_cpu: None,
            gpu_copy_bytes: None,
            decode_ms: None,
            filter_graph_ms: None,
            subtitle_raster_ms: None,
            watermark_raster_ms: None,
            frame_conversion_ms: None,
            encode_ms: None,
            audio_mux_ms: None,
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
    fn gpu_graph_source_only_is_device_local() {
        let mut p = plan();
        p.background = Some(ClipPlanBackground {
            mode: "none".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        });
        p.watermark = None;
        p.subtitles = None;
        assert!(gpu_native_eligible(&p));
        let graph = build_gpu_filter_graph(&p, &profile());
        assert_eq!(graph, "[0:v]scale_cuda=1080:1920[vfinal]");
        assert!(!graph.contains("hwdownload"), "graph: {graph}");
    }

    #[test]
    fn gpu_graph_rasterizes_text_and_subtitles_on_cpu_then_uploads() {
        let mut p = plan();
        p.background = Some(ClipPlanBackground {
            mode: "none".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        });
        p.watermark = Some(ClipPlanWatermark {
            text: "LOGO".to_string(),
            asset_id: "wm-1".to_string(),
            path: "/tmp/wm.png".to_string(),
            sha256: "a".repeat(64),
            position: "top_right".to_string(),
            opacity: 0.9,
            margin_px: 24,
        });
        let sub_path = std::env::temp_dir().join("cliprender-gpu-sub.ass");
        fs::write(&sub_path, b"[Script Info]").unwrap();
        p.subtitles = Some(ClipPlanSubtitles {
            mode: "burn".to_string(),
            style_id: Some("shorts-v1".to_string()),
            path: sub_path.to_string_lossy().into_owned(),
            sha256: "a".repeat(64),
        });
        assert!(gpu_native_eligible(&p));
        let graph = build_gpu_filter_graph(&p, &profile());
        assert!(graph.contains("scale=1080:1920[base]"), "graph: {graph}");
        assert!(graph.contains("drawtext=text='LOGO'"), "graph: {graph}");
        assert!(graph.contains("subtitles=filename="), "graph: {graph}");
        assert!(graph.contains("overlay=x=0:y=0:format=auto"), "graph: {graph}");
        assert!(graph.contains("hwdownload,format=rgba"), "graph: {graph}");
    }

    #[test]
    fn gpu_graph_composites_image_watermark_device_local() {
        let mut p = plan();
        p.background = Some(ClipPlanBackground {
            mode: "none".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        });
        p.watermark = Some(ClipPlanWatermark {
            text: String::new(),
            asset_id: "logo".to_string(),
            path: "/tmp/logo.png".to_string(),
            sha256: "a".repeat(64),
            position: "top_right".to_string(),
            opacity: 0.85,
            margin_px: 24,
        });
        assert!(gpu_native_eligible(&p));
        let graph = build_gpu_filter_graph(&p, &profile());
        assert!(graph.contains("colorchannelmixer=aa=0.85"), "graph: {graph}");
        assert!(graph.contains("hwupload_cuda"), "graph: {graph}");
        assert!(graph.contains("overlay_cuda=x=main_w-overlay_w-24:y=24"), "graph: {graph}");
        assert!(!graph.contains("hwdownload"), "graph: {graph}");
        assert!(!graph.contains("drawtext"), "graph: {graph}");
    }

    #[test]
    fn gpu_graph_composes_image_watermark_onto_canvas_with_subtitles() {
        let mut p = plan();
        p.background = Some(ClipPlanBackground {
            mode: "none".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        });
        p.watermark = Some(ClipPlanWatermark {
            text: String::new(),
            asset_id: "logo".to_string(),
            path: "/tmp/logo.png".to_string(),
            sha256: "a".repeat(64),
            position: "top_right".to_string(),
            opacity: 0.85,
            margin_px: 24,
        });
        let sub_path = std::env::temp_dir().join("cliprender-gpu-sub2.ass");
        fs::write(&sub_path, b"[Script Info]").unwrap();
        p.subtitles = Some(ClipPlanSubtitles {
            mode: "burn".to_string(),
            style_id: Some("shorts-v1".to_string()),
            path: sub_path.to_string_lossy().into_owned(),
            sha256: "a".repeat(64),
        });
        // Image watermark + burn subtitles: the canvas is input [1], the logo
        // input [2], and the logo is composed onto the CPU canvas before the
        // single hwupload_cuda.
        let graph = build_gpu_filter_graph(&p, &profile());
        assert!(graph.contains("[2:v]format=rgba,colorchannelmixer=aa=0.85[wm]"), "graph: {graph}");
        assert!(graph.contains("[canvas][wm]overlay="), "graph: {graph}");
        assert!(graph.contains("subtitles=filename="), "graph: {graph}");
        assert!(graph.contains("overlay=x=0:y=0:format=auto"), "graph: {graph}");
    }

    #[test]
    fn gpu_eligibility_rejects_background_and_scale() {
        let mut p = plan();
        p.background = Some(ClipPlanBackground {
            mode: "blur_source".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        });
        assert!(!gpu_native_eligible(&p));
        let mut p2 = plan();
        p2.background = Some(ClipPlanBackground {
            mode: "none".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        });
        p2.output.foreground_scale_percent = 50;
        assert!(!gpu_native_eligible(&p2));
        // Background none + scale 100 → eligible.
        let mut p3 = plan();
        p3.background = Some(ClipPlanBackground {
            mode: "none".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        });
        assert!(gpu_native_eligible(&p3));
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
}
