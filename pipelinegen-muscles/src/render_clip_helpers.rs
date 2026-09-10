
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
        // Allocation-free: iterate split_whitespace directly, no Vec collect.
        let mut iter = line.split_whitespace();
        let f0 = match iter.next() { Some(v) => v, None => return };
        if f0 != "bench:" { return; }
        let f1 = match iter.next() { Some(v) => v, None => return };
        let f2 = match iter.next() { Some(v) => v, None => return };
        let f3 = match iter.next() { Some(v) => v, None => return };
        let f4 = match iter.next() { Some(v) => v, None => return };
        let f5 = match iter.next() { Some(v) => v, None => return };
        let f6 = match iter.next() { Some(v) => v, None => return };
        let f7 = match iter.next() { Some(v) => v, None => return };
        if f2 != "user" || f4 != "sys" || f6 != "real" { return; }
        let _ = (f1, f3); // user/sys values unused, keep for shape validation
        let real_us: i64 = match f5.parse() { Ok(v) => v, Err(_) => return };
        self.saw_bench = true;
        match f7 {
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

    let mut final_label: String;
    match bg_mode {
        BACKGROUND_NONE => {
            graph.push_str(&format!(
                "[src_fg]scale={fg_w}:{fg_h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[fg];"
            ));
            final_label = "[fg]".to_string();
        }
        _ => {
            graph.push_str(&format!(
                "[src_fg]scale={fg_w}:{fg_h}:force_original_aspect_ratio=decrease,setsar=1[fg];"
            ));
            graph.push_str("[bg][fg]overlay=(W-w)/2:(H-h)/2:shortest=1:format=auto[v0];");
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
                "{final_label}drawtext=text='{text}':font='Montserrat':fontcolor=white@{}:fontsize=48:borderw=3:bordercolor=black@{}:{x}:{y}[v1];",
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

/// build_gpu_filter_graph composes the strict CUDA chain — ZERO readback of
/// the base video. Only an image watermark is supported here. Text and burned
/// subtitles are rejected by render_clip before this function runs.
///
///   [0:v] NVDEC → scale_cuda (device-local base)
///   image watermark → hwupload_cuda (only the watermark crosses the PCIe bus)
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

    if text_watermark || burn_subtitles {
        // Defensive marker: production rejects these plans before reaching
        // this builder, preventing any future caller from adding hwdownload.
        return "ZERO_COPY_UNSUPPORTED".to_string();
    }
    if !image_watermark {
        // Plain source-only clip: base video straight to NVENC, all CUDA.
        return format!("[0:v]scale_cuda={w}:{h}[vfinal]");
    }
    let mut graph = format!("[0:v]scale_cuda={w}:{h}[base];");
    if image_watermark {
        let index = 1;
        let watermark = plan.watermark.as_ref().expect("image watermark present");
        let (x, y) = watermark_position(watermark, w, h);
        // Image-only overlay: upload the logo and position it with
        // overlay_cuda (x/y expressions resolve against the base video).
        graph.push_str(&format!(
            "[{index}:v]format=nv12,colorchannelmixer=aa={}[wm];[wm]hwupload_cuda[overlay];[base][overlay]overlay_cuda={x}:{y}[vfinal]",
            watermark.opacity
        ));
        return graph;
    }
    graph
}

fn has_alpha_pixel_format(pixel_format: &str) -> bool {
    matches!(
        pixel_format,
        "rgba" | "bgra" | "argb" | "abgr" | "yuva420p" | "yuva422p" | "yuva444p"
            | "yuva420p10le" | "yuva422p10le" | "yuva444p10le" | "gbrap" | "gbrap10le"
            | "ya8" | "ya16le"
    )
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

