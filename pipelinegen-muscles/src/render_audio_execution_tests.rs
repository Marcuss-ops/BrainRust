use super::{
    bgm_loudness_trim_db, duck_envelope, event_filter, fade_envelope, final_mix_script,
    linear_gain, loudness_trim_filter, parse_volumedetect_mean, partition_stems,
    silence_bed_filter, source_normalize, stem_count_from, stem_filter_script, track_gain_expr,
    volume_eval_mode, Automation, ChainSpec, SourceProbe, BGM_SOURCE_LOUDNESS_TARGET_DB,
    STEM_FANOUT,
};

fn chain(path: &str) -> ChainSpec {
    ChainSpec {
        path: path.to_string(),
        source_in_us: 0,
        source_end_us: 1_000_000,
        delay_ms: 0,
        gain: "1.000000".to_string(),
        eval_mode: "eval=once",
        normalize: String::new(),
    }
}

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
    assert!(
        expr.contains("if(between(t,0.000000,25.000000)"),
        "fade shape: {expr}"
    );
    assert!(
        expr.contains("0.316228"),
        "duck ratio is relative to the base gain: {expr}"
    );
}

#[test]
fn event_filter_plays_only_the_declared_source_range_once() {
    // The fundamental expansion's 2nd BGM event ([20s, 40s), source
    // restarting from 0) must be a single atrim of exactly [0,20s)
    // delayed to 20s — the whole 20s source played once, never looped.
    let filter = event_filter(1, 0, 20_000_000, 20_000, "0.063096", "eval=once", "");
    assert!(filter.contains("atrim=start=0:end=20,"), "{filter}");
    assert!(filter.contains("adelay=20000|20000"), "{filter}");
    assert!(filter.contains("volume='0.063096':eval=once"), "{filter}");
    assert!(filter.ends_with("[a1]"), "{filter}");
}

#[test]
fn event_filter_keeps_eval_frame_for_automated_gains() {
    // An event whose gain expression depends on absolute output time
    // (duck/fade automation targeting its track) must keep eval=frame:
    // only the per-frame evaluation makes between(t, ...) windows land
    // at the declared absolute positions after the adelay.
    let filter = event_filter(
        1,
        0,
        20_000_000,
        20_000,
        "0.063096*if(between(t,0.000000,15.000000),0.316228,1)",
        "eval=frame",
        "",
    );
    assert!(
        filter
            .contains("volume='0.063096*if(between(t,0.000000,15.000000),0.316228,1)':eval=frame"),
        "{filter}"
    );
    assert!(filter.contains("adelay=20000|20000"), "{filter}");
}

#[test]
fn event_filter_truncates_the_last_loop_event_not_the_source() {
    // The fundamental expansion's 4th event covers [60s, 75s): a 15s
    // trim from the same 20s source, delayed to 60s. Rust renders the
    // declared 15s — it never plays the whole 20s source again nor
    // loops it to fill the timeline.
    let filter = event_filter(3, 0, 15_000_000, 60_000, "0.063096", "eval=once", "");
    assert!(filter.contains("atrim=start=0:end=15,"), "{filter}");
    assert!(filter.contains("adelay=60000|60000"), "{filter}");
}

#[test]
fn event_filter_applies_declared_source_trim_for_sfx() {
    // An SFX with source_in_ms=250, duration_ms=900 is trimmed to the
    // declared [0.25s, 1.15s) slice of its source — the source is never
    // played beyond the compiled event.
    let filter = event_filter(0, 250_000, 1_150_000, 12_000, "0.398107", "eval=once", "");
    assert!(filter.contains("atrim=start=0.25:end=1.15,"), "{filter}");
    assert!(filter.contains("adelay=12000|12000"), "{filter}");
}

#[test]
fn event_filter_never_invents_a_loop() {
    // The renderer must never add a loop/aloop/stream_loop filter: a
    // compiled plan carries N explicit events for N repeats, and Rust
    // executes them verbatim.
    for filter in [
        event_filter(0, 0, 20_000_000, 0, "0.063096", "eval=once", ""),
        event_filter(3, 0, 15_000_000, 60_000, "0.063096", "eval=frame", ""),
    ] {
        assert!(!filter.contains("loop"), "filter must never loop: {filter}");
    }
}

#[test]
fn source_normalize_skips_conversion_for_canonical_assets() {
    let canonical = SourceProbe {
        duration_sec: 10.0,
        sample_rate: Some(48000),
        channels: Some(2),
        channel_layout: Some("stereo".into()),
    };
    assert_eq!(source_normalize(&canonical), "");
}

