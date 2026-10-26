//! Deterministic corpus behavior, size bands, modes and scored precision/recall.

use super::super::*;
use super::common::*;

#[test]
fn end_to_end_outputs_preserve_sentences_in_supported_languages() {
    let cases = [
        (
            "en",
            [
                "The company announced layoffs.",
                "Revenue increased this year.",
            ],
        ),
        (
            "it",
            [
                "L'azienda ha annunciato licenziamenti.",
                "I ricavi sono aumentati quest'anno.",
            ],
        ),
        (
            "es",
            [
                "La empresa anunció despidos.",
                "Los ingresos aumentaron este año.",
            ],
        ),
        (
            "pt",
            [
                "A empresa anunciou demissões.",
                "A receita aumentou este ano.",
            ],
        ),
        (
            "fr",
            [
                "L'entreprise a annoncé des licenciements.",
                "Les revenus ont augmenté cette année.",
            ],
        ),
        (
            "de",
            [
                "Das Unternehmen kündigte Entlassungen an.",
                "Der Umsatz stieg in diesem Jahr.",
            ],
        ),
    ];
    for (language, lines) in cases {
        let result = run(Request {
            semantic_bullet_spans: None,
            transcript: lines.join(" "),
            language: language.into(),
            embeddings: orthogonal_embeddings(lines.len()),
            timings: Vec::new(),
            options: Options {
                summary_length: SummaryLength::Short,
                min_heavy: 1,
                max_heavy: 2,
                top_fraction: Some(1.0),
                bullet_count: 2,
            },
            embedding_ms: 0.0,
            lexical_only: false,
            stopwords: lexicon_stopwords(language),
        })
        .unwrap();
        assert_eq!(result.ranked.len(), 2, "language: {language}");
        assert_eq!(result.heavy_sentences.len(), 2, "language: {language}");
        assert_eq!(result.bullet_points.len(), 2, "language: {language}");
        for line in lines {
            assert!(result.summary.contains(line), "language: {language}");
            assert!(
                result
                    .bullet_points
                    .iter()
                    .any(|bullet| bullet.text == line),
                "language: {language}"
            );
        }
    }
}

