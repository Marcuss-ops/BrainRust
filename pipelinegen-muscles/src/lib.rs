//! Native execution components for PipelineGen.
//!
//! This crate owns capability-scoped media execution for the Rust migration.
//! Components here may perform CPU-heavy or process-level work, but must not
//! become an alternate source of canonical state.
//!
//! Initial boundary rules:
//! - SQLite remains owned by the Go application.
//! - Transactional outbox decisions remain owned by the Go application.
//! - Google Drive and Qdrant access must arrive through explicit contracts.
//! - FFmpeg invocation belongs behind a typed execution interface.
//! - No direct credentials or environment-file loading from this crate.

#![deny(unsafe_code)]

/// Stable placeholder proving that the crate is wired and testable.
pub const COMPONENT: &str = "pipelinegen-muscles";
const PROTOCOL_VERSION: &str = "mediaexec.v1";

mod config;
mod encoder;
mod protocol;

use protocol::{CutItem, CutJob, MediaMetadata, Request, Response};
use serde::Deserialize;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;

/// Processes one capability request. The protocol exposes media capabilities,
/// never arbitrary command execution.
pub fn process(request: Request) -> Response {
    let operation = request.operation.clone();
    if request.version != PROTOCOL_VERSION {
        return Response {
            ok: false,
            operation,
            source_path: request.source_path,
            items: Vec::new(),
            metadata: None,
            error: Some(format!("unsupported protocol version: {}", request.version)),
        };
    }
    let mut response = match request.operation.as_str() {
        "health" => Response {
            ok: true,
            operation: "health".to_string(),
            source_path: None,
            items: Vec::new(),
            metadata: None,
            error: None,
        },
        "cut_batch" => cut_batch(request),
        "probe" => probe(request),
        "normalize" => transform(request, "normalize"),
        "cut_copy" => transform(request, "cut_copy"),
        "cut_and_normalize" => transform(request, "cut_and_normalize"),
        "watermark" => transform(request, "watermark"),
        "extract_frame" => transform(request, "extract_frame"),
        "generate_proxy" => transform(request, "generate_proxy"),
        "generate_storyboard" => transform(request, "generate_storyboard"),
        "remux_hls" => transform(request, "remux_hls"),
        "trim" => transform(request, "trim"),
        "render_stock" => render_stock(request),
        "admin_render" => admin_render(request),
        operation => Response {
            ok: false,
            operation: operation.to_string(),
            source_path: request.source_path,
            items: Vec::new(),
            metadata: None,
            error: Some(format!("unsupported operation: {operation}")),
        },
    };
    if !response.ok {
        response.operation = operation;
    }
    response
}

fn cut_batch(request: Request) -> Response {
    let source_path = request.source_path.clone();
    let source = match request.source_path.as_deref() {
        Some(value) if !value.is_empty() && Path::new(value).is_file() => value,
        Some(value) => {
            return failed_response(source_path, format!("source file is not readable: {value}"));
        }
        None => return failed_response(source_path, "source_path is required".to_string()),
    };

    let jobs = request.jobs.unwrap_or_default();
    let mut items = Vec::with_capacity(jobs.len());
    let ffmpeg = request
        .ffmpeg_path
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ffmpeg".to_string());
    let profile = request.media.profile();
    let encoder = match request.media.encoder() {
        Ok(value) => value,
        Err(error) => return failed_response(source_path, error),
    };

    for job in jobs {
        items.push(cut_one(
            &ffmpeg,
            source,
            &job,
            &encoder,
            profile,
            request.no_audio.unwrap_or(false),
            &ffprobe_path(&ffmpeg),
        ));
    }

    let all_failed = !items.is_empty() && items.iter().all(|item| item.status == "failed");
    Response {
        ok: !all_failed,
        operation: "cut_batch".to_string(),
        source_path,
        error: if all_failed {
            Some("all cut jobs failed".to_string())
        } else {
            None
        },
        metadata: None,
        items,
    }
}

