use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::probe::{ffprobe_path, probe_file};
use crate::process::FFmpegRunner;
use crate::protocol::{self, MediaMetadata, Request, Response};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) fn execute(request: Request) -> Response {
    if request.render_plan.is_some() {
        return render_stock_canonical(request);
    }
    render_stock(request)
}

#[derive(Debug, Deserialize)]
struct CanonicalRenderPlan {
    version: String,
    fps: u32,
    fps_numerator: i64,
    fps_denominator: i64,
    duration_frames: i64,
    timeline_hash: String,
    manifest_sha256: String,
    plan_sha256: String,
    manifest: Vec<CanonicalManifestEntry>,
    video_tracks: Vec<CanonicalVideoTrack>,
}

#[derive(Debug, Deserialize)]
struct CanonicalManifestEntry {
    asset_id: String,
    path: String,
    sha256: String,
    frame_count: i64,
}

#[derive(Debug, Deserialize)]
struct CanonicalVideoTrack {
    index: i32,
    segments: Vec<CanonicalVideoSegment>,
}

#[derive(Debug, Deserialize)]
struct CanonicalVideoSegment {
    asset_id: String,
    source: CanonicalFrameRange,
    timeline: CanonicalFrameRange,
    z_index: i32,
}

#[derive(Debug, Deserialize)]
struct CanonicalFrameRange {
    start_frame: i64,
    frame_count: i64,
}

fn render_stock_canonical(request: Request) -> Response {
    let raw_plan = match request.render_plan.clone() {
        Some(value) => value,
        None => return failed_response(None, "render_plan is required".to_string()),
    };
    let plan: CanonicalRenderPlan = match serde_json::from_value(raw_plan) {
        Ok(plan) => plan,
        Err(error) => return failed_response(None, format!("invalid render_plan: {error}")),
    };
    if let Err(error) = validate_canonical_plan(&plan) {
        return failed_response(None, error);
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    let profile = match request.media.profile() {
        Ok(profile) => profile,
        Err(error) => return failed_response(None, error),
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let part = part_path(output);
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }

    let manifest = plan
        .manifest
        .iter()
        .map(|entry| (entry.asset_id.clone(), (entry.path.clone(), entry.frame_count)))
        .collect::<HashMap<_, _>>();
    let mut segments = plan
        .video_tracks
        .iter()
        .flat_map(|track| track.segments.iter())
        .collect::<Vec<_>>();
    // The current canonical compiler emits one primary track. Do not silently
    // flatten additional tracks (which would lose their z-order semantics);
    // validation rejects them below. Sorting by destination frame and z-index
    // makes the executor's ordering explicit for future same-frame layers.
    segments.sort_by_key(|segment| (segment.timeline.start_frame, segment.z_index));
    if segments.is_empty() {
        return failed_response(None, "render_plan has no video segments".to_string());
    }
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for segment in &segments {
        let Some((path, frame_count)) = manifest.get(&segment.asset_id) else {
            return failed_response(None, format!("render_plan asset missing: {}", segment.asset_id));
        };
        let Some(source_end) = segment.source.start_frame.checked_add(segment.source.frame_count) else {
            return failed_response(None, "render_plan source frame range overflows".to_string());
        };
        if source_end > *frame_count {
            return failed_response(None, format!("render_plan source frame range exceeds asset: {}", segment.asset_id));
        }
        command.args(["-i", path]);
    }
    let mut filter = String::new();
    for (index, segment) in segments.iter().enumerate() {
        let end_frame = match segment.source.start_frame.checked_add(segment.source.frame_count) {
            Some(value) => value,
            None => return failed_response(None, "render_plan source frame range overflows".to_string()),
        };
        filter.push_str(&format!(
            "[{index}:v]trim=start_frame={}:end_frame={},setpts=PTS-STARTPTS,scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1[v{index}];",
            segment.source.start_frame,
            end_frame,
            profile.width,
            profile.height,
            profile.width,
            profile.height,
            format!("{}/{}", plan.fps_numerator, plan.fps_denominator)
        ));
    }
    for index in 0..segments.len() {
        filter.push_str(&format!("[v{index}]"));
    }
    filter.push_str(&format!("concat=n={}:v=1:a=0[vfinal]", segments.len()));
    command.args(["-filter_complex", &filter, "-map", "[vfinal]"]);
    if let Err(error) = append_video_options(&mut command, &request) {
        return failed_response(None, error);
    }
    command.args(["-an", "-movflags", "+faststart", &part]);
    match command.output() {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("canonical render failed: {}", String::from_utf8_lossy(&result.stderr).trim()))
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("canonical render failed to start: {error}"))
        }
    }
}

