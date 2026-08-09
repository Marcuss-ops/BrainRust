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

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub operation: String,
    pub ffmpeg_path: Option<String>,
    pub source_path: Option<String>,
    pub jobs: Option<Vec<CutJob>>,
    pub codec: Option<String>,
    pub preset: Option<String>,
    pub crf: Option<i32>,
    pub no_audio: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CutJob {
    pub job_id: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub output_path: String,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub ok: bool,
    pub operation: String,
    pub source_path: Option<String>,
    pub items: Vec<CutItem>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CutItem {
    pub job_id: String,
    pub output_path: Option<String>,
    pub status: String,
    pub size_bytes: u64,
    pub duration_sec: f64,
    pub error: Option<String>,
}

/// Processes one capability request. The protocol exposes media capabilities,
/// never arbitrary command execution.
pub fn process(request: Request) -> Response {
    match request.operation.as_str() {
        "health" => Response {
            ok: true,
            operation: "health".to_string(),
            source_path: None,
            items: Vec::new(),
            error: None,
        },
        "cut_batch" => cut_batch(request),
        operation => Response {
            ok: false,
            operation: operation.to_string(),
            source_path: request.source_path,
            items: Vec::new(),
            error: Some(format!("unsupported operation: {operation}")),
        },
    }
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

    for job in jobs {
        items.push(cut_one(
            &ffmpeg,
            source,
            &job,
            request.codec.as_deref().unwrap_or("libx264"),
            request.preset.as_deref().unwrap_or(""),
            request.crf.unwrap_or(0),
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
        items,
    }
}

fn cut_one(
    ffmpeg: &str,
    source: &str,
    job: &CutJob,
    codec: &str,
    preset: &str,
    crf: i32,
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
            if let Ok(duration_sec) = validate_output(ffprobe, &job.output_path, no_audio, duration) {
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
        .args(["-vf", "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,fps=24"])
        .args(["-map", "0:v:0?"]);
    if !no_audio {
        command.args(["-map", "0:a:0?"]);
    }
    command.args(["-c:v", codec]);
    if !preset.is_empty() {
        command.args(["-preset", preset]);
    }
    if crf > 0 && !codec.contains("nvenc") {
        command.args(["-crf", &crf.to_string()]);
    }
    command.args([
        "-pix_fmt",
        "yuv420p",
        "-vsync",
        "cfr",
        "-g",
        "48",
        "-avoid_negative_ts",
        "make_zero",
    ]);
    if no_audio {
        command.arg("-an");
    } else {
        command.args(["-c:a", "aac", "-b:a", "128k", "-ar", "48000", "-ac", "2"]);
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

fn failed_response(source_path: Option<String>, error: String) -> Response {
    Response {
        ok: false,
        operation: "cut_batch".to_string(),
        source_path,
        items: Vec::new(),
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
            operation: "run_command".to_string(),
            ffmpeg_path: None,
            source_path: None,
            jobs: None,
            codec: None,
            preset: None,
            crf: None,
            no_audio: None,
        });
        assert!(!response.ok);
        assert!(response.error.unwrap().contains("unsupported operation"));
    }
}
