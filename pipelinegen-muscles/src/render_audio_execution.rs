use super::plan::{track_events, validate_plan};
use super::probe::{probe_audio, probe_source_duration};
use super::Plan;
use crate::artifact::{failed_response, mix_path, part_path, publish_output};
use crate::process::FFmpegRunner;
use crate::protocol::{AudioAsset, Request, Response};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub(super) fn execute(request: Request) -> Response {
    let output = match request.output_path.as_deref() {
        Some(value) if !value.is_empty() => value,
        _ => return failed_response(None, "output_path is required".into()),
    };
    let plan: Plan = match request
        .audio_plan
        .as_ref()
        .and_then(|value| serde_json::from_value(value.clone()).ok())
    {
        Some(value) => value,
        None => return failed_response(None, "audio_plan is invalid".into()),
    };
    if let Err(error) = validate_plan(&plan) {
        return failed_response(None, error);
    }
    let by_id: HashMap<String, String> = request
        .audio_assets
        .unwrap_or_default()
        .into_iter()
        .map(|asset: AudioAsset| (asset.asset_id, asset.path))
        .collect();
    let mut source_durations: HashMap<String, f64> = HashMap::new();
    let mut ensure_source = |asset_id: &str, path: &str, required_end_us: i64| -> Result<(), String> {
        let duration = if let Some(duration) = source_durations.get(path) {
            *duration
        } else {
            let duration = probe_source_duration(request.ffmpeg_path.as_deref().unwrap_or("ffmpeg"), path)?;
            source_durations.insert(path.to_owned(), duration);
            duration
        };
        if duration * 1_000_000.0 + 40_000.0 < required_end_us as f64 {
            return Err(format!("audio asset {asset_id} is shorter than required source range"));
        }
        Ok(())
    };
    let mut inputs = Vec::new();
    let mut filters = Vec::new();
    for (track_id, event) in track_events(&plan)
        .into_iter()
        .filter(|(_, event)| event.r#type != "SILENCE")
    {
        let path = match event.asset_id.as_ref().and_then(|id| by_id.get(id)) {
            Some(path) if Path::new(path).is_file() => path,
            Some(path) => return failed_response(Some(path.clone()), format!("audio source is not readable: {path}")),
            None => return failed_response(None, "audio event asset is unresolved".into()),
        };
        let index = inputs.len();
        inputs.push(path.clone());
        let source_end = match event.source_in_us.checked_add(event.source_duration_us) {
            Some(value) if event.source_duration_us > 0 => value,
            _ => return failed_response(None, "audio event source range is incomplete".into()),
        };
        if let Err(error) = ensure_source(event.asset_id.as_deref().unwrap_or(""), path, source_end) {
            return failed_response(Some(path.clone()), error);
        }
        let delay_ms = (event.timeline_start_us + 500) / 1000;
        let mut gain = format!("{}", event.gain_db);
        for automation in &plan.automation {
            let target = if !automation.target_track_id.trim().is_empty() {
                &automation.target_track_id
            } else {
                &automation.target_layer
            };
            if target.eq_ignore_ascii_case(&track_id) {
                gain = format!("if(between(t,{},{}) ,{},{} )", automation.start_us as f64 / 1_000_000.0, automation.end_us as f64 / 1_000_000.0, automation.gain_db, gain);
            }
        }
        filters.push(format!("[{index}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay={delay_ms}|{delay_ms},volume={gain}[a{index}]", event.source_in_us as f64 / 1_000_000.0, source_end as f64 / 1_000_000.0));
    }
    for (layer_name, layers) in [("BGM", &plan.background_music), ("SFX", &plan.sfx)] {
        for layer in layers {
            let path = match by_id.get(&layer.asset_id) {
                Some(path) if Path::new(path).is_file() => path,
                Some(path) => return failed_response(Some(path.clone()), format!("audio source is not readable: {path}")),
                None => return failed_response(None, format!("{layer_name} asset is unresolved")),
            };
            if layer.duration_us <= 0 || layer.timeline_start_us < 0 {
                return failed_response(None, format!("{layer_name} layer range is invalid"));
            }
            let index = inputs.len();
            inputs.push(path.clone());
            if let Err(error) = ensure_source(&layer.asset_id, path, layer.duration_us) {
                return failed_response(Some(path.clone()), error);
            }
            let mut gain = format!("{}", layer.gain_db);
            for automation in &plan.automation {
                if automation.target_track_id.eq_ignore_ascii_case(layer_name)
                    || automation.target_layer.eq_ignore_ascii_case(layer_name)
                {
                    if automation.start_us < 0 || automation.end_us <= automation.start_us {
                        return failed_response(None, "audio automation range is invalid".into());
                    }
                    gain = format!("if(between(t,{},{}) ,{},{} )", automation.start_us as f64 / 1_000_000.0, automation.end_us as f64 / 1_000_000.0, automation.gain_db, gain);
                }
            }
            let delay_ms = (layer.timeline_start_us + 500) / 1000;
            let duration = layer.duration_us as f64 / 1_000_000.0;
            filters.push(format!("[{index}:a]atrim=duration={duration},asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay={delay_ms}|{delay_ms},volume='{gain}'[a{index}]"));
        }
    }
    if inputs.is_empty() {
        filters.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={}[a0]", plan.duration_us as f64 / 1_000_000.0));
    }
    let count = std::cmp::max(inputs.len(), 1);
    let labels: String = (0..count).map(|index| format!("[a{index}]")).collect();
    // Pad the mixed stream to the exact canonical timeline duration. SILENCE
    // events carry no asset and are intentionally dropped above, so a scene
    // whose narration/clip ends before its window leaves a trailing gap that
    // would otherwise make the final_audio shorter than plan.duration_us.
    let pad_secs = plan.duration_us as f64 / 1_000_000.0;
    let filter = if count == 1 {
        format!("{};[a0]apad=whole_dur={pad_secs},alimiter=limit=0.95[aout]", filters.join(";"))
    } else {
        format!("{};{}amix=inputs={count}:duration=longest:normalize=0[mixed];[mixed]apad=whole_dur={pad_secs},alimiter=limit=0.95[aout]", filters.join(";"), labels)
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    let part = part_path(output);
    let mix_part = mix_path(output);
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");

    // Pass 1 — mix: the full filter graph (amix + limiter) into lossless
    // PCM. Timing this pass separately from the AAC encode is what feeds the
    // durable mix_ms / aac_encode_ms metrics.
    let mut mix_command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    mix_command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in &inputs {
        mix_command.args(["-i", input]);
    }
    mix_command.args([
        "-filter_complex", &filter, "-map", "[aout]", "-t",
        &(plan.duration_us as f64 / 1_000_000.0).to_string(), "-c:a", "pcm_s16le",
        &mix_part,
    ]);
    let mix_started = std::time::Instant::now();
    let mix_result = mix_command.output();
    let mix_ms = mix_started.elapsed().as_millis() as i64;
    match mix_result {
        Ok(result) if result.status.success() => {}
        Ok(result) => {
            let _ = fs::remove_file(&mix_part);
            let _ = fs::remove_file(&part);
            return failed_response(None, format!("render_audio_plan mix failed: {}", String::from_utf8_lossy(&result.stderr).trim()));
        }
        Err(error) => {
            let _ = fs::remove_file(&mix_part);
            let _ = fs::remove_file(&part);
            return failed_response(None, format!("render_audio_plan mix failed to start: {error}"));
        }
    }

    // Pass 2 — encode: lossless PCM → canonical AAC-LC 48kHz stereo master.
    let mut encode_command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    encode_command.args([
        "-hide_banner", "-loglevel", "error", "-y", "-i", &mix_part,
        "-c:a", "aac", "-profile:a", "aac_low", "-ar", "48000", "-ac", "2",
        "-b:a", &plan.canonical_audio_profile.bitrate, "-t",
        &(plan.duration_us as f64 / 1_000_000.0).to_string(), &part,
    ]);
    let encode_started = std::time::Instant::now();
    let encode_result = encode_command.output();
    let encode_ms = encode_started.elapsed().as_millis() as i64;
    let _ = fs::remove_file(&mix_part);
    match encode_result {
        Ok(result) if result.status.success() => {}
        Ok(result) => {
            let _ = fs::remove_file(&part);
            return failed_response(None, format!("render_audio_plan encode failed: {}", String::from_utf8_lossy(&result.stderr).trim()));
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            return failed_response(None, format!("render_audio_plan encode failed to start: {error}"));
        }
    }

    // Probe the encoded master, then attach the stage timings to the
    // certified metadata before publishing.
    let probe_started = std::time::Instant::now();
    let probe_result = probe_audio(ffmpeg, &part, plan.duration_us as f64 / 1_000_000.0);
    let probe_ms = probe_started.elapsed().as_millis() as i64;
    match probe_result {
        Ok(mut metadata) => {
            metadata.mix_ms = Some(mix_ms);
            metadata.encode_ms = Some(encode_ms);
            metadata.probe_ms = Some(probe_ms);
            match publish_output(&part, output) {
                Ok(()) => Response { ok: true, operation: "render_audio_plan".into(), source_path: None, items: Vec::new(), metadata: Some(metadata), error: None },
                Err(error) => failed_response(None, error),
            }
        }
        Err(error) => { let _ = fs::remove_file(&part); failed_response(None, error) }
    }
}
