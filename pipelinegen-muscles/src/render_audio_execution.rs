use super::plan::{track_events, validate_plan};
use super::probe::{probe_audio, probe_source_duration};
use super::{Automation, Plan};
use crate::artifact::{failed_response, part_path, publish_output, sha256_file};
use crate::process::FFmpegRunner;
use crate::protocol::{AudioAsset, Request, Response};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// The plan carries gains in dB, while the FFmpeg volume filter treats a plain
// number as a linear multiplier (volume=0 silences the stream, and negative
// "gains" would invert/amplify instead of attenuating). Convert at the
// boundary with the standard power ratio so 0 dB maps to unity and the
// canonical duck levels (-12/-18/-24 dB) attenuate as the mix policy intends.
fn linear_gain(gain_db: f64) -> String {
    format!("{:.6}", 10f64.powf(gain_db / 20.0))
}

// num formats a literal for a filter expression with fixed precision so the
// generated graph is deterministic across builds and locales.
fn num(value: f64) -> String {
    format!("{:.6}", value)
}

fn secs(us: i64) -> f64 {
    us as f64 / 1_000_000.0
}

// fade_envelope realizes one BGM fade automation entry: unity outside its
// window, inside the linear ramp from silence up to unity over attack and
// back down to silence over release. The compiler emits GainDB == layer
// gain on fade entries, so multiplying the event's own gain by this shape
// alone produces exactly the contracted "silence → layer gain → silence"
// envelope; attack/release of zero collapse that edge to an instant step.
fn fade_envelope(start_us: i64, end_us: i64, attack_us: i64, release_us: i64) -> String {
    let rise = (attack_us > 0)
        .then(|| format!("(t-{})/{}", num(secs(start_us)), num(secs(attack_us))));
    let fall = (release_us > 0)
        .then(|| format!("({}-t)/{}", num(secs(end_us)), num(secs(release_us))));
    let shape = match (rise, fall) {
        (Some(rise), Some(fall)) => format!("min(min({rise},{fall}),1)"),
        (Some(rise), None) => format!("min({rise},1)"),
        (None, Some(fall)) => format!("min({fall},1)"),
        (None, None) => return String::from("1"),
    };
    format!(
        "if(between(t,{},{}),{},1)",
        num(secs(start_us)),
        num(secs(end_us)),
        shape
    )
}

// duck_envelope realizes one ducking automation entry: unity outside its
// window, the duck ratio inside, and a linear ramp between the two over
// attack at the window start and release at the window end. The duck target
// is an absolute level, so it enters as a ratio against the event's own
// gain (10^((duck_db - event_db)/20)) — multiplying keeps the absolute
// value the plan declares while the ramps keep the transition inaudible
// as a switch. Zero attack/release degrade to the historical step.
fn duck_envelope(start_us: i64, end_us: i64, attack_us: i64, release_us: i64, ratio: f64) -> String {
    let d = num(ratio);
    let start = num(secs(start_us));
    let end = num(secs(end_us));
    let ramp_in = format!(
        "1+({d}-1)*min((t-{s})/{a},1)",
        d = d,
        s = start,
        a = num(secs(attack_us)),
    );
    let ramp_out = format!(
        "{d}+(1-{d})*min(({e}-t)/{r},1)",
        d = d,
        e = end,
        r = num(secs(release_us)),
    );
    let inside = match (attack_us > 0, release_us > 0) {
        (true, true) => format!(
            "if(lt(t,{sa}),{rin},if(lt(t,{er}),{d},{rout}))",
            sa = num(secs(start_us + attack_us)),
            rin = ramp_in,
            er = num(secs(end_us - release_us)),
            d = d,
            rout = ramp_out,
        ),
        (false, true) => format!(
            "if(lt(t,{er}),{d},{rout})",
            er = num(secs(end_us - release_us)),
            d = d,
            rout = ramp_out,
        ),
        (true, false) => format!(
            "if(lt(t,{sa}),{rin},{d})",
            sa = num(secs(start_us + attack_us)),
            rin = ramp_in,
            d = d,
        ),
        (false, false) => d.clone(),
    };
    format!("if(between(t,{s},{e}),{inside},1)", s = start, e = end)
}