fn cut_one(
    ffmpeg: &str,
    source: &str,
    job: &CutJob,
    encoder: &config::EncoderPolicy,
    profile: config::VideoProfile,
    no_audio: bool,
    ffprobe: &str,
) -> CutItem {
    let failed = |message: String| CutItem {
        job_id: job.job_id.clone(),
        output_path: None,
        status: "failed".to_string(),
        size_bytes: 0,
        duration_sec: 0.0,
        error: Some(message),
    };

    if job.output_path.is_empty() {
        return failed("output_path is required".to_string());
    }
    if !job.start_sec.is_finite() || !job.end_sec.is_finite() || job.start_sec < 0.0 {
        return failed("invalid cut timestamps".to_string());
    }
    let duration = job.end_sec - job.start_sec;
    if duration <= 0.0 {
        return failed("end_sec must be greater than start_sec".to_string());
    }

    // Deterministic output paths are resumable execution artifacts. Reuse a
    // non-empty existing output only after the same canonical validation used
    // for a newly cut file; corrupt or partial files are regenerated.
    if let Ok(metadata) = fs::metadata(&job.output_path) {
        if metadata.len() > 0 {
            if let Ok(duration_sec) = validate_output(ffprobe, &job.output_path, no_audio, duration)
            {
                return CutItem {
                    job_id: job.job_id.clone(),
                    output_path: Some(job.output_path.clone()),
                    status: "validated".to_string(),
                    size_bytes: metadata.len(),
                    duration_sec,
                    error: None,
                };
            }
            let _ = fs::remove_file(&job.output_path);
        }
    }

    let output = Path::new(&job.output_path);
    if let Some(parent) = output.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed(format!("create output directory: {error}"));
        }
    }
    let part_path = part_path(&job.output_path);
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-ss", &job.start_sec.to_string()])
        .args(["-i", source])
        .args(["-t", &duration.to_string()])
        .args(["-vf", &format!("scale={}:{:}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}", profile.width, profile.height, profile.width, profile.height, profile.fps)])
        .args(["-map", "0:v:0?"]);
    if !no_audio {
        command.args(["-map", "0:a:0?"]);
    }
    encoder::append_video_args(&mut command, encoder, profile, None);
    command.args(["-avoid_negative_ts", "make_zero"]);
    if no_audio {
        command.arg("-an");
    } else {
        command.args([
            "-c:a",
            profile.audio_codec,
            "-b:a",
            profile.audio_bitrate,
            "-ar",
            "48000",
            "-ac",
            "2",
        ]);
    }
    command.args(["-movflags", "+faststart", &part_path]);

    if let Err(error) = command.output().and_then(|output| {
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(io::Error::other(if stderr.is_empty() {
                format!("ffmpeg exited with {}", output.status)
            } else {
                stderr
            }))
        }
    }) {
        let _ = fs::remove_file(&part_path);
        return failed(format!("ffmpeg cut failed: {error}"));
    }

    if let Err(error) = fs::rename(&part_path, &job.output_path) {
        let _ = fs::remove_file(&part_path);
        return failed(format!("publish cut output: {error}"));
    }
    let size_bytes = match fs::metadata(&job.output_path) {
        Ok(metadata) if metadata.len() > 0 => metadata.len(),
        Ok(_) => return failed("ffmpeg produced an empty output".to_string()),
        Err(error) => return failed(format!("stat cut output: {error}")),
    };
    let duration_sec = match validate_output(ffprobe, &job.output_path, no_audio, duration) {
        Ok(duration_sec) => duration_sec,
        Err(error) => {
            let _ = fs::remove_file(&job.output_path);
            return failed(format!("validate cut output: {error}"));
        }
    };

    CutItem {
        job_id: job.job_id.clone(),
        output_path: Some(job.output_path.clone()),
        status: "validated".to_string(),
        size_bytes,
        duration_sec,
        error: None,
    }
}

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    format: ProbeFormat,
    streams: Vec<ProbeStream>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    pix_fmt: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
}

fn ffprobe_path(ffmpeg: &str) -> String {
    let path = Path::new(ffmpeg);
    if path.file_name().and_then(|name| name.to_str()) == Some("ffmpeg") {
        return path
            .with_file_name("ffprobe")
            .to_string_lossy()
            .into_owned();
    }
    "ffprobe".to_string()
}

