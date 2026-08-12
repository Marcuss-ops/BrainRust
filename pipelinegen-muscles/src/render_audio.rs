use crate::artifact::{failed_response, part_path, publish_output};
use crate::process::FFmpegRunner;
use crate::protocol::{AudioAsset, MediaMetadata, Request, Response};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(serde::Deserialize)]
struct Plan {
    duration_us: i64,
    primary_events: Vec<Event>,
    #[serde(default)]
    background_music: Vec<Layer>,
    #[serde(default)]
    sfx: Vec<Layer>,
    #[serde(default)]
    automation: Vec<Automation>,
    canonical_audio_profile: Output,
}
#[derive(serde::Deserialize)]
struct Event {
    r#type: String,
    asset_id: Option<String>,
    timeline_start_us: i64,
    duration_us: i64,
    #[serde(default)]
    source_in_us: i64,
    #[serde(default)]
    source_duration_us: i64,
    #[serde(default)]
    use_original_audio: bool,
    gain_db: f64,
}
#[derive(serde::Deserialize)]
struct Output {
    codec: String,
    profile: String,
    sample_rate: i64,
    channels: i64,
    channel_layout: String,
    bitrate: String,
}
#[derive(serde::Deserialize)]
struct Layer {
    asset_id: String,
    timeline_start_us: i64,
    duration_us: i64,
    #[serde(default)]
    gain_db: f64,
}
#[derive(serde::Deserialize)]
struct Automation {
    target_layer: String,
    start_us: i64,
    end_us: i64,
    gain_db: f64,
    #[serde(default)]
    attack_us: i64,
    #[serde(default)]
    release_us: i64,
}
#[derive(serde::Deserialize)]
struct Probe {
    format: Format,
    streams: Vec<Stream>,
}
#[derive(serde::Deserialize)]
struct Format {
    duration: Option<String>,
    bit_rate: Option<String>,
}
#[derive(serde::Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    start_time: Option<String>,
}

fn validate_plan(plan: &Plan) -> Result<(), String> {
    if plan.duration_us <= 0
        || plan.canonical_audio_profile.codec != "aac"
        || plan.canonical_audio_profile.sample_rate != 48000
        || plan.canonical_audio_profile.channels != 2
        || plan.canonical_audio_profile.channel_layout != "stereo"
        || !plan.canonical_audio_profile.profile.eq_ignore_ascii_case("lc")
        || plan.canonical_audio_profile.bitrate.trim().is_empty()
    {
        return Err("audio_plan violates canonical AAC-LC stereo contract".into());
    }
    let mut expected = 0;
    for event in &plan.primary_events {
        if event.timeline_start_us != expected
            || event.duration_us <= 0
            || event.timeline_start_us < 0
            || event.timeline_start_us + event.duration_us > plan.duration_us
        {
            return Err("audio_plan events are not contiguous and bounded by the timeline".into());
        }
        match event.r#type.as_str() {
            "SILENCE" => {}
            "VOICEOVER" => {
                if event.asset_id.as_deref().unwrap_or("").is_empty() {
                    return Err("audio event asset is unresolved".into());
                }
            }
            "CLIP_AUDIO" => {
                if event.asset_id.as_deref().unwrap_or("").is_empty() {
                    return Err("audio event asset is unresolved".into());
                }
                if !event.use_original_audio {
                    return Err("CLIP_AUDIO must use the original clip audio".into());
                }
            }
            _ => return Err("audio_plan contains an unknown event type".into()),
        }
        if event.source_in_us < 0
            || event.source_duration_us < 0
            || ((event.r#type == "VOICEOVER" || event.r#type == "CLIP_AUDIO")
                && event.source_duration_us <= 0)
        {
            return Err("audio event source range is invalid".into());
        }
        expected += event.duration_us;
    }
    if expected != plan.duration_us {
        return Err("audio_plan duration does not equal event end".into());
    }
    for layer in plan.background_music.iter().chain(plan.sfx.iter()) {
        if layer.asset_id.trim().is_empty()
            || layer.timeline_start_us < 0
            || layer.duration_us <= 0
            || layer.timeline_start_us + layer.duration_us > plan.duration_us
        {
            return Err("audio layer is outside the canonical timeline".into());
        }
    }
    for automation in &plan.automation {
        if automation.target_layer.trim().is_empty()
            || automation.start_us < 0
            || automation.end_us <= automation.start_us
            || automation.end_us > plan.duration_us
            || automation.attack_us < 0
            || automation.release_us < 0
        {
            return Err("audio automation is outside the canonical timeline".into());
        }
    }
    Ok(())
}