#[test]
fn source_normalize_keeps_conversion_for_non_canonical_assets() {
    // Mono 24 kHz voiceover: must be resampled AND upmixed to stereo.
    let voiceover = SourceProbe {
        duration_sec: 10.0,
        sample_rate: Some(24000),
        channels: Some(1),
        channel_layout: Some("mono".into()),
    };
    assert_eq!(
        source_normalize(&voiceover),
        "aresample=48000,aformat=channel_layouts=stereo,"
    );
    // Stereo 44.1 kHz BGM: resample needed.
    let bgm = SourceProbe {
        duration_sec: 10.0,
        sample_rate: Some(44100),
        channels: Some(2),
        channel_layout: Some("stereo".into()),
    };
    assert_eq!(
        source_normalize(&bgm),
        "aresample=48000,aformat=channel_layouts=stereo,"
    );
    // 48 kHz 2ch with unknown layout: pin the stereo layout explicitly.
    let ambiguous = SourceProbe {
        duration_sec: 10.0,
        sample_rate: Some(48000),
        channels: Some(2),
        channel_layout: None,
    };
    assert_eq!(
        source_normalize(&ambiguous),
        "aresample=48000,aformat=channel_layouts=stereo,"
    );
}

#[test]
fn event_filter_skips_normalize_for_canonical_sources() {
    let filter = event_filter(1, 0, 20_000_000, 20_000, "0.063096", "eval=once", "");
    assert!(!filter.contains("aresample"), "{filter}");
    assert!(!filter.contains("aformat"), "{filter}");
    assert!(
        filter.contains("asetpts=PTS-STARTPTS,adelay=20000|20000"),
        "{filter}"
    );
}

#[test]
fn event_filter_normalizes_non_canonical_sources() {
    let filter = event_filter(
        1,
        0,
        20_000_000,
        20_000,
        "0.063096",
        "eval=once",
        "aresample=48000,aformat=channel_layouts=stereo,",
    );
    assert!(
            filter.contains("asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo,adelay=20000|20000"),
            "{filter}"
        );
}

#[test]
fn volume_eval_mode_is_frame_only_with_automation() {
    // Static gains (no targeting entries) evaluate once; any automation
    // envelope references t and must re-evaluate per frame.
    assert_eq!(volume_eval_mode(false), "eval=once");
    assert_eq!(volume_eval_mode(true), "eval=frame");
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

#[test]
fn bgm_loudness_trim_lands_any_source_at_the_canonical_target() {
    // The bed's whole point: a full-scale tone (-3 dB mean), a curated
    // quiet bed (-37 dB mean) and a hot source all end up entering the
    // mix at the SAME loudness, so BackgroundMusicGainDB is
    // source-independent.
    for mean_db in [-3.0, -37.0, -18.0, 0.0] {
        let trim = bgm_loudness_trim_db(mean_db).expect("finite mean must trim");
        assert!(
            ((mean_db + trim) - BGM_SOURCE_LOUDNESS_TARGET_DB).abs() < 1e-9,
            "mean {mean_db} dB + trim {trim} must reach {BGM_SOURCE_LOUDNESS_TARGET_DB} dB"
        );
    }
}

#[test]
fn bgm_loudness_trim_is_clamped_to_the_headroom_budget() {
    // A near-silent source must not be boosted more than ±24 dB: the
    // clamp keeps its noise floor from rising into audibility, and
    // keeps a loud source from eating the limiter's headroom.
    let silence = bgm_loudness_trim_db(-91.0).expect("finite mean must trim");
    assert_eq!(silence, 24.0);
    let loud = bgm_loudness_trim_db(6.0).expect("finite mean must trim");
    assert_eq!(loud, -24.0);
}

#[test]
fn bgm_loudness_trim_fails_open_on_non_finite_measurement() {
    // volumedetect reports digital silence as -inf: no trim, the
    // render proceeds exactly as it did before normalization existed.
    assert_eq!(bgm_loudness_trim_db(f64::NEG_INFINITY), None);
    assert_eq!(bgm_loudness_trim_db(f64::NAN), None);
}

#[test]
fn parse_volumedetect_mean_reads_the_mean_line() {
    let stderr = b"[Parsed_volumedetect_0 @ 0x0] n_samples: 480000\n\n[Parsed_volumedetect_0 @ 0x0] mean_volume: -23.4 dB\n[Parsed_volumedetect_0 @ 0x0] max_volume: -3.0 dB\n";
    assert_eq!(parse_volumedetect_mean(stderr), Some(-23.4));
}

#[test]
fn parse_volumedetect_mean_fails_open_on_silence_or_missing_stat() {
    assert_eq!(
        parse_volumedetect_mean(b"[Parsed_volumedetect_0] mean_volume: -inf dB\n"),
        None
    );
    assert_eq!(parse_volumedetect_mean(b"no volumedetect here\n"), None);
}

#[test]
fn loudness_trim_filter_is_a_volume_fragment_for_the_normalize_prefix() {
    assert_eq!(loudness_trim_filter(-19.99), "volume=-19.990000dB,");
    // The fragment must compose into the event filter chain before
    // adelay without disturbing the canonical volume expression.
    let filter = event_filter(
        0,
        0,
        10_000_000,
        0,
        "0.5",
        "eval=once",
        &loudness_trim_filter(-20.0),
    );
    assert!(
        filter.contains("volume=-20.000000dB,adelay=0|0"),
        "{filter}"
    );
    assert!(filter.contains("volume='0.5':eval=once"), "{filter}");
}

// ── Stem fan-out ──────────────────────────────────────────────────────

#[test]
fn stem_count_caps_the_fanout_and_shrinks_with_the_chain_count() {
    assert_eq!(stem_count_from(0, None), 0);
    assert_eq!(stem_count_from(1, None), 1);
    assert_eq!(stem_count_from(STEM_FANOUT - 1, None), STEM_FANOUT - 1);
    assert_eq!(stem_count_from(58, None), STEM_FANOUT);
    // The host override is clamped to the chain count and must be >= 1.
    assert_eq!(stem_count_from(58, Some(8)), 8);
    assert_eq!(stem_count_from(2, Some(8)), 2);
    assert_eq!(stem_count_from(58, Some(0)), STEM_FANOUT);
}

#[test]
fn partition_stems_cover_every_chain_exactly_once() {
    let chains: Vec<ChainSpec> = (0..7).map(|index| chain(&format!("p{index}"))).collect();
    let stems = partition_stems(chains, 3);
    assert_eq!(stems.len(), 3);
    let mut seen: Vec<String> = stems.iter().flatten().map(|c| c.path.clone()).collect();
    seen.sort();
    assert_eq!(
        seen,
        (0..7).map(|index| format!("p{index}")).collect::<Vec<_>>()
    );
    // Round-robin balances the stems to within one chain.
    let sizes: Vec<usize> = stems.iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![3, 2, 2]);
}