fn validate_output(
    ffprobe: &str,
    path: &str,
    no_audio: bool,
    expected_duration: f64,
) -> Result<f64, String> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            path,
        ])
        .output()
        .map_err(|error| format!("ffprobe failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("ffprobe exited with {}", output.status));
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("ffprobe parse failed: {error}"))?;
    let duration = probe
        .format
        .duration
        .as_deref()
        .ok_or_else(|| "ffprobe returned no duration".to_string())?
        .parse::<f64>()
        .map_err(|error| format!("invalid duration: {error}"))?;
    if duration <= 0.0 || (duration - expected_duration).abs() > 0.25 {
        return Err(format!(
            "duration violation: got {duration:.3}s, expected {expected_duration:.3}s"
        ));
    }
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| "no video stream".to_string())?;
    if video.width != Some(1920)
        || video.height != Some(1080)
        || video.codec_name.as_deref() != Some("h264")
        || video.pix_fmt.as_deref() != Some("yuv420p")
        || parse_frame_rate(video.avg_frame_rate.as_deref().unwrap_or(""))
            .map_or(true, |fps| (fps - 24.0).abs() > 0.5)
    {
        return Err("canonical video profile violation".to_string());
    }
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    if no_audio {
        if audio.is_some() {
            return Err("audio stream present with no_audio=true".to_string());
        }
    } else {
        let audio = audio.ok_or_else(|| "audio stream is missing".to_string())?;
        if audio.codec_name.as_deref() != Some("aac")
            || audio.sample_rate.as_deref() != Some("48000")
            || audio.channels != Some(2)
        {
            return Err("canonical audio profile violation".to_string());
        }
    }
    Ok(duration)
}

fn parse_frame_rate(value: &str) -> Option<f64> {
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator.parse::<f64>().ok()?;
        let denominator = denominator.parse::<f64>().ok()?;
        if denominator == 0.0 {
            return None;
        }
        return Some(numerator / denominator);
    }
    value.parse::<f64>().ok()
}