fn probe_source_duration(ffmpeg: &str, path: &str) -> Result<f64, String> {
    let ffprobe = crate::probe::ffprobe_path(ffmpeg);
    let mut command = FFmpegRunner::from_ffprobe_path(&ffprobe).ffprobe();
    command.args([
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
        path,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("ffprobe source failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("source asset is corrupt or unreadable: {path}"));
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("invalid source duration for {path}: {error}"))?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err(format!("source asset has no usable duration: {path}"));
    }
    Ok(duration)
}

fn probe_audio(ffmpeg: &str, path: &str, expected_duration: f64) -> Result<MediaMetadata, String> {
    let ffprobe = crate::probe::ffprobe_path(ffmpeg);
    let mut command = FFmpegRunner::from_ffprobe_path(&ffprobe).ffprobe();
    command.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        path,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("ffprobe failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!("ffprobe exited with {}", output.status));
    }
    let probe: Probe = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("ffprobe parse failed: {error}"))?;
    let duration = probe
        .format
        .duration
        .as_deref()
        .ok_or("ffprobe returned no duration")?
        .parse::<f64>()
        .map_err(|error| format!("invalid duration: {error}"))?;
    let audio = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
        .collect::<Vec<_>>();
    if audio.len() != 1 {
        return Err(format!(
            "expected exactly one audio stream, found {}",
            audio.len()
        ));
    }
    let stream = audio[0];
    if stream.codec_name.as_deref() != Some("aac")
        || stream
            .profile
            .as_deref()
            .map_or(true, |profile| !profile.eq_ignore_ascii_case("LC"))
        || stream.sample_rate.as_deref() != Some("48000")
        || stream.channels != Some(2)
    {
        return Err("rendered audio violates canonical AAC-LC 48kHz stereo contract".into());
    }
    if (duration - expected_duration).abs() > 0.25 {
        return Err(format!(
            "rendered audio duration {duration:.3}s differs from expected {expected_duration:.3}s"
        ));
    }
    Ok(MediaMetadata {
        duration_sec: duration,
        bitrate: probe
            .format
            .bit_rate
            .as_deref()
            .and_then(|value| value.parse().ok()),
        width: 0,
        height: 0,
        fps: 0.0,
        video_codec: None,
        pixel_format: None,
        audio_codec: stream.codec_name.clone(),
        audio_profile: stream.profile.clone(),
        sample_rate: stream
            .sample_rate
            .as_deref()
            .and_then(|value| value.parse().ok()),
        channels: stream.channels,
        start_pts: stream
            .start_time
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .map(|value| value.round() as i64),
        has_video: false,
        has_audio: true,
    })
}

