//! Phrase Impact sentence projection.
//!
//! The segmenter itself is NOT owned here: `semantic-core::sentences` is the
//! one canonical implementation shared with VisualNER and Scene Intelligence.
//! This module only maps its byte spans onto [`Segment`] (which carries an
//! owned `text`) and applies Phrase Impact's embedding word bound.

use super::*;

pub fn split_sentences(text: &str, language: &str) -> Vec<Segment> {
    split_sentence_spans(text, language, Some(MAX_SEGMENT_WORDS))
}

/// Canonical sentence boundary detector shared with Scene Intelligence. The
/// optional word limit preserves phrase-impact's bounded embedding chunks
/// while letting SI retain its original whole-sentence granularity.
pub fn split_sentence_spans(text: &str, language: &str, max_words: Option<usize>) -> Vec<Segment> {
    crate::sentences::split_sentences(text, language, max_words)
        .into_iter()
        .map(|span| Segment {
            start_byte: span.start_byte,
            end_byte: span.end_byte,
            text: span.text(text).to_string(),
        })
        .collect()
}
