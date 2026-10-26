//! One bullet candidate generation and editorial ranking algorithm.
//!
//! Before this module existed, VisualNER generated one bullet per
//! entity-bearing sentence with a constant score, and Phrase Impact ranked its
//! own candidate pool with a separate importance graph. The two surfaces could
//! disagree about which sentence matters, so the semantic manifest and the
//! editorial bullets described the same script differently.
//!
//! Here there is exactly one definition of a bullet candidate (a source
//! sentence that contains at least one grounded signal) and one definition of
//! its editorial salience ([`score_sentence`]). VisualNER projects the
//! candidates into its own JSON type, the semantic engine carries the score on
//! every candidate, and Phrase Impact combines the same score with its
//! embedding importance through [`editorial_priority`]. No caller re-implements
//! the weight table.
//!
//! The score is deterministic and evidence-based. It is not a provider
//! confidence and it does not claim external truth: it ranks already-grounded
//! source sentences by how much visual/measurable content they carry.

use crate::sentences::{split_sentences, SentenceSpan};

/// A sentence shorter than this is a fragment, not a bullet.
pub const MIN_SUBSTANTIVE_WORDS: usize = 3;

/// What kind of grounded signal a mention provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// A picturable subject (person, organization, place, facility, object).
    Visual,
    /// A measurement (number, percentage, money, date).
    Measurement,
}

/// Maps an extracted semantic type to the signals that can make a sentence
/// an editorial bullet. Generic noun phrases remain available as phrase
/// inventory, but do not independently turn every prose sentence into a
/// bullet or an important segment.
pub fn signal_kind_for_entity_type(entity_type: &str) -> Option<SignalKind> {
    match entity_type {
        "PERSON" | "ORGANIZATION" | "ORG" | "LOCATION" | "FACILITY" | "BRAND" | "PRODUCT"
        | "WORK" | "OBJECT" | "STRUCTURE" | "EVENT" => Some(SignalKind::Visual),
        "NUMBER" | "PERCENT" | "MONEY" | "MEASUREMENT" | "DATE" => Some(SignalKind::Measurement),
        _ => None,
    }
}

/// One grounded mention inside the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulletSignal {
    pub start_byte: usize,
    pub end_byte: usize,
    pub kind: SignalKind,
}

/// A source sentence proposed as a bullet, with its editorial salience.
#[derive(Debug, Clone, PartialEq)]
pub struct BulletCandidate {
    /// Scene-local index assigned by the canonical segmenter.
    pub sentence_index: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    /// Exact source slice at `[start_byte, end_byte)`.
    pub text: String,
    /// Number of distinct visual mentions in the sentence.
    pub visual_mentions: usize,
    /// Number of distinct measurement mentions in the sentence.
    pub measurement_mentions: usize,
    /// Deterministic salience in `[0, 1]`.
    pub score: f32,
}

/// Generates candidates for `source`, splitting it with the canonical
/// segmenter.
pub fn generate(source: &str, language: &str, signals: &[BulletSignal]) -> Vec<BulletCandidate> {
    let sentences = split_sentences(source, language, None);
    generate_with_sentences(source, &sentences, signals)
}

/// Generates candidates over sentence spans the caller already owns (so a
/// caller that split once does not split twice).
pub fn generate_with_sentences(
    source: &str,
    sentences: &[SentenceSpan],
    signals: &[BulletSignal],
) -> Vec<BulletCandidate> {
    let mut out = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let mut visual = 0usize;
        let mut measurement = 0usize;
        let mut seen: Vec<(usize, usize, SignalKind)> = Vec::new();
        for signal in signals {
            if signal.start_byte < sentence.start_byte
                || signal.end_byte > sentence.end_byte
                || signal.start_byte >= signal.end_byte
            {
                continue;
            }
            let key = (signal.start_byte, signal.end_byte, signal.kind);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            match signal.kind {
                SignalKind::Visual => visual += 1,
                SignalKind::Measurement => measurement += 1,
            }
        }
        if visual + measurement == 0 {
            continue;
        }
        out.push(BulletCandidate {
            sentence_index,
            start_byte: sentence.start_byte,
            end_byte: sentence.end_byte,
            text: text.to_string(),
            visual_mentions: visual,
            measurement_mentions: measurement,
            score: score_sentence(text, visual, measurement),
        });
    }
    out
}

