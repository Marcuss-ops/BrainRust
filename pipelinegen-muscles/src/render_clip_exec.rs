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
    let audio_bitrate = match request
        .media
        .audio_bitrate
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => value,
        None => {
            return failed_response(
                None,
                "PROFILE_REQUIRED: audio_bitrate is required for encoded media".to_string(),
            )
        }
    };
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
    // string. cuda_native is strict: only a device-local graph is accepted.
    // Text watermark and burned ASS subtitles require CPU rasterization in
    // this FFmpeg path, so they must never be silently called zero-copy.
    let backend = request
        .render_backend
        .as_deref()
        .unwrap_or("ffmpeg_fallback");
    let requested_codec = encoder.codec.trim().to_ascii_lowercase();
    let nvenc_encoder = requested_codec == "nvenc" || requested_codec.ends_with("_nvenc");
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
    let gpu_native = backend == "cuda_native"
        && nvenc_encoder
        && gpu_native_eligible(&clip_plan);
    // Strict zero-copy contract: cuda_native must never enter the software
    // overlay graph. drawtext/libass necessarily rasterize on CPU in this
    // FFmpeg build and therefore are rejected here, rather than causing a
    // hidden hwdownload/readback. Use an image watermark and subtitle sidecar
    // for this path; GPU glyph compositing belongs to a GPU text compositor,
    // not this Rust FFmpeg adapter.
    if backend == "cuda_native" && (text_watermark || burn_subtitles) {
        return failed_response(
            None,
            "ZERO_COPY_UNSUPPORTED: cuda_native Rust path cannot rasterize text watermark or burned ASS subtitles on GPU; use image watermark + subtitle sidecar or a certified GPU text compositor".to_string(),
        );
    }
    // Other unsupported CUDA plans are also rejected before FFmpeg starts.
    if backend == "cuda_native" && !gpu_native {
        return failed_response(
            None,
            "cuda_native selected but the plan/encoder is not eligible for the zero-readback CUDA hybrid (background, scale or software encoder); resolver/media-policy mismatch".to_string(),
        );
    }
    // The stock overlay_cuda filter in the deployed FFmpeg only accepts a
    // matching opaque CUDA pixel format. It cannot blend RGBA/YUVA over the
    // NV12 decoder surface. Reject alpha-bearing assets rather than silently
    // dropping transparency; opaque RGB assets are normalized to NV12 by the
    // graph below and remain on the device after upload.
    if gpu_native && image_watermark {
        let watermark_path = clip_plan
            .watermark
            .as_ref()
            .map(|watermark| watermark.path.as_str())
            .unwrap_or("");
        let watermark_meta = match probe::probe_file(&ffprobe, watermark_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                return failed_response(
                    None,
                    format!("ZERO_COPY_UNSUPPORTED: cannot verify watermark pixel format: {error}"),
                )
            }
        };
        let pixel_format = watermark_meta.pixel_format.as_deref().unwrap_or("");
        if has_alpha_pixel_format(pixel_format) {
            return failed_response(
                None,
                format!(
                    "ZERO_COPY_UNSUPPORTED: overlay_cuda cannot alpha-blend watermark format {pixel_format} over NV12; native GPU alpha compositor required"
                ),
            );
        }
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
    // Inputs: [0] source, optional background asset, and optional image
    // watermark. The strict CUDA path has no CPU canvas input.
    if gpu_native {
        // Decode through NVDEC and keep the base video device-local; the
        // PATH B graph never downloads it.
        command.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
    } else if nvenc_encoder {
        // Decode through NVDEC on GPU for accelerated hardware decoding
        command.args(["-hwaccel", "cuda"]);
    }
    command.args(["-i", source]);
    if clip_plan.background.as_ref().map(|bg| bg.mode.as_str()) == Some(BACKGROUND_ASSET) {
        command.args([
            "-stream_loop",
            "-1",
            "-i",
            clip_plan
                .background
                .as_ref()
                .and_then(|bg| bg.path.as_deref())
                .unwrap_or(""),
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
                        video_zero_copy: Some(gpu_native),
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
                        ..Default::default()
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