fn validate_canonical_plan(plan: &CanonicalRenderPlan) -> Result<(), String> {
    if plan.version != "render-plan.v2"
        || plan.fps == 0
        || plan.fps_numerator <= 0
        || plan.fps_denominator <= 0
        || plan.duration_frames <= 0
    {
        return Err("invalid render_plan version, frame rate, or duration_frames".to_string());
    }
    let nominal_numerator = match plan.fps_numerator.checked_add(plan.fps_denominator / 2) {
        Some(value) => value,
        None => return Err("render_plan nominal fps calculation overflows".to_string()),
    };
    let nominal_fps = nominal_numerator / plan.fps_denominator;
    if nominal_fps != i64::from(plan.fps) {
        return Err("render_plan nominal fps disagrees with rational frame rate".to_string());
    }
    for hash in [&plan.timeline_hash, &plan.manifest_sha256, &plan.plan_sha256] {
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
            return Err("render_plan hashes must be lowercase SHA256 values".to_string());
        }
    }
    let mut manifest = HashMap::new();
    for entry in &plan.manifest {
        if entry.asset_id.trim().is_empty()
            || entry.path.trim().is_empty()
            || entry.sha256.len() != 64
            || !entry.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            || entry.frame_count <= 0
        {
            return Err("render_plan manifest entry is incomplete".to_string());
        }
        if manifest.contains_key(&entry.asset_id) {
            return Err(format!("render_plan manifest asset is duplicated: {}", entry.asset_id));
        }
        if !Path::new(&entry.path).is_file() {
            return Err(format!("render_plan manifest path is not readable: {}", entry.path));
        }
        let actual = sha256_file(&entry.path)?;
        if actual != entry.sha256 {
            return Err(format!("render_plan manifest hash mismatch for {}", entry.asset_id));
        }
        manifest.insert(entry.asset_id.clone(), entry.frame_count);
    }
    let mut expected_timeline = 0i64;
    for track in &plan.video_tracks {
        if track.index < 0 {
            return Err("render_plan track index is invalid".to_string());
        }
        if track.index != 0 && !track.segments.is_empty() {
            return Err("render_plan additional populated tracks are unsupported".to_string());
        }
        for segment in &track.segments {
            let Some(timeline_end) = segment.timeline.start_frame.checked_add(segment.timeline.frame_count) else {
                return Err(format!("render_plan integer frame segment overflows: {}", segment.asset_id));
            };
            let Some(asset_frame_count) = manifest.get(&segment.asset_id) else {
                return Err(format!("render_plan asset is missing: {}", segment.asset_id));
            };
            let Some(source_end) = segment.source.start_frame.checked_add(segment.source.frame_count) else {
                return Err(format!("render_plan source frame range overflows: {}", segment.asset_id));
            };
            if segment.source.start_frame < 0 || segment.source.frame_count <= 0 || segment.timeline.start_frame != expected_timeline || segment.timeline.frame_count <= 0 || segment.source.frame_count != segment.timeline.frame_count || source_end > *asset_frame_count || timeline_end > plan.duration_frames || segment.z_index < 0 {
                return Err(format!("render_plan integer frame segment is invalid: {}", segment.asset_id));
            }
            expected_timeline = expected_timeline
                .checked_add(segment.timeline.frame_count)
                .ok_or_else(|| "render_plan timeline frame count overflows".to_string())?;
        }
    }
    if expected_timeline != plan.duration_frames {
        return Err("render_plan timeline does not cover duration_frames".to_string());
    }
    Ok(())
}