// track_gain_expr composes the volume expression for one event or layer:
// the static gain multiplied by every automation envelope targeting its
// track. Fade entries contribute their silence→gain→silence shape, duck
// entries their hold-with-ramps shape; envelopes are cheap (no nesting of
// the base expression), so any number of entries composes linearly.
fn track_gain_expr(base_gain_db: f64, automations: &[(&Automation, bool)]) -> String {
    let mut expr = linear_gain(base_gain_db);
    for (automation, triggered) in automations {
        let envelope = if *triggered {
            let ratio = 10f64.powf((automation.gain_db - base_gain_db) / 20.0);
            duck_envelope(
                automation.start_us,
                automation.end_us,
                automation.attack_us,
                automation.release_us,
                ratio,
            )
        } else {
            fade_envelope(
                automation.start_us,
                automation.end_us,
                automation.attack_us,
                automation.release_us,
            )
        };
        expr.push('*');
        expr.push_str(&envelope);
    }
    expr
}

// event_filter builds the ffmpeg filter graph for ONE already-compiled
// audio event. This is the boundary where "Go decides, Rust executes" is
// enforced: the graph plays EXACTLY the declared source range
// [source_in_us, source_end) once, delayed to the declared timeline start.
// There is no loop filter, no stream_loop, and no duration discovery — a
// plan that wants the music repeated must contain N explicit events (the
// Go loop expander's output). If the source is shorter than the declared
// range, ensure_source fails the render instead of inventing a loop.
fn event_filter(
    index: usize,
    source_in_us: i64,
    source_end_us: i64,
    delay_ms: i64,
    gain: &str,
) -> String {
    format!(
        "[{index}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay={delay_ms}|{delay_ms},volume='{gain}':eval=frame[a{index}]",
        source_in_us as f64 / 1_000_000.0,
        source_end_us as f64 / 1_000_000.0
    )
}

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
    // The plan carries the canonical microsecond duration, but the video
    // renderer emits FrameAt(duration_us) whole frames, which can round the
    // timeline up by up to half a frame. The master is padded to the
    // frame-aligned duration (master_duration_us) so audio_duration >=
    // video_duration instead of leaving a silent tail at the end of the mux.
    let master_duration_us = plan.master_duration_us.unwrap_or(plan.duration_us);
    let master_secs = master_duration_us as f64 / 1_000_000.0;
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
    for (track_id, event) in track_events(&plan)
        .into_iter()
        .filter(|(_, event)| event.r#type != "SILENCE")
    {
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
        let delay_ms = (event.timeline_start_us + 500) / 1000;
        let mut targeting: Vec<(&Automation, bool)> = Vec::new();
        for automation in &plan.automation {
            let target = if !automation.target_track_id.trim().is_empty() {
                &automation.target_track_id
            } else {
                &automation.target_layer
            };
            if target.eq_ignore_ascii_case(&track_id) {
                // A triggered entry ducks under its trigger track; an
                // untriggered one is a fade shape over the layer window.
                targeting.push((automation, !automation.trigger_track_id.trim().is_empty()));
            }
        }
        let gain = track_gain_expr(event.gain_db, &targeting);
        // eval=frame is mandatory for automation: the volume filter's
        // default eval=once decides the gain at the first frame (t=0),
        // so every event would keep ONE level for its whole duration — a
        // duck window [0,20s) would duck a looped BGM's second event
        // (delay 20s) instead of the first, and multi-scene clip ducking
        // windows would never match. With eval=frame the expression is
        // evaluated per frame and, because adelay shifts the stream PTS
        // to absolute output time, the between(t, ...) windows land at
        // the exact absolute positions the plan declares.
        filters.push(event_filter(
            index,
            event.source_in_us,
            source_end,
            delay_ms,
            &gain,
        ));
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
            let mut targeting: Vec<(&Automation, bool)> = Vec::new();
            for automation in &plan.automation {
                if automation.target_track_id.eq_ignore_ascii_case(layer_name)
                    || automation.target_layer.eq_ignore_ascii_case(layer_name)
                {
                    if automation.start_us < 0 || automation.end_us <= automation.start_us {
                        return failed_response(None, "audio automation range is invalid".into());
                    }
                    targeting.push((automation, !automation.trigger_track_id.trim().is_empty()));
                }
            }
            let gain = track_gain_expr(layer.gain_db, &targeting);
            let delay_ms = (layer.timeline_start_us + 500) / 1000;
            let duration = layer.duration_us as f64 / 1_000_000.0;
            // Same eval=frame contract as the track path above: automation
            // windows are absolute output time and must be re-evaluated per
            // frame after the adelay.
            filters.push(format!("[{index}:a]atrim=duration={duration},asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay={delay_ms}|{delay_ms},volume='{gain}':eval=frame[a{index}]"));
        }
    }
    // Silence anchor + apad + atrim. The full-length silent bed is always
    // part of the mix, so the graph emits samples for the entire canonical
    // duration even when every real source ends early (SILENCE events carry
    // no asset and are intentionally dropped above). apad then fills any
    // remaining shortfall and atrim removes any overflow, so the master is
    // pinned to exactly plan.duration_us instead of relying on the output
    // `-t` limit alone.
    let pad_secs = master_secs;
    filters.push(format!(
        "anullsrc=r=48000:cl=stereo,atrim=duration={pad_secs},asetpts=PTS-STARTPTS[asilence]"
    ));
    let mix_labels: String = (0..inputs.len())
        .map(|index| format!("[a{index}]"))
        .chain(std::iter::once("[asilence]".to_string()))
        .collect();
    let filter = if inputs.is_empty() {
        format!("{};[asilence]apad=whole_dur={pad_secs},atrim=duration={pad_secs},alimiter=limit=0.95[aout]", filters.join(";"))
    } else {
        format!("{};{}amix=inputs={}:duration=longest:normalize=0[mixed];[mixed]apad=whole_dur={pad_secs},atrim=duration={pad_secs},alimiter=limit=0.95[aout]", filters.join(";"), mix_labels, inputs.len() + 1)
    };
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    let part = part_path(output);
    // Keep the filter graph out of argv. With many clips/scenes the graph can
    // exceed the host ARG_MAX limit even though the actual audio plan is
    // valid. ffmpeg's script input has the same semantics without that limit.
    let filter_script = format!("{part}.filter_complex.txt");
    if let Err(error) = fs::write(&filter_script, &filter) {
        return failed_response(None, format!("write audio filter script: {error}"));
    }
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");

    // Single pass: render the complete filter graph directly to the
    // canonical AAC-LC master. The previous PCM intermediate caused a full
    // second read/write/encode pass over the entire timeline. The filter
    // graph still performs the exact same mix, limiter, padding and trim;
    // only the lossless intermediate is removed.
    let mut encode_command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    encode_command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in &inputs {
        encode_command.args(["-i", input]);
    }
    encode_command.args([
        "-filter_complex_script",
        &filter_script,
        "-map",
        "[aout]",
        "-c:a",
        "aac",
        "-profile:a",
        "aac_low",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-b:a",
        &plan.canonical_audio_profile.bitrate,
        "-t",
        &master_secs.to_string(),
        &part,
    ]);
    let encode_started = std::time::Instant::now();
    let encode_result = encode_command.output();
    let aac_encode_ms = encode_started.elapsed().as_millis() as i64;
    let _ = fs::remove_file(&filter_script);
    match encode_result {
        Ok(result) if result.status.success() => {}
        Ok(result) => {
            let _ = fs::remove_file(&part);
            return failed_response(
                None,
                format!(
                    "render_audio_plan encode failed: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ),
            );
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            return failed_response(
                None,
                format!("render_audio_plan encode failed to start: {error}"),
            );
        }
    }

    // Probe the encoded master, then attach the stage timings to the
    // certified metadata before publishing.
    let probe_started = std::time::Instant::now();
    let probe_result = probe_audio(ffmpeg, &part, master_secs);
    let probe_ms = probe_started.elapsed().as_millis() as i64;
    match probe_result {
        Ok(mut metadata) => {
            // The mix is now fused into the single encode pass. Keep the
            // legacy metric explicit rather than reporting a fabricated
            // second-pass duration.
            metadata.mix_ms = Some(0);
            metadata.aac_encode_ms = Some(aac_encode_ms);
            metadata.probe_ms = Some(probe_ms);
            match publish_output(&part, output) {
                Ok(()) => {
                    // Hash the published final audio in its owner (Rust), so
                    // the Go adapter maps hash_ms + the digest instead of
                    // re-measuring the hash round-trip as a single block.
                    let hash_started = std::time::Instant::now();
                    match sha256_file(output) {
                        Ok(digest) => {
                            let hash_ms = hash_started.elapsed().as_millis() as i64;
                            metadata.hash_ms = Some(hash_ms.max(1));
                            metadata.final_audio_sha256 = Some(digest);
                            Response {
                                ok: true,
                                operation: "render_audio_plan".into(),
                                source_path: None,
                                items: Vec::new(),
                                metadata: Some(metadata),
                                error: None,
                            }
                        }
                        Err(error) => failed_response(None, error),
                    }
                }
                Err(error) => failed_response(None, error),
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&part);
            failed_response(None, error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{duck_envelope, event_filter, fade_envelope, linear_gain, track_gain_expr, Automation};

    #[test]
    fn fade_envelope_ramps_from_and_to_silence() {
        // Gate A's contract: 800ms fade-in, 1000ms fade-out over [0,25s).
        let expr = fade_envelope(0, 25_000_000, 800_000, 1_000_000);
        assert!(
            expr.starts_with("if(between(t,0.000000,25.000000),"),
            "envelope must be unity outside its window: {expr}"
        );
        assert!(
            expr.contains("min(min((t-0.000000)/0.800000,(25.000000-t)/1.000000),1)"),
            "envelope must ramp from silence and back: {expr}"
        );
    }

    #[test]
    fn fade_envelope_without_ramps_is_unity() {
        assert_eq!(fade_envelope(0, 10_000_000, 0, 0), "1");
    }

    #[test]
    fn duck_envelope_holds_ratio_with_edge_ramps() {
        // Duck to -28dB against a -18dB base over [0,15s): 120ms attack,
        // 350ms release.
        let ratio = 10f64.powf((-28.0 - -18.0) / 20.0);
        let expr = duck_envelope(0, 15_000_000, 120_000, 350_000, ratio);
        assert!(
            expr.contains("1+(0.316228-1)*min((t-0.000000)/0.120000,1)"),
            "attack must ramp base→duck: {expr}"
        );
        assert!(
            expr.contains("if(lt(t,14.650000),0.316228,"),
            "the ducked level must hold between the ramps: {expr}"
        );
        assert!(
            expr.contains("0.316228+(1-0.316228)*min((15.000000-t)/0.350000,1)"),
            "release must ramp duck→base: {expr}"
        );
    }

    #[test]
    fn duck_envelope_without_ramps_is_the_plain_step() {
        let expr = duck_envelope(5_000_000, 9_000_000, 0, 0, 0.25);
        assert_eq!(
            expr, "if(between(t,5.000000,9.000000),0.250000,1)",
            "zero attack/release must keep the historical step semantics"
        );
    }

    #[test]
    fn track_gain_expr_multiplies_the_static_gain_by_every_targeting_envelope() {
        let fade = Automation {
            target_track_id: "bgm".into(),
            target_layer: String::new(),
            trigger_track_id: String::new(),
            start_us: 0,
            end_us: 25_000_000,
            gain_db: -18.0,
            attack_us: 800_000,
            release_us: 1_000_000,
        };
        let duck = Automation {
            target_track_id: "bgm".into(),
            target_layer: String::new(),
            trigger_track_id: "voiceover".into(),
            start_us: 0,
            end_us: 15_000_000,
            gain_db: -28.0,
            attack_us: 120_000,
            release_us: 350_000,
        };
        let expr = track_gain_expr(-18.0, &[(&fade, false), (&duck, true)]);
        assert!(expr.starts_with("0.125893*"), "static gain first: {expr}");
        assert!(expr.contains("if(between(t,0.000000,25.000000)"), "fade shape: {expr}");
        assert!(expr.contains("0.316228"), "duck ratio is relative to the base gain: {expr}");
    }

    #[test]
    fn event_filter_plays_only_the_declared_source_range_once() {
        // The fundamental expansion's 2nd BGM event ([20s, 40s), source
        // restarting from 0) must be a single atrim of exactly [0,20s)
        // delayed to 20s — the whole 20s source played once, never looped.
        let filter = event_filter(1, 0, 20_000_000, 20_000, "0.063096");
        assert!(filter.contains("atrim=start=0:end=20,"), "{filter}");
        assert!(filter.contains("adelay=20000|20000"), "{filter}");
        assert!(filter.contains("volume='0.063096':eval=frame"), "{filter}");
        assert!(filter.ends_with("[a1]"), "{filter}");
    }

    #[test]
    fn event_filter_truncates_the_last_loop_event_not_the_source() {
        // The fundamental expansion's 4th event covers [60s, 75s): a 15s
        // trim from the same 20s source, delayed to 60s. Rust renders the
        // declared 15s — it never plays the whole 20s source again nor
        // loops it to fill the timeline.
        let filter = event_filter(3, 0, 15_000_000, 60_000, "0.063096");
        assert!(filter.contains("atrim=start=0:end=15,"), "{filter}");
        assert!(filter.contains("adelay=60000|60000"), "{filter}");
    }

    #[test]
    fn event_filter_applies_declared_source_trim_for_sfx() {
        // An SFX with source_in_ms=250, duration_ms=900 is trimmed to the
        // declared [0.25s, 1.15s) slice of its source — the source is never
        // played beyond the compiled event.
        let filter = event_filter(0, 250_000, 1_150_000, 12_000, "0.398107");
        assert!(filter.contains("atrim=start=0.25:end=1.15,"), "{filter}");
        assert!(filter.contains("adelay=12000|12000"), "{filter}");
    }

    #[test]
    fn event_filter_never_invents_a_loop() {
        // The renderer must never add a loop/aloop/stream_loop filter: a
        // compiled plan carries N explicit events for N repeats, and Rust
        // executes them verbatim.
        for filter in [
            event_filter(0, 0, 20_000_000, 0, "0.063096"),
            event_filter(3, 0, 15_000_000, 60_000, "0.063096"),
        ] {
            assert!(!filter.contains("loop"), "filter must never loop: {filter}");
        }
    }

    #[test]
    fn linear_gain_maps_zero_db_to_unity() {
        assert_eq!(linear_gain(0.0), "1.000000");
    }

    #[test]
    fn linear_gain_maps_policy_duck_levels() {
        assert_eq!(linear_gain(-12.0), "0.251189");
        assert_eq!(linear_gain(-18.0), "0.125893");
        assert_eq!(linear_gain(-24.0), "0.063096");
    }

    #[test]
    fn linear_gain_is_never_negative() {
        for db in [-30.0, -18.0, -6.0, 0.0, 3.0, 12.0] {
            let value: f64 = linear_gain(db).parse().unwrap();
            assert!(value.is_finite() && value > 0.0);
        }
    }
}