fn probe(request: Request) -> Response {
    let source = match request.source_path.as_deref() {
        Some(path) if !path.is_empty() && Path::new(path).is_file() => path,
        Some(path) => {
            return failed_response(
                Some(path.to_string()),
                format!("source file is not readable: {path}"),
            )
        }
        None => return failed_response(None, "source_path is required".to_string()),
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    match probe_file(&ffprobe_path(ffmpeg), source) {
        Ok(metadata) => Response {
            ok: true,
            operation: "probe".to_string(),
            source_path: Some(source.to_string()),
            items: Vec::new(),
            metadata: Some(metadata),
            error: None,
        },
        Err(error) => failed_response(Some(source.to_string()), error),
    }
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
    let profile = request.media.profile();
    let part = part_path(output);
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in &inputs {
        command.args(["-i", input]);
    }

    let transition_every = request.transition_every.unwrap_or(4);
    let effect_every = request.effect_every.unwrap_or(3);
    let duration = request.clip_duration_sec.unwrap_or(0);
    let mut effect_path = None;
    if !request.no_effects.unwrap_or(false) {
        if let Some(dir) = request.effects_dir.as_deref() {
            let mut effects: Vec<String> = fs::read_dir(dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("mp4"))
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            effects.sort();
            if !effects.is_empty() {
                let hint = request
                    .effect_index_hint
                    .unwrap_or(0)
                    .rem_euclid(effects.len() as i32) as usize;
                effect_path = Some(effects[hint].clone());
                command.args(["-i", &effects[hint]]);
            }
        }
    }

    let mut filter = String::new();
    let mut transitions = Vec::new();
    let overlay_index = inputs.len();
    for index in 0..inputs.len() {
        let mut filters = vec![format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={},setsar=1", profile.width, profile.height, profile.width, profile.height, profile.fps)];
        if !request.no_transitions.unwrap_or(false) && transition_every > 0 {
            if (index as i32 + 1) % transition_every == 0 {
                let name = transition_name((index as i32 + 1) / transition_every);
                filters.push(transition_filter(name, duration, false));
                transitions.push(name.to_string());
            }
            if index > 0 && (index as i32) % transition_every == 0 {
                let name = transition_name(index as i32 / transition_every);
                filters.push(transition_filter(name, duration, true));
            }
        }
        let with_effect = effect_path.is_some()
            && !request.no_effects.unwrap_or(false)
            && effect_every > 0
            && (index as i32 + 1) % effect_every == 0;
        if with_effect {
            let opacity = request.overlay_opacity.unwrap_or(0.25);
            filter.push_str(&format!("[{}:v]{}[vtemp{}];[{}:v]scale={}:{}:force_original_aspect_ratio=decrease,fps={},setsar=1,format=yuva420p,colorchannelmixer=aa={}[effect{}];[vtemp{}][effect{}]overlay=shortest=1[v{}];", index, filters.join(","), index, overlay_index, profile.width, profile.height, profile.fps, opacity, index, index, index, index));
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
        let result = Command::new(ffmpeg)
            .args([
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
            ])
            .output();
        let _ = fs::remove_file(&list_path);
        match result {
            Ok(result) if result.status.success() => concat_path,
            Ok(result) => {
                return failed_response(
                    None,
                    format!(
                        "stock concat failed: {}",
                        String::from_utf8_lossy(&result.stderr).trim()
                    ),
                )
            }
            Err(error) => {
                return failed_response(None, format!("stock concat failed to start: {error}"))
            }
        }
    };
    let profile = request.media.profile();
    let part = part_path(output);
    let mut command = Command::new(ffmpeg);
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
            profile.audio_codec,
            "-ar",
            "48000",
            "-ac",
            "2",
        ]);
    } else {
        command.arg("-an");
    }
    if let Err(error) = append_video_options(&mut command, request) {
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

fn transition_name(index: i32) -> &'static str {
    [
        "fadeblack",
        "fadewhite",
        "flash",
        "blur",
        "gray",
        "colorred",
        "colorblue",
        "colorgreen",
        "coloryellow",
        "colorpurple",
        "colororange",
        "colorpink",
        "negate",
        "vignette",
        "fastblur",
    ][index.rem_euclid(15) as usize]
}

fn transition_filter(name: &str, duration: i32, start: bool) -> String {
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

fn admin_render(request: Request) -> Response {
    let input = match request.source_path.as_deref() {
        Some(path) if Path::new(path).is_file() => path,
        _ => {
            return failed_response(
                request.source_path.clone(),
                "source_path is required and must be readable".to_string(),
            )
        }
    };
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => {
            return failed_response(
                Some(input.to_string()),
                "output_path is required".to_string(),
            )
        }
    };
    let font = match request.font.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(Some(input.to_string()), "font is required".to_string()),
    };
    let effects = request.effects.as_ref().cloned().unwrap_or_default();
    let overlays = request.overlays.as_ref().cloned().unwrap_or_default();
    let part = part_path(output);
    let mut command = Command::new(request.ffmpeg_path.as_deref().unwrap_or("ffmpeg"));
    command.args(["-hide_banner", "-loglevel", "error", "-y", "-i", input]);
    for effect in &effects {
        if !Path::new(&effect.path).is_file() {
            return failed_response(
                Some(input.to_string()),
                format!("effect is not readable: {}", effect.path),
            );
        }
        command.args(["-i", &effect.path]);
    }
    if overlays.is_empty() {
        return failed_response(
            Some(input.to_string()),
            "at least one overlay is required".to_string(),
        );
    }
    let mut video = String::from("[0:v]");
    for overlay in overlays {
        let color = if overlay.color.is_empty() {
            "white"
        } else {
            &overlay.color
        };
        let y = if overlay.y.is_empty() {
            "h*0.50"
        } else {
            &overlay.y
        };
        let text = overlay.text.replace('\'', "\\\\'");
        video.push_str(&format!("drawtext=fontfile={}:text='{}':fontcolor={}:fontsize={}:borderw=3:bordercolor=black:x=(w-text_w)/2:y={}:enable='between(t\\,{}\\,{})',", font, text, color, overlay.size, y, overlay.start, overlay.end));
    }
    video = format!("{}[vout]", video.trim_end_matches(','));
    let mut filter = format!("{};[0:a]aresample=48000,volume=0.78[base]", video);
    let mut labels = String::from("[base]");
    for (index, effect) in effects.iter().enumerate() {
        let duration = if effect.duration <= 0.0 {
            1.0
        } else {
            effect.duration
        };
        let volume = if effect.volume.is_empty() {
            "1"
        } else {
            &effect.volume
        };
        filter.push_str(&format!(";[{}:a]aresample=48000,atrim=duration={:.3},asetpts=PTS-STARTPTS,volume={},adelay={}|{}[sfx{}]", index + 1, duration, volume, effect.delay_ms, effect.delay_ms, index));
        labels.push_str(&format!("[sfx{}]", index));
    }
    filter.push_str(&format!(";{}amix=inputs={}:duration=first:dropout_transition=0:normalize=0,alimiter=limit=0.95[aout]", labels, effects.len() + 1));
    command.args([
        "-filter_complex",
        &filter,
        "-map",
        "[vout]",
        "-map",
        "[aout]",
    ]);
    if let Err(error) = append_video_options(&mut command, &request) {
        return failed_response(None, error);
    }
    command.args([
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-movflags",
        "+faststart",
        "-shortest",
        &part,
    ]);
    match command.output() {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: "admin_render".to_string(),
                source_path: Some(input.to_string()),
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(Some(input.to_string()), error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                Some(input.to_string()),
                format!(
                    "admin render failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(
                Some(input.to_string()),
                format!("admin render failed: {error}"),
            )
        }
    }
}