fn sha256_file(path: &str) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| format!("compute SHA256 for {path}: {error}"))?;
    if !output.status.success() {
        return Err(format!("compute SHA256 for {path} failed"));
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        return Err(format!("sha256sum returned an invalid digest for {path}"));
    }
    Ok(digest)
}

fn render_stock(request: Request) -> Response {
    let inputs = request.input_paths.clone().unwrap_or_default();
    if inputs.is_empty() {
        return failed_response(None, "input_paths is required".to_string());
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    if inputs.iter().any(|path| !Path::new(path).is_file()) {
        return failed_response(
            Some(inputs.join(",")),
            "a render input is not readable".to_string(),
        );
    }
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    if request.no_transitions.unwrap_or(false) && request.no_effects.unwrap_or(false) {
        return render_stock_simple(&request, &inputs, output);
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let profile = match request.media.profile() {
        Ok(value) => value,
        Err(error) => return failed_response(None, error),
    };
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in &inputs {
        command.args(["-i", input]);
    }

    let duration = request.clip_duration_sec.unwrap_or(0);
    let transitions = request.transitions.clone().unwrap_or_default();
    let effect_assignments = request.effect_paths.clone().unwrap_or_default();
    if let Err(error) = validate_resolved_render_plan(
        inputs.len(),
        request.no_transitions.unwrap_or(false),
        &transitions,
        request.no_effects.unwrap_or(false),
        &effect_assignments,
    ) {
        return failed_response(None, error);
    }

    let mut effect_for_clip: Vec<Option<(usize, String)>> = vec![None; inputs.len()];
    for (assignment_index, effect) in effect_assignments.iter().enumerate() {
        if effect.clip_index >= inputs.len() || !Path::new(&effect.path).is_file() {
            return failed_response(
                None,
                format!("invalid resolved effect path: {}", effect.path),
            );
        }
        command.args(["-i", &effect.path]);
        effect_for_clip[effect.clip_index] =
            Some((inputs.len() + assignment_index, effect.path.clone()));
    }

    let mut filter = String::new();
    for index in 0..inputs.len() {
        let mut filters = vec![format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps)];
        if !request.no_transitions.unwrap_or(false) {
            for transition in transitions
                .iter()
                .filter(|transition| transition.clip_index == index)
            {
                filters.push(transition_filter(
                    &transition.id,
                    duration,
                    transition.segment == "start",
                ));
            }
        }
        let effect = effect_for_clip[index].as_ref();
        let with_effect = effect.is_some() && !request.no_effects.unwrap_or(false);
        if with_effect {
            let (effect_input, _) = effect.expect("checked above");
            let opacity = request.overlay_opacity.unwrap_or(0.25);
            filter.push_str(&format!("[{}:v]{}[vtemp{}];[{}:v]scale={}:{}:force_original_aspect_ratio=decrease,fps={},setsar=1,format=yuva420p,colorchannelmixer=aa={}[effect{}];[vtemp{}][effect{}]overlay=shortest=1[v{}];", index, filters.join(","), index, effect_input, profile.width, profile.height, profile.fps, opacity, index, index, index, index));
        } else {
            filter.push_str(&format!("[{}:v]{}[v{}];", index, filters.join(","), index));
        }
    }
    for index in 0..inputs.len() {
        filter.push_str(&format!("[v{}]", index));
    }
    filter.push_str(&format!("concat=n={}:v=1:a=0[vfinal]", inputs.len()));
    command.args(["-filter_complex", &filter, "-map", "[vfinal]"]);
    if !request.keep_audio.unwrap_or(false) {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, &request) {
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);
    match command.output() {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "stock render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("stock render failed: {error}"))
        }
    }
}