#[test]
fn partition_stems_with_zero_chains_is_empty() {
    assert!(partition_stems(Vec::new(), 0).iter().all(Vec::is_empty));
}
#[test]
fn stem_filter_script_remaps_stream_indices_to_stem_local_inputs() {
    // A stem's argv only opens ITS sources: a stem carrying global
    // chains 0 and 2 must reference [0:a]/[1:a] — its own two argv
    // positions — with matching stem-local output labels, never [2:a].
    let stems = partition_stems(vec![chain("a"), chain("b"), chain("c")], 2);
    assert_eq!(stems[0].len(), 2);
    let script = stem_filter_script(&stems[0], 10.0);
    assert!(script.contains("[0:a]atrim=start=0:end=1"), "{script}");
    assert!(script.contains("[1:a]atrim=start=0:end=1"), "{script}");
    assert!(!script.contains("[2:a]"), "{script}");
    assert!(
        script.contains("[a0][a1][asilence]amix=inputs=3"),
        "{script}"
    );
}

#[test]
fn stem_filter_script_sums_chains_plus_silence_and_pins_the_master() {
    let script = stem_filter_script(&[chain("a"), chain("b")], 10.0);
    assert!(script.contains("anullsrc=r=48000:cl=stereo"), "{script}");
    assert!(
        script.contains("amix=inputs=3:duration=longest:normalize=0[mixed]"),
        "{script}"
    );
    assert!(
        script.contains("apad=whole_dur=10,atrim=duration=10[astem]"),
        "{script}"
    );
    // The limiter belongs to the final pass over the TOTAL mix only.
    assert!(!script.contains("alimiter"), "{script}");
}

#[test]
fn final_mix_script_pins_the_master_and_limits_the_total_mix_once() {
    for count in 0..=3 {
        let script = final_mix_script(count, 5.5);
        assert!(
                script.contains(
                    "apad=whole_dur=5.5,atrim=duration=5.5,alimiter=limit=0.95,aformat=sample_fmts=s16[aout]"
                ),
                "{script}"
            );
        assert_eq!(script.matches("alimiter").count(), 1, "{script}");
        // The s16 conversion belongs to the encode boundary only — the
        // stems themselves stay lossless float.
        assert_eq!(script.matches("aformat").count(), 1, "{script}");
    }
    assert!(
        final_mix_script(2, 5.5).contains("amix=inputs=2"),
        "{}",
        final_mix_script(2, 5.5)
    );
    // A plan with no audible events renders pure silence, still pinned.
    assert!(final_mix_script(0, 5.5).contains(&silence_bed_filter(5.5)));
}
