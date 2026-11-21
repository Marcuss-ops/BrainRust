use super::*;

fn cue(start_ms: i64, end_ms: i64, text: &str) -> TimedCue {
    TimedCue {
        start_ms,
        end_ms,
        text: text.to_string(),
    }
}

fn words_of(cues: &[TimedCue]) -> Vec<String> {
    cues.iter()
        .flat_map(|c| {
            c.text
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn assert_readable(policy: ShortFormPolicy, out: &[TimedCue]) {
    for (i, c) in out.iter().enumerate() {
        let dur = c.end_ms - c.start_ms;
        assert!(dur > 0, "caption {i} has an empty window: {c:?}");
        let lines = wrapped_lines(&c.text, policy.max_chars_per_line);
        assert!(
            lines <= policy.max_lines,
            "caption {i} wraps to {lines} lines: {c:?}"
        );
        let chars = c.text.chars().count();
        if chars > policy.max_chars_per_line {
            assert!(
                dur <= policy.reading_ms(chars),
                "caption {i} dwells {dur}ms for {chars} chars: {c:?}"
            );
        }
        if i > 0 {
            assert!(
                c.start_ms >= out[i - 1].end_ms,
                "captions {}/{} overlap",
                i - 1,
                i
            );
        }
    }
}

#[test]
fn splits_long_whisper_segment() {
    let source = vec![cue(
            13040,
            20480,
            "this is fine and I did it and but I was in Dubai in 2004 and went to the top of this building",
        )];
    let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
    assert!(
        out.len() >= 2,
        "long segment produced {} captions",
        out.len()
    );
    assert_readable(ShortFormPolicy::default(), &out);
    for c in &out {
        assert!(
            c.start_ms >= 13040 && c.end_ms <= 20480,
            "escaped window: {c:?}"
        );
    }
    assert_eq!(
        words_of(&out).join(" "),
        source[0]
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    );
}

#[test]
fn preserves_readable_transcript() {
    let source = vec![
        cue(0, 2000, "Okay, no."),
        cue(2000, 4000, "I'm on the go."),
        cue(4000, 5000, "How does that even work?"),
    ];
    let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
    assert_eq!(out, source);
}

#[test]
fn closes_sub_second_gap() {
    let source = vec![
        cue(0, 3000, "first caption here"),
        cue(3200, 6000, "second caption here"),
    ];
    let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].end_ms, out[1].start_ms);
}

#[test]
fn keeps_real_silence() {
    let source = vec![
        cue(0, 3000, "first caption here"),
        cue(6200, 9000, "second caption here"),
    ];
    let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
    assert_eq!(out.len(), 2);
    assert!(
        out[0].end_ms < out[1].start_ms,
        "a 3.2s silence must stay silent"
    );
}

#[test]
fn real_whisper_segments_stay_readable() {
    let source = vec![
            cue(0, 4800, "Where he can tell what colored underwear a man is wearing just from just from looking at"),
            cue(5440, 12920, "You do not disappoint. Oh, man. Listen, and this is any scraping the surface, buddy. This is okay"),
            cue(16120, 18800, "Should I do me to do me to do it to you?"),
            cue(18800, 23720, "I have so many I have so many just you facing the wrong way on a toilet"),
            cue(23720, 30799, "You just naked in an empty bathtub. It's great. I am just just the water way to start the day the water slowly running around"),
        ];
    let policy = ShortFormPolicy::default();
    let out = normalize_short_form_cues(&source, policy);
    assert!(
        out.len() >= source.len() * 2,
        "got {} captions from {} segments",
        out.len(),
        source.len()
    );
    assert_readable(policy, &out);
    assert_eq!(words_of(&out).join(" "), words_of(&source).join(" "));
}

#[test]
fn normalization_is_deterministic() {
    let source = vec![
        cue(
            0,
            5200,
            "where he can tell what colored underwear a man is wearing just from looking at him",
        ),
        cue(
            5440,
            12920,
            "you do not disappoint oh man listen and this is any scraping the surface",
        ),
    ];
    let first = normalize_short_form_cues(&source, ShortFormPolicy::default());
    let second = normalize_short_form_cues(&source, ShortFormPolicy::default());
    assert_eq!(first, second);
}

#[test]
fn merges_sub_second_caption() {
    let source = vec![cue(0, 1000, "first"), cue(1100, 1300, "flash")];
    let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
    assert_eq!(out.len(), 1, "expected merge: {out:?}");
    assert_eq!(out[0].text, "first flash");
    assert_eq!((out[0].start_ms, out[0].end_ms), (0, 1300));
}

#[test]
fn merged_captions_stay_bounded() {
    let source = vec![
            cue(0, 1000, "first"),
            cue(1100, 1300, "flash"),
            cue(1300, 9000, "a much longer utterance that definitely splits into several captions because it keeps going"),
            cue(9100, 9200, "tiny"),
            cue(10000, 15000, "and then a closing line that also needs a couple of captions"),
        ];
    let policy = ShortFormPolicy::default();
    let out = normalize_short_form_cues(&source, policy);
    assert!(!out.is_empty());
    assert_eq!(out[0].start_ms, source[0].start_ms);
    assert!(out.last().unwrap().end_ms <= source.last().unwrap().end_ms);
    for i in 1..out.len() {
        assert!(out[i].start_ms >= out[i - 1].end_ms);
    }
    assert_eq!(words_of(&out).join(" "), words_of(&source).join(" "));
    assert_readable(policy, &out);
}

#[test]
fn empty_input_yields_nothing() {
    assert!(normalize_short_form_cues(&[], ShortFormPolicy::default()).is_empty());
    let degenerate = vec![cue(0, 0, "x")];
    assert!(normalize_short_form_cues(&degenerate, ShortFormPolicy::default()).is_empty());
}

#[test]
fn distributes_text_across_timing() {
    let timing = vec![cue(0, 1000, ""), cue(1000, 2000, "")];
    let out = cues_with_text(&timing, "one two three four");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].text, "one two");
    assert_eq!(out[1].text, "three four");
    assert_eq!((out[1].start_ms, out[1].end_ms), (1000, 2000));
}