fn transform(request: Request, operation: &str) -> Response {
    let input = match request.source_path.as_deref() {
        Some(path)
            if !path.is_empty() && (operation == "remux_hls" || Path::new(path).is_file()) =>
        {
            path
        }
        Some(path) => {
            return failed_response(
                Some(path.to_string()),
                format!("source file is not readable: {path}"),
            )
        }
        None => return failed_response(None, "source_path is required".to_string()),
    };
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => {
            return failed_response(
                Some(input.to_string()),
                "output_path is required".to_string(),
            )
        }
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(
                Some(input.to_string()),
                format!("create output directory: {error}"),
            );
        }
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    match operation {
        "normalize" => {
            let profile = request.media.profile();
            command.args(["-i", input, "-vf"]);
            command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}", profile.width, profile.height, profile.width, profile.height, profile.fps));
            if let Err(error) = append_video_options(&mut command, &request) {
                return failed_response(Some(input.to_string()), error);
            }
            if request.keep_audio.unwrap_or(false) {
                command.args(["-c:a", "aac", "-ar", "48000", "-ac", "2"]);
            } else {
                command.arg("-an");
            }
        }
        "extract_frame" => {
            command.args([
                "-ss",
                &request.timestamp_sec.unwrap_or(0.0).to_string(),
                "-i",
                input,
                "-frames:v",
                "1",
                "-an",
            ]);
        }
        "cut_copy" => {
            command.args([
                "-ss",
                &request.start_sec.unwrap_or(0.0).to_string(),
                "-i",
                input,
            ]);
            if let Some(end) = request.end_sec.filter(|value| *value > 0.0) {
                command.args(["-t", &(end - request.start_sec.unwrap_or(0.0)).to_string()]);
            }
            command.args(["-map", "0:v:0?", "-map", "0:a:0?", "-c", "copy"]);
            if request.no_audio.unwrap_or(false) {
                command.arg("-an");
            }
        }
        "cut_and_normalize" => {
            let profile = request.media.profile();
            let start = request.start_sec.unwrap_or(0.0);
            let end = request.end_sec.unwrap_or(start);
            command.args([
                "-ss",
                &start.to_string(),
                "-i",
                input,
                "-t",
                &(end - start).to_string(),
                "-vf",
            ]);
            command.arg(format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,fps={}", profile.width, profile.height, profile.width, profile.height, profile.fps));
            if let Err(error) = append_video_options(&mut command, &request) {
                return failed_response(Some(input.to_string()), error);
            }
            if request.no_audio.unwrap_or(false) {
                command.arg("-an");
            } else {
                command.args(["-c:a", profile.audio_codec, "-ar", "48000", "-ac", "2"]);
            }
        }
        "watermark" => {
            let overlay = request.overlay_path.as_deref().ok_or(());
            if overlay.is_err() {
                return failed_response(
                    Some(input.to_string()),
                    "overlay_path is required".to_string(),
                );
            }
            command.args([
                "-i",
                input,
                "-i",
                overlay.unwrap_or_default(),
                "-filter_complex",
            ]);
            command.arg(format!(
                "[1:v]format=rgba,colorchannelmixer=aa={}[wm];[0:v][wm]overlay=(W-w)/2:(H-h)/2",
                request.opacity.unwrap_or(0.25)
            ));
            command.args(["-map", "0:v:0", "-map", "0:a:0?"]);
            if let Err(error) = append_video_options(&mut command, &request) {
                return failed_response(Some(input.to_string()), error);
            }
            command.args(["-c:a", "aac"]);
        }
        "generate_proxy" => {
            command.args(["-i", input, "-vf", "scale=-2:720"]);
            if let Err(error) = append_video_options(&mut command, &request) {
                return failed_response(Some(input.to_string()), error);
            }
            command.args(["-c:a", "aac"]);
        }
        "generate_storyboard" => {
            let interval = request.interval_frames.unwrap_or(10).max(1);
            let columns = request.columns.unwrap_or(5).max(1);
            let rows = request.rows.unwrap_or(5).max(1);
            command.args(["-i", input, "-vf"]);
            command.arg(format!(
                "select=not(mod(n\\,{interval})),scale=320:-1,tile={columns}x{rows}"
            ));
            command.args(["-frames:v", "1", "-an"]);
        }
        "remux_hls" => {
            command.args(["-i", input, "-c", "copy"]);
        }
        "trim" => {
            let max_duration = request.max_duration_sec.unwrap_or(0.0);
            if !max_duration.is_finite() || max_duration <= 0.0 {
                return failed_response(
                    Some(input.to_string()),
                    "max_duration_sec must be positive".to_string(),
                );
            }
            command.args(["-i", input, "-t", &max_duration.to_string()]);
            let extension = Path::new(input)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(extension.as_str(), "mp4" | "mov" | "mkv") {
                command.args(["-map", "0:v:0?", "-map", "0:a:0?"]);
                if let Err(error) = append_video_options(&mut command, &request) {
                    return failed_response(Some(input.to_string()), error);
                }
                command.args(["-c:a", "aac", "-b:a", "192k"]);
            } else if extension == "wav" {
                command.args(["-vn", "-c:a", "pcm_s16le"]);
            } else {
                command.args(["-vn", "-c:a", "libmp3lame", "-q:a", "2"]);
            }
        }
        _ => {
            return failed_response(
                Some(input.to_string()),
                format!("unsupported transform: {operation}"),
            )
        }
    }
    let part = part_path(output);
    command.args(["-movflags", "+faststart", &part]);
    match command.output() {
        Ok(result) if result.status.success() => match publish_output(&part, output) {
            Ok(()) => Response {
                ok: true,
                operation: operation.to_string(),
                source_path: Some(input.to_string()),
                items: Vec::new(),
                metadata: None,
                error: None,
            },
            Err(error) => failed_response(Some(input.to_string()), error),
        },
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                Some(input.to_string()),
                format!(
                    "{operation} failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(
                Some(input.to_string()),
                format!("{operation} failed to start: {error}"),
            )
        }
    }
}

