use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::protocol::{Request, Response};
use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) fn execute(request: Request) -> Response {
    admin_render(request)
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
    let profile = match request.media.profile() {
        Ok(value) => value,
        Err(error) => return failed_response(Some(input.to_string()), error),
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
    let mut filter = format!("{};[0:a]aresample={},volume=0.78[base]", video, profile.sample_rate);
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
        filter.push_str(&format!(";[{}:a]aresample={},atrim=duration={:.3},asetpts=PTS-STARTPTS,volume={},adelay={}|{}[sfx{}]", index + 1, profile.sample_rate, duration, volume, effect.delay_ms, effect.delay_ms, index));
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
        profile.audio_codec.as_str(),
        "-b:a",
        profile.audio_bitrate.as_str(),
        "-ar",
        &profile.sample_rate.to_string(),
        "-ac",
        &profile.channels.to_string(),
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
