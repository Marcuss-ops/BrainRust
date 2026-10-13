use super::*;

pub(super) fn linear_gain(gain_db: f64) -> String {
    format!("{:.6}", 10f64.powf(gain_db / 20.0))
}

// num formats a literal for a filter expression with fixed precision so the
// generated graph is deterministic across builds and locales.
pub(super) fn num(value: f64) -> String {
    format!("{:.6}", value)
}

pub(super) fn secs(us: i64) -> f64 {
    us as f64 / 1_000_000.0
}

// fade_envelope realizes one BGM fade automation entry: unity outside its
// window, inside the linear ramp from silence up to unity over attack and
// back down to silence over release. The compiler emits GainDB == layer
// gain on fade entries, so multiplying the event's own gain by this shape
// alone produces exactly the contracted "silence → layer gain → silence"
// envelope; attack/release of zero collapse that edge to an instant step.
pub(super) fn fade_envelope(start_us: i64, end_us: i64, attack_us: i64, release_us: i64) -> String {
    let rise =
        (attack_us > 0).then(|| format!("(t-{})/{}", num(secs(start_us)), num(secs(attack_us))));
    let fall =
        (release_us > 0).then(|| format!("({}-t)/{}", num(secs(end_us)), num(secs(release_us))));
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
pub(super) fn duck_envelope(
    start_us: i64,
    end_us: i64,
    attack_us: i64,
    release_us: i64,
    ratio: f64,
) -> String {
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
pub(super) fn track_gain_expr(base_gain_db: f64, automations: &[(&Automation, bool)]) -> String {
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

// volume_eval_mode picks the FFmpeg volume filter's expression-evaluation
// mode. Automation envelopes (fade/duck) reference t — the absolute output
// time once adelay shifts the stream PTS — so the expression MUST be
// re-evaluated every frame: eval=frame. A static gain (no targeting
// entries) is a constant expression: eval=once evaluates it at the first
// frame and reuses the value for every frame, which is behaviourally
// identical and skips the per-frame expression evaluation the filter would
// otherwise pay for every event in the graph. The canonical mix-policy
// levels (0 dB / -3 dB / -12 dB voiceovers with no ducking) are static, so
// they take the once path; only BGM fades and trigger-driven ducks pay the
// per-frame cost.
pub(super) fn volume_eval_mode(has_automation: bool) -> &'static str {
    if has_automation {
        "eval=frame"
    } else {
        "eval=once"
    }
}

// source_normalize emits the resample/reformat chain an input needs to
// join the canonical 48 kHz stereo mix. An asset that probes as already
// canonical (48 kHz, 2 channels, explicit stereo layout) skips both
// filters: aresample would be a no-op and aformat would only re-declare
// the layout it already has. Everything else keeps the normalizing chain
// — mono voiceovers must still be upmixed to stereo, 44.1 kHz BGM
// resampled, and layout-ambiguous stereo streams pinned to an explicit
// stereo layout so amix sees uniform inputs. The empty string is safe in
// the filter string: the surrounding commas just join adjacent filters.
// ── BGM loudness normalization ─────────────────────────────────────────────
// The canonical mix policy expresses the BGM bed as a gain RELATIVE TO
// UNITY VOICEOVER (Go: kernel/audio/mix_policy.go BackgroundMusicGainDB),
// but a gain is only source-independent when every BGM source enters the
// mix at the same loudness. Sources are not alike: curated beds carry low
// native loudness (about -37 dB mean) while a full-scale test tone sits
// near -3 dB, so the same bed gain plays 30+ dB apart — and a loud source
// drives the master limiter (alimiter limit=0.95) hard enough to shave
// other tracks' peaks (it cost the SFX presence gate its 6 dB margin).
//
// Every BGM source is therefore measured ONCE per render (ffmpeg
// volumedetect mean_volume, cached per path) and statically trimmed so it
// enters the graph at BGM_SOURCE_LOUDNESS_TARGET_DB before the plan's
// canonical gain and duck automation apply. The plan itself is untouched:
// events keep GainDB == BackgroundMusicGainDB, and because the duck ratio
// is computed against the event's declared gain, the ducked level shifts
// by the same trim — bed AND duck stay source-independent.
//
// The trim is clamped to ±BGM_LOUDNESS_TRIM_MAX_DB: a boost must not lift
// a near-silent source's noise floor into audibility, and a cut keeps
// loud sources from eating the headroom the limiter needs. Measurement
// failure fails OPEN (no trim, today's behaviour): a render must not die
// because a loudness probe hiccuped on an otherwise valid source.