fn append_video_options(command: &mut Command, request: &Request) -> Result<(), String> {
    let encoder = request.media.encoder()?;
    encoder::append_video_args(
        command,
        &encoder,
        request.media.profile(),
        request.keyframe_interval,
    );
    if let Some(duration) = request.media.duration_sec.filter(|value| *value > 0.0) {
        command.args(["-t", &duration.to_string()]);
    }
    Ok(())
}

fn probe_file(ffprobe: &str, path: &str) -> Result<MediaMetadata, String> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            path,
        ])
        .output()
        .map_err(|error| format!("ffprobe failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("ffprobe exited with {}", output.status));
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("ffprobe parse failed: {error}"))?;
    let duration_sec = probe
        .format
        .duration
        .as_deref()
        .unwrap_or("0")
        .parse::<f64>()
        .unwrap_or(0.0);
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"));
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    Ok(MediaMetadata {
        duration_sec,
        width: video.and_then(|stream| stream.width).unwrap_or(0),
        height: video.and_then(|stream| stream.height).unwrap_or(0),
        fps: video
            .and_then(|stream| parse_frame_rate(stream.avg_frame_rate.as_deref().unwrap_or("")))
            .unwrap_or(0.0),
        video_codec: video.and_then(|stream| stream.codec_name.clone()),
        audio_codec: audio.and_then(|stream| stream.codec_name.clone()),
        sample_rate: audio.and_then(|stream| stream.sample_rate.as_deref()?.parse().ok()),
        channels: audio.and_then(|stream| stream.channels),
        has_video: video.is_some(),
        has_audio: audio.is_some(),
    })
}