fn render_stock_simple(request: &Request, inputs: &[String], output: &str) -> Response {
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    match copy_only_eligibility(request, inputs, ffmpeg) {
        Ok(true) => return render_stock_copy_only(request, inputs, output, ffmpeg),
        Ok(false) => {}
        Err(error) => return failed_response(None, error),
    }
    if gpu_composite_eligibility(request, inputs, ffmpeg) {
        return render_stock_gpu_composite(request, inputs, output, ffmpeg);
    }
    let source = if inputs.len() == 1 {
        inputs[0].clone()
    } else {
        let list_path = format!("{output}.rust.concat.txt");
        let concat_path = format!("{output}.rust.concat.mp4");
        let lines = inputs
            .iter()
            .map(|path| format!("file '{}'", path.replace('\'', "'\\''")))
            .collect::<Vec<_>>();
        if let Err(error) = fs::write(&list_path, lines.join("\n")) {
            return failed_response(None, format!("write concat list: {error}"));
        }
        let mut concat = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
        concat.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            &list_path,
            "-c",
            "copy",
            &concat_path,
        ]);
        let result = concat.output();
        let _ = fs::remove_file(&list_path);
        match result {
            Ok(result) if result.status.success() => concat_path,
            Ok(result) => {
                let _ = fs::remove_file(&concat_path);
                return failed_response(
                    None,
                    format!(
                        "stock concat failed: {}",
                        String::from_utf8_lossy(&result.stderr).trim()
                    ),
                );
            }
            Err(error) => {
                let _ = fs::remove_file(&concat_path);
                return failed_response(None, format!("stock concat failed to start: {error}"));
            }
        }
    };
    let profile = match request.media.profile() {
        Ok(value) => value,
        Err(error) => {
            if inputs.len() > 1 {
                let _ = fs::remove_file(&source);
            }
            return failed_response(None, error);
        }
    };
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        &source,
        "-vf",
    ]);
    command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps));
    command.args(["-map", "0:v:0?"]);
    if request.keep_audio.unwrap_or(false) {
        command.args([
            "-map",
            "0:a:0?",
            "-c:a",
            profile.audio_codec.as_str(),
            "-ar",
            &profile.sample_rate.to_string(),
            "-ac",
            &profile.channels.to_string(),
        ]);
    } else {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, request) {
        if inputs.len() > 1 {
            let _ = fs::remove_file(&source);
        }
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);
    let result = command.output();
    if inputs.len() > 1 {
        let _ = fs::remove_file(&source);
    }
    match result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "stock normalize failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("stock normalize failed to start: {error}"))
        }
    }
}

/// GPU compositing is deliberately narrower than GPU encoding. It is selected
/// only when NVENC is resolved, no pixel effect/transition needs a CPU-only
/// filter, and all inputs already share the requested geometry and frame rate.
/// This prevents a hidden GPU->CPU->GPU transfer and leaves resize/pad/fps work
/// on the safe CPU composite path until a CUDA-compatible graph exists.
fn gpu_composite_eligibility(request: &Request, inputs: &[String], ffmpeg: &str) -> bool {
    if !request.no_transitions.unwrap_or(false)
        || !request.no_effects.unwrap_or(false)
        || request
            .transitions
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || request
            .effect_paths
            .as_ref()
            .is_some_and(|items| !items.is_empty())
    {
        return false;
    }
    let Ok(profile) = request.media.profile() else {
        return false;
    };
    let Ok(encoder) = request.media.encoder() else {
        return false;
    };
    if !encoder.codec.to_ascii_lowercase().ends_with("_nvenc") {
        return false;
    }
    let ffprobe = ffprobe_path(ffmpeg);
    let mut codec: Option<String> = None;
    for input in inputs {
        let Ok(metadata) = probe_file(&ffprobe, input) else {
            return false;
        };
        if !metadata.has_video
            || metadata.width != profile.width
            || metadata.height != profile.height
            || (metadata.fps - profile.fps as f64).abs() > 0.5
            || metadata.video_codec.is_none()
        {
            return false;
        }
        if let Some(reference) = &codec {
            if metadata.video_codec.as_deref() != Some(reference.as_str()) {
                return false;
            }
        } else {
            codec = metadata.video_codec;
        }
    }
    true
}

