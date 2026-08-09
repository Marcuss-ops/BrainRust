use crate::artifact::{failed_response, part_path};
use crate::config;
use crate::encoder;
use crate::probe::{ffprobe_path, validate_output};
use crate::protocol::{CutItem, CutJob, Request, Response};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

pub(crate) fn execute(request: Request) -> Response {
    cut_batch(request)
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
    let profile = match request.media.profile() {
        Ok(value) => value,
        Err(error) => return failed_response(source_path, error),
    };
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
            profile.clone(),
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
            if let Ok(duration_sec) = validate_output(
                ffprobe,
                &job.output_path,
                no_audio,
                duration,
                &profile,
                encoder,
            ) {
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
    if let Err(error) = encoder::append_video_args(&mut command, encoder, &profile, None) {
        return failed(error);
    }
    command.args(["-avoid_negative_ts", "make_zero"]);
    if no_audio {
        command.arg("-an");
    } else {
        command.args([
            "-c:a",
            profile.audio_codec.as_str(),
            "-b:a",
            &profile.audio_bitrate,
            "-ar",
            &profile.sample_rate.to_string(),
            "-ac",
            &profile.channels.to_string(),
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
    let duration_sec = match validate_output(
        ffprobe,
        &job.output_path,
        no_audio,
        duration,
        &profile,
        encoder,
    ) {
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