fn part_path(final_path: &str) -> String {
    let path = Path::new(final_path);
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => {
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let parent = path
                .parent()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let filename = format!("{stem}.part.{extension}");
            if parent.is_empty() {
                filename
            } else {
                Path::new(parent)
                    .join(filename)
                    .to_string_lossy()
                    .into_owned()
            }
        }
        None => format!("{final_path}.part"),
    }
}

fn publish_output(part_path: &str, final_path: &str) -> Result<(), String> {
    let metadata =
        fs::metadata(part_path).map_err(|error| format!("output is missing: {error}"))?;
    if metadata.len() == 0 {
        let _ = fs::remove_file(part_path);
        return Err("output is empty".to_string());
    }
    fs::rename(part_path, final_path).map_err(|error| format!("publish output: {error}"))
}

fn failed_response(source_path: Option<String>, error: String) -> Response {
    Response {
        ok: false,
        operation: "cut_batch".to_string(),
        source_path,
        items: Vec::new(),
        metadata: None,
        error: Some(error),
    }
}

/// Runs the newline-delimited JSON protocol used by the Go adapter.
pub fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => process(request),
            Err(error) => Response {
                ok: false,
                operation: "invalid".to_string(),
                source_path: None,
                items: Vec::new(),
                metadata: None,
                error: Some(format!("invalid request: {error}")),
            },
        };
        serde_json::to_writer(&mut stdout, &response).map_err(io::Error::other)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{process, Request, COMPONENT};

    #[test]
    fn component_name_is_stable() {
        assert_eq!(COMPONENT, "pipelinegen-muscles");
    }

    #[test]
    fn unsupported_operations_fail_closed() {
        let response = process(Request {
            version: "mediaexec.v1".to_string(),
            operation: "run_command".to_string(),
            ffmpeg_path: None,
            source_path: None,
            output_path: None,
            timestamp_sec: None,
            start_sec: None,
            end_sec: None,
            interval_frames: None,
            columns: None,
            rows: None,
            jobs: None,
            media: crate::config::MediaConfig::default(),
            no_audio: None,
            keep_audio: None,
            overlay_path: None,
            opacity: None,
            input_paths: None,
            no_transitions: None,
            transition_every: None,
            clip_duration_sec: None,
            no_effects: None,
            effects_dir: None,
            effect_every: None,
            effect_index_hint: None,
            overlay_opacity: None,
            keyframe_interval: None,
            font: None,
            effects: None,
            overlays: None,
            max_duration_sec: None,
        });
        assert!(!response.ok);
        assert!(response.error.unwrap().contains("unsupported operation"));
    }
}