fn render_stock_gpu_composite(
    request: &Request,
    inputs: &[String],
    output: &str,
    ffmpeg: &str,
) -> Response {
    let (source, temporary_source) = match concat_source(inputs, output, ffmpeg, "gpu") {
        Ok(value) => value,
        Err(error) => return failed_response(None, error),
    };
    let part = part_path(output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-hwaccel",
        "cuda",
        "-hwaccel_output_format",
        "cuda",
        "-i",
        &source,
        "-map",
        "0:v:0?",
    ]);
    if request.keep_audio.unwrap_or(false) {
        command.args(["-map", "0:a:0?"]);
    } else {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, request) {
        if temporary_source {
            let _ = fs::remove_file(&source);
        }
        return failed_response(None, error);
    }
    command.args(["-movflags", "+faststart", &part]);
    let result = command.output();
    if temporary_source {
        let _ = fs::remove_file(&source);
    }
    match result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "GPU composite render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("GPU composite render failed: {error}"))
        }
    }
}

fn concat_source(
    inputs: &[String],
    output: &str,
    ffmpeg: &str,
    suffix: &str,
) -> Result<(String, bool), String> {
    if inputs.len() == 1 {
        return Ok((inputs[0].clone(), false));
    }
    let list_path = format!("{output}.rust.{suffix}.concat.txt");
    let concat_path = format!("{output}.rust.{suffix}.concat.mp4");
    let lines = inputs
        .iter()
        .map(|path| format!("file '{}'", path.replace('\'', "'\\''")))
        .collect::<Vec<_>>();
    fs::write(&list_path, lines.join("\n"))
        .map_err(|error| format!("write {suffix} concat list: {error}"))?;
    let mut concat = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    concat.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &list_path,
        "-c",
        "copy",
        &concat_path,
    ]);
    let result = concat.output();
    let _ = fs::remove_file(&list_path);
    match result {
        Ok(result) if result.status.success() => Ok((concat_path, true)),
        Ok(result) => {
            let _ = fs::remove_file(&concat_path);
            Err(format!(
                "{suffix} concat failed: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            ))
        }
        Err(error) => {
            let _ = fs::remove_file(&concat_path);
            Err(format!("{suffix} concat failed to start: {error}"))
        }
    }
}

/// Resolves the stock fast path conservatively. A stream-copy render is only
/// safe when every input already satisfies the requested canonical contract;
/// otherwise the normal composite/encode path remains responsible for scale,
/// padding, frame-rate conversion, and codec normalization.
fn copy_only_eligibility(
    request: &Request,
    inputs: &[String],
    ffmpeg: &str,
) -> Result<bool, String> {
    if !request.no_transitions.unwrap_or(false)
        || !request.no_effects.unwrap_or(false)
        || request
            .transitions
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || request
            .effect_paths
            .as_ref()
            .is_some_and(|items| !items.is_empty())
    {
        return Ok(false);
    }

    let profile = request.media.profile()?;
    let encoder = request.media.encoder()?;
    let ffprobe = ffprobe_path(ffmpeg);
    let mut first: Option<MediaMetadata> = None;
    for input in inputs {
        let metadata = probe_file(&ffprobe, input)?;
        if !metadata.has_video
            || metadata.width != profile.width
            || metadata.height != profile.height
            || (metadata.fps - profile.fps as f64).abs() > 0.5
            || !video_codec_matches(metadata.video_codec.as_deref(), &encoder.codec)
            || metadata.pixel_format.as_deref() != Some("yuv420p")
        {
            return Ok(false);
        }
        if request.keep_audio.unwrap_or(false)
            && (!metadata.has_audio
                || metadata.audio_codec.as_deref() != Some(profile.audio_codec.as_str())
                || metadata.sample_rate != Some(profile.sample_rate)
                || metadata.channels != Some(profile.channels))
        {
            return Ok(false);
        }
        if let Some(reference) = &first {
            if metadata.video_codec != reference.video_codec
                || metadata.pixel_format != reference.pixel_format
                || metadata.audio_codec != reference.audio_codec
                || metadata.sample_rate != reference.sample_rate
                || metadata.channels != reference.channels
            {
                return Ok(false);
            }
        } else {
            first = Some(metadata);
        }
    }
    Ok(true)
}

fn video_codec_matches(actual: Option<&str>, expected: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    match actual {
        Some(codec) if expected.ends_with("_nvenc") => codec == "h264",
        Some(codec) if expected == "libx264" || expected == "h264" => codec == "h264",
        Some(codec) => codec == expected,
        None => false,
    }
}

fn render_stock_copy_only(
    request: &Request,
    inputs: &[String],
    output: &str,
    ffmpeg: &str,
) -> Response {
    let part = part_path(output);
    let list_path = format!("{output}.rust.copy.concat.txt");
    let lines = inputs
        .iter()
        .map(|path| format!("file '{}'", path.replace('\'', "'\\''")))
        .collect::<Vec<_>>();
    if let Err(error) = fs::write(&list_path, lines.join("\n")) {
        return failed_response(None, format!("write copy concat list: {error}"));
    }
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &list_path,
    ]);
    if !request.keep_audio.unwrap_or(false) {
        command.arg("-an");
    }
    command.args([
        "-map",
        "0:v:0",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        &part,
    ]);
    let result = command.output();
    let _ = fs::remove_file(&list_path);
    match result {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "render_stock".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(None, error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "copy-only stock render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!("copy-only stock render failed to start: {error}"),
            )
        }
    }
}