#[test]
fn deterministic_across_one_hundred_runs_and_invalid_vectors_fail_closed() {
    let lines = [
        "The board approved a major acquisition.",
        "Revenue grew by 35% this year.",
        "The company expects further growth.",
        "The board met again later.",
    ];
    let input = request(
        &lines,
        vec![
            vec3(1., 0., 0.),
            vec3(0.9, 0.1, 0.),
            vec3(0.95, 0.05, 0.),
            vec3(0.8, 0.2, 0.),
        ],
        Options::default(),
    );
    let first = run(input.clone()).unwrap();
    for _ in 0..99 {
        let next = run(input.clone()).unwrap();
        assert_eq!(next.summary, first.summary);
        assert_eq!(next.ranked, first.ranked);
        assert_eq!(next.heavy_sentences, first.heavy_sentences);
        assert_eq!(next.bullet_points, first.bullet_points);
        assert_eq!(next.timeline, first.timeline);
    }

    let invalid = request(&lines, vec![vec3(0., 0., 0.); 4], Options::default());
    assert!(run(invalid).unwrap_err().contains("zero norm"));

    let invalid_cases = [
        (
            Request {
                semantic_bullet_spans: None,
                transcript: "Sentence one. Sentence two.".into(),
                language: "en".into(),
                embeddings: vec![vec3(1., 0., 0.)],
                timings: Vec::new(),
                options: Options::default(),
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "embedding count",
        ),
        (
            Request {
                semantic_bullet_spans: None,
                transcript: "Sentence one.".into(),
                language: "en".into(),
                embeddings: vec![vec3(f32::NAN, 0., 0.)],
                timings: Vec::new(),
                options: Options::default(),
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "non-finite",
        ),
        (
            Request {
                semantic_bullet_spans: None,
                transcript: "Sentence one.".into(),
                language: "en".into(),
                embeddings: vec![vec3(1., 0., 0.)],
                timings: vec![Timing {
                    start_us: 2_000_000,
                    end_us: 1_000_000,
                }],
                options: Options::default(),
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "invalid/nonchronological",
        ),
        (
            Request {
                semantic_bullet_spans: None,
                transcript: "Sentence one.".into(),
                language: "en".into(),
                embeddings: vec![vec3(1., 0., 0.)],
                timings: Vec::new(),
                options: Options {
                    top_fraction: Some(1.1),
                    ..Options::default()
                },
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "top_fraction",
        ),
    ];
    for (invalid, expected_error) in invalid_cases {
        assert!(
            run(invalid).unwrap_err().contains(expected_error),
            "expected validation error containing {expected_error}"
        );
    }
}

#[test]
fn heavy_selection_obeys_fraction_and_minimum_maximum_for_transcript_sizes() {
    for (count, expected) in [(10, 3), (25, 4), (50, 7), (100, 13), (200, 15)] {
        let lines = labeled_test_sentences(count);
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let result = run(request(
            &borrowed,
            orthogonal_embeddings(count),
            Options {
                min_heavy: 3,
                max_heavy: 15,
                top_fraction: Some(0.125),
                ..Options::default()
            },
        ))
        .unwrap();
        assert_eq!(
            result.heavy_sentences.len(),
            expected,
            "unexpected selection count for {count} sentences"
        );
        assert!(result.heavy_sentences.len() <= 15);
        assert!(result
            .heavy_sentences
            .windows(2)
            .all(|pair| pair[0].importance >= pair[1].importance));
    }
}

#[test]
fn one_thousand_word_transcript_produces_stable_summary_bullets_and_top_ten() {
    let sentences: Vec<String> = (0..50)
        .map(|index| {
            format!("Sentence {index} company announced financial results and increased production across international markets this year during its latest quarterly operating period.")
        })
        .collect();
    assert_eq!(
        sentences.iter().map(|line| word_count(line)).sum::<usize>(),
        1_000
    );
    let borrowed: Vec<&str> = sentences.iter().map(String::as_str).collect();
    let input = request(
        &borrowed,
        orthogonal_embeddings(sentences.len()),
        Options {
            summary_length: SummaryLength::Short,
            bullet_count: 5,
            min_heavy: 10,
            max_heavy: 10,
            top_fraction: Some(0.1),
        },
    );
    let first = run(input.clone()).unwrap();
    assert_eq!(first.ranked.len(), 50);
    assert_eq!(first.heavy_sentences.len(), 10);
    assert_eq!(first.bullet_points.len(), 5);
    assert!(!first.summary.is_empty());
    assert!(first.summary.split_whitespace().count() <= 80);
    assert!(first
        .bullet_points
        .iter()
        .all(|bullet| sentences.iter().any(|line| line == &bullet.text)));
    assert!(first.timings.total_ms.is_finite());
    for _ in 0..9 {
        let next = run(input.clone()).unwrap();
        assert_eq!(next.summary, first.summary);
        assert_eq!(next.ranked, first.ranked);
        assert_eq!(next.heavy_sentences, first.heavy_sentences);
        assert_eq!(next.bullet_points, first.bullet_points);
        assert_eq!(next.timeline, first.timeline);
        assert!(next.timings.total_ms.is_finite());
    }
}

#[test]
fn bullet_count_modes_return_unique_source_sentences() {
    let lines = labeled_test_sentences(12);
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
    for count in [3, 5, 10] {
        let result = run(request(
            &borrowed,
            orthogonal_embeddings(lines.len()),
            Options {
                bullet_count: count,
                ..Options::default()
            },
        ))
        .unwrap();
        assert_eq!(result.bullet_points.len(), count);
        for (index, bullet) in result.bullet_points.iter().enumerate() {
            assert!(lines.iter().any(|line| line == &bullet.text));
            assert!(result.bullet_points[..index]
                .iter()
                .all(|prior| prior.text != bullet.text));
        }
    }
}

#[test]
fn summary_length_modes_stay_within_declared_word_bands_when_source_allows() {
    let lines = labeled_test_sentences(50);
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
    for (mode, minimum, maximum) in [
        (SummaryLength::Short, 50, 80),
        (SummaryLength::Medium, 100, 150),
        (SummaryLength::Long, 200, 300),
    ] {
        let result = run(request(
            &borrowed,
            orthogonal_embeddings(lines.len()),
            Options {
                summary_length: mode,
                ..Options::default()
            },
        ))
        .unwrap();
        let words = word_count(&result.summary);
        assert!(
            (minimum..=maximum).contains(&words),
            "summary has {words} words, expected {minimum}..={maximum}"
        );
        for sentence in result.summary.split_inclusive('.') {
            assert!(lines.iter().any(|line| line == sentence.trim()));
        }
    }
}

#[test]
fn synthetic_labeled_corpus_computes_precision_and_recall_transparently() {
    #[derive(Deserialize)]
    struct Corpus {
        dataset_type: String,
        label_semantics: String,
        embedding_note: String,
        cases: Vec<Case>,
    }
    #[derive(Deserialize)]
    struct Case {
        sentences: Vec<Label>,
    }
    #[derive(Deserialize)]
    struct Label {
        text: String,
        label: u8,
        #[serde(alias = "embedding_group")]
        _embedding_group: String,
    }
    let corpus: Corpus =
        serde_json::from_str(include_str!("../../fixtures/phrase_impact_labels.json")).unwrap();
    assert_eq!(
        corpus.dataset_type,
        "synthetic_developer_authored_not_human_evaluation"
    );
    assert!(corpus.label_semantics.contains("not human validation"));
    assert!(corpus.embedding_note.contains("not model embeddings"));
    assert_eq!(corpus.cases.len(), 33);
    let case_count = corpus.cases.len();
    let mut p5_hits = 0;
    let mut p10_hits = 0;
    let mut relevant = 0;
    let mut very_important = 0;
    let mut r10_hits = 0;
    for case in corpus.cases {
        let vectors = case
            .sentences
            .iter()
            .map(|sentence| lexical_vector(&sentence.text, &lexicon_stopword_set("en")))
            .map(|vector| vector.into_iter().map(|value| value as f32).collect())
            .collect();
        let text = case
            .sentences
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let result = run(Request {
            semantic_bullet_spans: None,
            transcript: text,
            language: "en".into(),
            embeddings: vectors,
            timings: Vec::new(),
            options: Options {
                min_heavy: 1,
                max_heavy: 12,
                ..Default::default()
            },
            embedding_ms: 0.0,
            lexical_only: false,
            stopwords: lexicon_stopwords("en"),
        })
        .unwrap();
        let labels: Vec<bool> = case
            .sentences
            .iter()
            .map(|sentence| {
                assert!(sentence.label <= 2);
                sentence.label > 0
            })
            .collect();
        assert_eq!(labels.len(), 12);
        assert!(labels.iter().filter(|&&label| label).count() >= 3);
        relevant += labels.iter().filter(|&&label| label).count();
        very_important += case
            .sentences
            .iter()
            .filter(|sentence| sentence.label == 2)
            .count();
        p5_hits += result
            .ranked
            .iter()
            .take(5)
            .filter(|sentence| labels[sentence.index])
            .count();
        p10_hits += result
            .ranked
            .iter()
            .take(10)
            .filter(|sentence| labels[sentence.index])
            .count();
        r10_hits += result
            .ranked
            .iter()
            .take(10)
            .filter(|sentence| labels[sentence.index])
            .count();
    }
    assert!(very_important > 0, "fixture must include label 2");
    let p5 = p5_hits as f64 / (case_count * 5) as f64;
    let p10 = p10_hits as f64 / (case_count * 10) as f64;
    let r10 = r10_hits as f64 / relevant as f64;
    eprintln!(
        "synthetic fixture only; not human evidence: P@5={p5:.3}, P@10={p10:.3}, R@10={r10:.3}"
    );
    // These loose checks protect only the synthetic harness wiring. They
    // are not evidence for the requested human-judged Precision@K targets.
    assert!(p5 > 0.45, "synthetic regression P@5={p5:.3}");
    assert!(p10 > 0.25, "synthetic regression P@10={p10:.3}");
    assert!(r10 >= 0.75, "synthetic regression R@10={r10:.3}");
}
