//! Sentence ranking, event cues, de-duplication and ordered output contracts.

use super::super::*;
use super::common::*;

#[test]
fn ranked_timing_and_content_fields_match_the_public_json_contract() {
    let lines = [
        "Apple reported record revenue.",
        "20,000 workers were laid off.",
    ];
    let result = run(request(
        &lines,
        vec![vec3(1., 0., 0.), vec3(0., 1., 0.)],
        Options {
            min_heavy: 2,
            max_heavy: 2,
            ..Default::default()
        },
    ))
    .unwrap();
    let json = serde_json::to_value(result).unwrap();
    assert!(json["ranked"][0]["index"].is_number());
    assert!(json["ranked"][0]["start"].is_null());
    assert!(json["ranked"][0]["importance"].is_number());
    assert!(json["ranked"][0]["centrality"].is_number());
    assert!(json["ranked"][0]["novelty"].is_number());
    assert!(json["ranked"][0]["text"].is_string());
    assert!(json["heavy_sentences"][0]["index"].is_number());
    assert!(json["heavy_sentences"][0]["importance"].is_number());
    assert!(json["heavy_sentences"][0]["centrality"].is_number());
    assert!(json["heavy_sentences"][0]["novelty"].is_number());
    assert!(json["heavy_sentences"][0]["text"].is_string());
    assert!(json["timeline"][0]["index"].is_number());
    assert!(json["bullet_points"][0]["sentence_index"].is_number());
    assert!(json["bullet_points"][0]["text"].is_string());
    assert!(json["summary"].is_string());
    assert!(json["bullet_points"].is_array());
    assert!(json["heavy_sentences"].is_array());
    assert!(json["timeline"].is_array());
    for stage in [
        "split_ms",
        "embedding_ms",
        "similarity_ms",
        "ranking_ms",
        "summary_ms",
        "bullet_ms",
        "total_ms",
    ] {
        assert!(json["timings"][stage].is_number(), "missing timing {stage}");
    }
}

