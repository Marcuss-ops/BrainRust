use crate::artifact::{failed_response, part_path, publish_output};
use crate::encoder::append_video_options;
use crate::process::FFmpegRunner;
use crate::protocol::{self, Request, Response};
use std::fs;
use std::path::Path;

pub(crate) fn execute(request: Request) -> Response {
    render_stock(request)
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