pub(crate) fn reject_unresolved_selection(request: &Request) -> Option<String> {
    if request.transition_every.is_some()
        || request.effects_dir.is_some()
        || request.effect_every.is_some()
        || request.effect_index_hint.is_some()
    {
        return Some(
            "unresolved transition/effect selection is not supported; Go must send explicit IDs and paths"
                .to_string(),
        );
    }
    None
}

pub(crate) fn validate_resolved_render_plan(
    input_count: usize,
    no_transitions: bool,
    transitions: &[protocol::RenderTransition],
    no_effects: bool,
    effects: &[protocol::RenderEffectPath],
) -> Result<(), String> {
    if !no_transitions {
        if transitions.is_empty() {
            return Err("unresolved render plan: transitions are required".to_string());
        }
        for transition in transitions {
            if transition.clip_index >= input_count
                || !matches!(transition.segment.as_str(), "start" | "end")
                || !supported_transition(&transition.id)
            {
                return Err(format!("invalid resolved transition: {}", transition.id));
            }
        }
    }
    if !no_effects {
        if effects.is_empty() {
            return Err("unresolved render plan: effect paths are required".to_string());
        }
        for effect in effects {
            if effect.clip_index >= input_count || effect.path.trim().is_empty() {
                return Err(format!("invalid resolved effect path: {}", effect.path));
            }
        }
    }
    Ok(())
}

pub(crate) fn supported_transition(name: &str) -> bool {
    matches!(
        name,
        "fadeblack"
            | "fadewhite"
            | "flash"
            | "blur"
            | "gray"
            | "colorred"
            | "colorblue"
            | "colorgreen"
            | "coloryellow"
            | "colorpurple"
            | "colororange"
            | "colorpink"
            | "negate"
            | "vignette"
            | "fastblur"
    )
}

pub(crate) fn transition_filter(name: &str, duration: i32, start: bool) -> String {
    let at = if start { 0.0 } else { duration as f64 - 0.5 };
    match name {
        "fadeblack" => format!(
            "fade=t={}:st={:.6}:d=0.5",
            if start { "in" } else { "out" },
            at
        ),
        "fadewhite" => format!(
            "fade=t={}:st={:.6}:d=0.5:color=white",
            if start { "in" } else { "out" },
            at
        ),
        "flash" => format!(
            "fade=t={}:st={:.6}:d=0.2:color=white",
            if start { "in" } else { "out" },
            if start { 0.0 } else { duration as f64 - 0.2 }
        ),
        "blur" => format!(
            "boxblur=15:enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        "gray" => format!(
            "fade=t={}:st={:.6}:d=0.5:color=gray",
            if start { "in" } else { "out" },
            at
        ),
        "negate" => format!(
            "negate=enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        "vignette" => format!(
            "vignette=enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        "fastblur" => format!(
            "boxblur=30:enable='{}(t,{:.6})'",
            if start { "lt" } else { "gt" },
            at
        ),
        color => {
            let color = color.strip_prefix("color").unwrap_or("black");
            format!(
                "fade=t={}:st={:.6}:d=0.5:color={}",
                if start { "in" } else { "out" },
                at,
                color
            )
        }
    }
}
