//! Extractive summary bans: negation, polarity and faithful verbatim wording.

use super::super::*;
use super::common::*;

fn is_extractive(summary: &str, segments: &[Segment]) -> bool {
    let mut remaining = summary;
    let mut matched = 0;
    for segment in segments {
        let Some(tail) = remaining.strip_prefix(&segment.text) else {
            continue;
        };
        remaining = tail.strip_prefix(' ').unwrap_or(tail);
        matched += 1;
        if remaining.is_empty() {
            return matched > 0;
        }
    }
    false
}

#[test]
fn synthetic_transcript_summarization_keeps_negation_and_source_spans() {
    #[derive(Deserialize)]
    struct GroundTruth {
        sentences: Vec<LabeledSentence>,
    }
    #[derive(Deserialize)]
    struct LabeledSentence {
        id: String,
        text: String,
        label: String,
        #[serde(default)]
        must_preserve_negation: Option<String>,
    }

    let transcript = include_str!("../../fixtures/elon_musk_synthetic_transcript_it.txt");
    let truth: GroundTruth = serde_json::from_str(include_str!(
        "../../fixtures/elon_musk_synthetic_ground_truth.json"
    ))
    .unwrap();
    let result = run(Request {
        semantic_bullet_spans: None,
        transcript: transcript.to_string(),
        language: "it".into(),
        embeddings: Vec::new(),
        timings: Vec::new(),
        options: Options {
            summary_length: SummaryLength::Short,
            bullet_count: 10,
            min_heavy: 15,
            max_heavy: 15,
            top_fraction: Some(0.125),
        },
        embedding_ms: 0.0,
        lexical_only: true,
        stopwords: lexicon_stopwords("it"),
    })
    .unwrap();
    assert!(truth
        .sentences
        .iter()
        .all(|sentence| matches!(sentence.label.as_str(), "heavy" | "trap")));
    let segments = split_sentences(transcript, "it");
    let segment_texts: std::collections::HashSet<&str> = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    for sentence in &truth.sentences {
        assert!(
            segment_texts.contains(sentence.text.as_str()),
            "fixture sentence {} not found in source split",
            sentence.id
        );
    }
    let heavy_count = truth
        .sentences
        .iter()
        .filter(|sentence| sentence.label == "heavy")
        .count();
    assert_eq!(heavy_count, 9);
    let id_by_text: std::collections::HashMap<&str, &str> = truth
        .sentences
        .iter()
        .map(|sentence| (sentence.text.as_str(), sentence.id.as_str()))
        .collect();
    let heavy_ids: std::collections::HashSet<&str> = truth
        .sentences
        .iter()
        .filter(|sentence| sentence.label == "heavy")
        .map(|sentence| sentence.id.as_str())
        .collect();
    let mut ranking_metrics = Vec::new();
    for cutoff in [5, 10] {
        let top = result.ranked.iter().take(cutoff).collect::<Vec<_>>();
        let hits = top
            .iter()
            .filter(|sentence| {
                id_by_text
                    .get(sentence.text.as_str())
                    .is_some_and(|id| heavy_ids.contains(id))
            })
            .count();
        let precision = hits as f64 / top.len() as f64;
        assert!(precision.is_finite() && (0.0..=1.0).contains(&precision));
        ranking_metrics.push((cutoff, precision));
    }
    assert_eq!(ranking_metrics.len(), 2);
    let negation = truth
        .sentences
        .iter()
        .find_map(|sentence| sentence.must_preserve_negation.as_deref())
        .unwrap();
    let summary_contains_claim = result.summary.to_lowercase().contains("licenziamenti");
    let summary_preserves_negation = result
        .summary
        .to_lowercase()
        .contains(&negation.to_lowercase());
    if summary_contains_claim {
        assert!(
            summary_preserves_negation,
            "summary mentioned layoffs but dropped negation: {}",
            result.summary
        );
    }
    assert!(
        is_extractive(&result.summary, &segments),
        "summary must only contain complete source sentences: {}",
        result.summary
    );
    for bullet in &result.bullet_points {
        assert!(
            segments.iter().any(|segment| segment.text == bullet.text),
            "bullet must be a source sentence: {}",
            bullet.text
        );
    }
}

#[test]
fn summary_and_bullets_are_extractively_faithful_and_distinct_from_rank() {
    let lines = [
        "Tesla opened a new factory in Mexico.",
        "The factory will produce electric vehicles.",
        "Production is expected to begin next year.",
        "The project will employ 5,000 workers.",
    ];
    let result = run(request(
        &lines,
        vec![
            vec3(1., 0., 0.),
            vec3(0., 1., 0.),
            vec3(0., 0., 1.),
            vec3(-1., 0., 0.),
        ],
        Options {
            summary_length: SummaryLength::Short,
            min_heavy: 2,
            max_heavy: 2,
            bullet_count: 3,
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(result.summary.contains("factory") && result.summary.contains("Mexico"));
    assert!(!result.summary.to_lowercase().contains("largest"));
    assert_ne!(
        result.summary,
        result
            .heavy_sentences
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert!(result.bullet_points.len() <= 3);
    assert!(result
        .bullet_points
        .iter()
        .all(|bullet| lines.contains(&bullet.text.as_str())));
    let importance_by_index: Vec<f64> = (0..lines.len())
        .map(|index| {
            result
                .ranked
                .iter()
                .find(|sentence| sentence.index == index)
                .unwrap()
                .importance
        })
        .collect();
    assert!(result.bullet_points.windows(2).all(|pair| {
        importance_by_index[pair[0].sentence_index] >= importance_by_index[pair[1].sentence_index]
    }));
    let polarity = ["The company did not declare bankruptcy."];
    let result = run(request(
        &polarity,
        vec![vec3(1., 0., 0.)],
        Options {
            min_heavy: 1,
            max_heavy: 1,
            summary_length: SummaryLength::Short,
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(result.summary.contains("did not declare bankruptcy"));
}

#[test]
fn extractive_summary_keeps_entity_associations_and_sentence_polarity() {
    let lines = [
        "Elon Musk discussed Tesla during the interview.",
        "Tim Cook discussed Apple during the interview.",
        "The company did not declare bankruptcy.",
    ];
    let result = run(request(
        &lines,
        orthogonal_embeddings(lines.len()),
        Options {
            summary_length: SummaryLength::Short,
            min_heavy: 1,
            max_heavy: 3,
            bullet_count: 3,
            ..Options::default()
        },
    ))
    .unwrap();
    assert!(result
        .summary
        .split_inclusive('.')
        .all(|sentence| lines.contains(&sentence.trim())));
    assert!(result.summary.contains("Elon Musk discussed Tesla"));
    assert!(result.summary.contains("Tim Cook discussed Apple"));
    assert!(result.summary.contains("did not declare bankruptcy"));
    assert!(result
        .bullet_points
        .iter()
        .all(|bullet| lines.contains(&bullet.text.as_str())));
}

#[test]
fn statistical_sentence_is_not_a_title_but_stays_a_bullet() {
    assert!(is_statistical_sentence(
        "Sales rose 40% compared to last year across every region we track."
    ));
    assert!(!is_statistical_sentence("Formula 1"));
    assert!(!is_statistical_sentence("Industria 4.0"));
    assert!(!is_statistical_sentence("G7 summit"));
    assert!(!is_statistical_sentence("Windows 11"));
}