pub(crate) fn execute(request: Request) -> Response {
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
    let mut ensure_source = |asset_id: &str,
                             path: &str,
                             required_end_us: i64|
     -> Result<(), String> {
        let duration = if let Some(duration) = source_durations.get(path) {
            *duration
        } else {
            let duration =
                probe_source_duration(request.ffmpeg_path.as_deref().unwrap_or("ffmpeg"), path)?;
            source_durations.insert(path.to_owned(), duration);
            duration
        };
        if duration * 1_000_000.0 + 40_000.0 < required_end_us as f64 {
            return Err(format!(
                "audio asset {asset_id} is shorter than required source range"
            ));
        }
        Ok(())
    };
    let mut inputs = Vec::new();
    let mut filters = Vec::new();
    for event in plan.primary_events.iter().filter(|event| event.r#type != "SILENCE") {
        let path = match event.asset_id.as_ref().and_then(|id| by_id.get(id)) {
            Some(path) if Path::new(path).is_file() => path,
            Some(path) => {
                return failed_response(
                    Some(path.clone()),
                    format!("audio source is not readable: {path}"),
                )
            }
            None => return failed_response(None, "audio event asset is unresolved".into()),
        };
        let index = inputs.len();
        inputs.push(path.clone());
        let source_end = match event.source_in_us.checked_add(event.source_duration_us) {
            Some(value) if event.source_duration_us > 0 => value,
            _ => return failed_response(None, "audio event source range is incomplete".into()),
        };
        if let Err(error) = ensure_source(event.asset_id.as_deref().unwrap_or(""), path, source_end)
        {
            return failed_response(Some(path.clone()), error);
        }
        // The plan remains canonical microseconds; FFmpeg's adelay filter
        // accepts integer milliseconds, so rounding happens only at this
        // executor boundary and never changes the sealed plan/hash.
        let delay_ms = (event.timeline_start_us + 500) / 1000;
        filters.push(format!("[{index}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay={delay_ms}|{delay_ms},volume={}[a{index}]", event.source_in_us as f64 / 1_000_000.0, source_end as f64 / 1_000_000.0, event.gain_db));
    }
    for (layer_name, layers) in [("BGM", &plan.background_music), ("SFX", &plan.sfx)] {
        for layer in layers {
            let path = match by_id.get(&layer.asset_id) {
                Some(path) if Path::new(path).is_file() => path,
                Some(path) => {
                    return failed_response(
                        Some(path.clone()),
                        format!("audio source is not readable: {path}"),
                    )
                }
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
                if automation.target_layer.eq_ignore_ascii_case(layer_name) {
                    if automation.start_us < 0 || automation.end_us <= automation.start_us {
                        return failed_response(None, "audio automation range is invalid".into());
                    }
                    gain = format!(
                        "if(between(t,{},{}) ,{},{} )",
                        automation.start_us as f64 / 1_000_000.0,
                        automation.end_us as f64 / 1_000_000.0,
                        automation.gain_db,
                        gain
                    );
                }
            }
            // adelay is millisecond-granular; keep the canonical microsecond
            // values authoritative and round only for this FFmpeg filter.
            let delay_ms = (layer.timeline_start_us + 500) / 1000;
            let duration = layer.duration_us as f64 / 1_000_000.0;
            filters.push(format!("[{index}:a]atrim=duration={duration},asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay={delay_ms}|{delay_ms},volume='{gain}'[a{index}]"));
        }
    }
    if inputs.is_empty() {
        filters.push(format!(
            "anullsrc=r=48000:cl=stereo,atrim=duration={}[a0]",
            plan.duration_us as f64 / 1_000_000.0
        ));
    }
    let count = std::cmp::max(inputs.len(), 1);
    let labels: String = (0..count).map(|index| format!("[a{index}]")).collect();
    let filter = if count == 1 {
        format!("{}anull,alimiter=limit=0.95[aout]", filters.join(";"))
    } else {
        format!(
            "{};{}amix=inputs={count}:duration=longest:normalize=0[mixed];[mixed]alimiter=limit=0.95[aout]",
            filters.join(";"),
            labels
        )
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    let part = part_path(output);
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in inputs {
        command.args(["-i", &input]);
    }
    let profile = "aac_low";
    command.args([
        "-filter_complex",
        &filter,
        "-map",
        "[aout]",
        "-t",
        &(plan.duration_us as f64 / 1_000_000.0).to_string(),
        "-c:a",
        "aac",
        "-profile:a",
        profile,
        "-ar",
        "48000",
        "-ac",
        "2",
        "-b:a",
        &plan.canonical_audio_profile.bitrate,
        &part,
    ]);
    match command.output() {
        Ok(result) if result.status.success() => {
            match probe_audio(ffmpeg, &part, plan.duration_us as f64 / 1_000_000.0) {
                Ok(metadata) => match publish_output(&part, output) {
                    Ok(()) => Response {
                        ok: true,
                        operation: "render_audio_plan".into(),
                        source_path: None,
                        items: Vec::new(),
                        metadata: Some(metadata),
                        error: None,
                    },
                    Err(error) => failed_response(None, error),
                },
                Err(error) => {
                    let _ = fs::remove_file(&part);
                    failed_response(None, error)
                }
            }
        }
        Ok(result) => {
            let _ = fs::remove_file(&part);
            failed_response(
                None,
                format!(
                    "render_audio_plan failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            )
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, format!("render_audio_plan failed to start: {error}"))
        }
    }
}