#[test]
fn event_cues_outrank_numeric_only_and_filler_heavy_sentences() {
    assert_eq!(content_signal("The company announced revenue.", "en"), 1.0);
    assert_eq!(content_signal("The meeting had 20 attendees.", "en"), 0.25);
    for numeric_fact in [
        "Profits reached $14.8 billion.",
        "Revenue grew by 35%.",
        "20,000 workers were affected.",
    ] {
        assert_eq!(content_signal(numeric_fact, "en"), 0.25, "{numeric_fact}");
    }
    assert_eq!(
        content_signal("The first result in 30 years was announced in 2026.", "en"),
        1.0
    );
    let clean_score = apply_noise_penalty("The company announced layoffs.", 0.8);
    let filler_score =
        apply_noise_penalty("Well, you know, um, the company announced layoffs.", 0.8);
    assert!(filler_score < clean_score);

    let lines = [
        "Well, you know, the company basically, uh, reported revenue growth.",
        "But the really important thing is they announced 5,000 layoffs.",
        "Executives discussed the plan.",
    ];
    let result = run(request(
        &lines,
        vec![vec3(1., 0., 0.), vec3(1., 0., 0.), vec3(1., 0., 0.)],
        Options {
            min_heavy: 3,
            max_heavy: 3,
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(result.ranked[0].text, lines[1]);

    let event_lines = [
        "The company held a meeting.",
        "Employees discussed the new strategy.",
        "Then the CEO announced that 20,000 employees would be laid off.",
        "The announcement shocked investors.",
        "The meeting ended later that afternoon.",
    ];
    let event_result = run(request(
        &event_lines,
        vec![
            vec3(0., 0., 1.),
            vec3(0., 1., 0.),
            vec3(1., 0., 0.),
            vec3(0.99, 0.01, 0.),
            vec3(0., 0., 1.),
        ],
        Options {
            min_heavy: 2,
            max_heavy: 2,
            top_fraction: Some(0.4),
            ..Options::default()
        },
    ))
    .unwrap();
    assert!(event_result.heavy_sentences.iter().any(|sentence| {
        sentence.text == "Then the CEO announced that 20,000 employees would be laid off."
    }));
    assert!(event_result.ranked.iter().any(|sentence| {
        sentence.text == "The announcement shocked investors." && sentence.importance >= 0.9
    }));
}

#[test]
fn relevant_event_beats_novelty_outlier_and_duplicates_are_removed() {
    let lines = [
        "Apple reported record revenue.",
        "iPhone sales increased.",
        "The company expanded in Europe.",
        "My neighbor owns three cats.",
        "Apple expects more growth next year.",
    ];
    let result = run(request(
        &lines,
        vec![
            vec3(1.0, 0.0, 0.0),
            vec3(0.98, 0.1, 0.0),
            vec3(0.96, 0.2, 0.0),
            vec3(0.0, 1.0, 0.0),
            vec3(0.97, 0.15, 0.0),
        ],
        Options {
            min_heavy: 1,
            max_heavy: 5,
            top_fraction: Some(0.2),
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(!result
        .heavy_sentences
        .iter()
        .any(|sentence| sentence.text.contains("neighbor")));
    let duplicate_lines = [
        "Apple reported record revenue.",
        "Apple reported record revenue.",
        "Apple reported record revenue.",
        "Sales increased strongly.",
        "The company announced a major acquisition.",
    ];
    let duplicates = run(request(
        &duplicate_lines,
        vec![
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(0.9, 0.1, 0.),
            vec3(0., 1., 0.),
        ],
        Options {
            min_heavy: 1,
            max_heavy: 5,
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(
        duplicates
            .heavy_sentences
            .iter()
            .filter(|sentence| sentence.text == duplicate_lines[0])
            .count(),
        1
    );

    let paraphrases = [
        "Apple reported record revenue.",
        "Apple posted record revenue.",
        "Apple announced record sales.",
        "Sales increased strongly.",
    ];
    let semantic_duplicates = run(request(
        &paraphrases,
        vec![
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(0., 1., 0.),
        ],
        Options {
            min_heavy: 1,
            max_heavy: 4,
            top_fraction: Some(1.0),
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(
        semantic_duplicates
            .heavy_sentences
            .iter()
            .filter(|sentence| paraphrases[..3].contains(&sentence.text.as_str()))
            .count()
            <= 1
    );
}

#[test]
fn timeline_is_chronological_and_ranked_is_descending() {
    let lines = [
        "Sentence at time five.",
        "Sentence at time twelve.",
        "Sentence at time twenty.",
    ];
    // The fixture states its own lexical input: this ordering contract must not
    // depend on which per-language stop-word data happens to be installed. An
    // empty set keeps every fixture token a content token, so the three
    // sentences differ only in their one distinct word — the
    // equal-pairwise-similarity shape this check was written against.
    let fixture_stopwords: HashSet<String> = HashSet::new();
    let result = run(Request {
        semantic_bullet_spans: None,
        transcript: lines.join(" "),
        language: "en".into(),
        embeddings: lines
            .iter()
            .map(|line| lexical_vector(line, &fixture_stopwords))
            .map(|vector| vector.into_iter().map(|value| value as f32).collect())
            .collect(),
        stopwords: Vec::new(),
        timings: vec![
            Timing {
                start_us: 5_000_000,
                end_us: 6_000_000,
            },
            Timing {
                start_us: 12_000_000,
                end_us: 13_000_000,
            },
            Timing {
                start_us: 20_000_000,
                end_us: 21_000_000,
            },
        ],
        options: Options {
            min_heavy: 3,
            max_heavy: 3,
            ..Default::default()
        },
        embedding_ms: 5.0,
        lexical_only: false,
    })
    .unwrap();
    assert_eq!(result.ranked[0].index, 1);
    assert!(result
        .ranked
        .windows(2)
        .all(|pair| pair[0].importance >= pair[1].importance));
    assert_eq!(
        result
            .timeline
            .iter()
            .map(|sentence| sentence.start_sec.unwrap())
            .collect::<Vec<_>>(),
        [5., 12., 20.]
    );
    assert!((result.timings.total_ms - 5.0) >= result.timings.split_ms);
    assert!(
        result.timings.total_ms
            >= 5.0
                + result.timings.split_ms
                + result.timings.similarity_ms
                + result.timings.ranking_ms
                + result.timings.summary_ms
                + result.timings.bullet_ms
    );
}