/// The single salience table for a grounded candidate sentence.
///
/// Visual subjects dominate (a bullet about people/places is more showable
/// than a bare statistic), measurements add evidence, and both fragments and
/// sprawling sentences are penalized. The result is clamped to `[0, 1]`.
pub fn score_sentence(text: &str, visual_mentions: usize, measurement_mentions: usize) -> f32 {
    let words = text.split_whitespace().count();
    let mut score = 0.20f32;
    score += 0.18 * visual_mentions.min(3) as f32;
    score += 0.10 * measurement_mentions.min(2) as f32;
    score += match words {
        0..=2 => -0.20,
        3..=5 => -0.05,
        6..=28 => 0.10,
        29..=45 => 0.02,
        _ => -0.10,
    };
    score.clamp(0.0, 1.0)
}

/// Editorial priority used by Phrase Impact when it ranks the semantic
/// candidate pool. Embedding importance stays the primary signal; the shared
/// semantic salience breaks ties and lifts evidence-rich sentences.
pub fn editorial_priority(importance: f64, semantic_score: f32) -> f64 {
    importance + 0.35 * f64::from(semantic_score)
}

/// A sentence is substantive enough to be published as a bullet.
pub fn is_substantive(text: &str) -> bool {
    text.split_whitespace().count() >= MIN_SUBSTANTIVE_WORDS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual(start: usize, end: usize) -> BulletSignal {
        BulletSignal {
            start_byte: start,
            end_byte: end,
            kind: SignalKind::Visual,
        }
    }

    fn measurement(start: usize, end: usize) -> BulletSignal {
        BulletSignal {
            start_byte: start,
            end_byte: end,
            kind: SignalKind::Measurement,
        }
    }

    #[test]
    fn candidates_require_a_grounded_signal_and_keep_exact_spans() {
        let source = "Lo Stadio Aurora apre nel 1959. Una frase senza segnali.";
        let signals = [visual(3, 17), measurement(23, 27)];
        let candidates = generate(source, "it", &signals);
        assert_eq!(candidates.len(), 1, "only the entity sentence is a bullet");
        let candidate = &candidates[0];
        assert_eq!(candidate.text, "Lo Stadio Aurora apre nel 1959.");
        assert_eq!(
            source.get(candidate.start_byte..candidate.end_byte),
            Some(candidate.text.as_str())
        );
        assert_eq!(candidate.visual_mentions, 1);
        assert_eq!(candidate.measurement_mentions, 1);
        assert!((0.0..=1.0).contains(&candidate.score));
    }

    #[test]
    fn evidence_rich_sentences_outrank_sparse_ones() {
        let source =
            "Torino. Lo Stadio Aurora di Torino ha 10.000 posti e appartiene alla famiglia Rossi.";
        let signals = [
            visual(0, 6),
            visual(8, 24),
            visual(28, 34),
            measurement(38, 44),
            visual(69, 77),
            visual(78, 83),
        ];
        let candidates = generate(source, "it", &signals);
        assert_eq!(candidates.len(), 2);
        assert!(
            candidates[1].score > candidates[0].score,
            "the evidence-rich sentence must win: {candidates:?}"
        );
        // A fragment is penalized but still a candidate when grounded.
        assert!(candidates[0].score < 0.5);
    }

    #[test]
    fn duplicate_signals_are_counted_once() {
        let source = "Il pallone è sul campo.";
        let signals = [visual(3, 10), visual(3, 10), visual(17, 22)];
        let candidates = generate(source, "it", &signals);
        assert_eq!(candidates[0].visual_mentions, 2);
    }

    #[test]
    fn editorial_priority_is_monotonic_in_the_shared_score() {
        let base = editorial_priority(0.5, 0.0);
        let lifted = editorial_priority(0.5, 0.8);
        assert!(lifted > base);
        assert!((lifted - base - 0.28).abs() < 1e-6);
    }

    #[test]
    fn sentence_without_signals_is_not_a_candidate() {
        let candidates = generate("Nessuna entità qui.", "it", &[]);
        assert!(candidates.is_empty());
    }
}