#[test]
fn distribution_never_loses_words() {
    let timing = vec![cue(0, 10, ""), cue(10, 20, ""), cue(20, 30, "")];
    let out = cues_with_text(&timing, "alpha beta gamma delta epsilon");
    assert_eq!(
        out.iter()
            .map(|c| c.text.split_whitespace().count())
            .sum::<usize>(),
        5
    );
    assert_eq!(
        out.iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        "alpha beta gamma delta epsilon"
    );
}

#[test]
fn ass_compilation_is_deterministic() {
    let cues = vec![cue(0, 3280, "hello"), cue(3280, 6000, "world")];
    let first = compile_ass_content(&cues, "shorts-v1").unwrap();
    let second = compile_ass_content(&cues, "shorts-v1").unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("[Script Info]"));
    assert!(first.contains("Style: shorts-v1,"));
    assert_eq!(first.matches("Dialogue:").count(), 2);
    assert!(first.contains("Dialogue: 0,0:00:00.00,0:00:03.28,shorts-v1,,0,0,0,,hello"));
}

#[test]
fn ass_style_changes_bytes() {
    let cues = vec![cue(0, 3280, "hello"), cue(3280, 6000, "world")];
    assert_ne!(
        compile_ass_content(&cues, "shorts-v1").unwrap(),
        compile_ass_content(&cues, "shorts-v2").unwrap()
    );
}

#[test]
fn ass_empty_cues_fail_closed() {
    assert!(compile_ass_content(&[], "shorts-v1").is_err());
}

#[test]
fn poppins_preset_has_stronger_shadow() {
    let content =
        compile_ass_content(&[cue(0, 3280, "hello")], "matt-damon-benchmark-v1-poppins").unwrap();
    assert!(content.contains("Style: matt-damon-benchmark-v1-poppins,Poppins,58,"));
    assert!(content.contains(",1,0,0,0,100,100,0,0,1,4.0,6,"));
    let poppins = resolve_font_preset("matt-damon-benchmark-v1-poppins");
    let montserrat = resolve_font_preset("shorts-v1");
    assert_eq!(poppins.font_name, "Poppins");
    assert!(poppins.shadow > montserrat.shadow);
}

#[test]
fn young_family_is_distinct_and_specific_first() {
    assert_eq!(resolve_font_preset("subs-young").font_size, 60);
    let pop = resolve_font_preset("subs-young-pop");
    assert_eq!((pop.font_size, pop.shadow, pop.margin_v), (64, 7, 44));
    let clean = resolve_font_preset("subs-young-clean");
    assert_eq!((clean.font_name, clean.font_size), ("Montserrat", 56));
    let center = resolve_font_preset("subs-young-center");
    assert_eq!(center.font_size, 60);
}

#[test]
fn center_style_uses_screen_center_alignment() {
    let content = compile_ass_content(&[cue(0, 1000, "hi")], "shorts-young-center").unwrap();
    assert!(content.contains(",1,4.0,5,5,10,10,40,1"), "{content}");
}

#[test]
fn ass_validator_accepts_generated_content() {
    let content = compile_ass_content(
        &[cue(0, 3280, "hello"), cue(3280, 6000, "world")],
        "shorts-v1",
    )
    .unwrap();
    assert!(validate_ass_content(&content, 6000).is_ok());
    assert!(validate_ass_content(&content, 1000).is_err());
    assert!(validate_ass_content("", 0).is_err());
}

#[test]
fn ass_wrap_respects_width_and_never_drops_text() {
    let text = "a very long subtitle line that must wrap into several pieces without losing words";
    let wrapped = wrap_asst_text(text, 20);
    for line in wrapped.split("\\N") {
        assert!(line.chars().count() <= 20, "line too long: {line:?}");
    }
    assert_eq!(wrapped.replace("\\N", " "), text);
}

#[test]
fn parses_vtt_cues() {
    let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nhello\nworld\n\n00:00:03.000 --> 00:00:04.000\nbye\n";
    let (plain, cues) = parse_subtitle_text("subs.vtt", vtt).unwrap();
    assert_eq!(cues.len(), 2);
    assert_eq!((cues[0].start_ms, cues[0].end_ms), (1000, 2500));
    assert_eq!(cues[0].text, "hello\nworld");
    assert_eq!(cues[1].text, "bye");
    assert_eq!(plain, "hello\nworld\nbye");
}

#[test]
fn parses_srt_cues() {
    let srt =
        "1\n00:00:01,000 --> 00:00:02,000\nfirst\n\n2\n00:00:03,000 --> 00:00:04,000\nsecond\n";
    let (plain, cues) = parse_subtitle_text("subs.srt", srt).unwrap();
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].start_ms, 1000);
    assert_eq!(cues[1].text, "second");
    assert_eq!(plain, "first\nsecond");
}

#[test]
fn parser_rejects_timestamps_go_would_not_match() {
    let malformed = "WEBVTT\n\n1:2:3.4-->5:6:7.8\nhello\n";
    assert!(parse_subtitle_text("bad.vtt", malformed).is_err());
    let no_millis = "WEBVTT\n\n00:00:01 --> 00:00:02\nhello\n";
    assert!(parse_subtitle_text("bad.vtt", no_millis).is_err());
}

#[test]
fn malformed_subtitle_file_fails_closed() {
    assert!(parse_subtitle_text("empty.vtt", "WEBVTT\n\n").is_err());
}
