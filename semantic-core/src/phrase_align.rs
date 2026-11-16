//! Deterministic phrase-to-word-timing alignment.
//!
//! This is the Rust owner for the matcher previously implemented in
//! `internal/capabilities/audio/phrase_locator.go`. It is a pure function of
//! the certified word boundaries and the requested phrase: Unicode NFC, case
//! folding, apostrophe folding, punctuation stripped around tokens, dash
//! splitting and non-overlapping document-order occurrences.
//!
//! The caller projects the returned word indices and microsecond anchors. No
//! timestamp is interpolated: a phrase that does not align fails closed.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// One certified word boundary, in the same order as the speech artifact.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct SpeechWord {
    pub index: i64,
    pub text: String,
    pub start_us: i64,
    pub end_us: i64,
}

/// One exact, non-overlapping occurrence of a phrase in the word timing.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct LocatedPhrase {
    pub text: String,
    pub occurrence: usize,
    pub word_start: i64,
    pub word_end: i64,
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlignError {
    PhraseNotFound(String),
}

impl std::fmt::Display for AlignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PhraseNotFound(detail) => {
                write!(f, "phrase not found in speech timing: {detail}")
            }
        }
    }
}

impl std::error::Error for AlignError {}

/// Finds every exact contiguous occurrence of `phrase` in the certified word
/// timing. Scanning is non-overlapping and proceeds in document order.
pub fn locate_phrase(words: &[SpeechWord], phrase: &str) -> Result<Vec<LocatedPhrase>, AlignError> {
    let query = phrase_tokens(phrase);
    if query.is_empty() {
        return Err(AlignError::PhraseNotFound("empty phrase".to_string()));
    }

    // Dash-joined script words align with separate TTS word boundaries, so
    // flatten each boundary through the same tokenizer as the phrase side.
    let mut normalized = Vec::new();
    let mut source_word = Vec::new();
    for (word_index, word) in words.iter().enumerate() {
        for token in fragment_tokens(&word.text) {
            normalized.push(token);
            source_word.push(word_index);
        }
    }
    if query.len() > normalized.len() {
        return Err(AlignError::PhraseNotFound(format!(
            "phrase has {} tokens but timing has {} word tokens",
            query.len(),
            normalized.len()
        )));
    }

    let mut found = Vec::new();
    let mut start = 0;
    while start + query.len() <= normalized.len() {
        let matched = query
            .iter()
            .zip(normalized[start..start + query.len()].iter())
            .all(|(query_token, timing_token)| tokens_match(query_token, timing_token));
        if !matched {
            start += 1;
            continue;
        }
        let first = &words[source_word[start]];
        let last = &words[source_word[start + query.len() - 1]];
        found.push(LocatedPhrase {
            text: phrase.trim().to_string(),
            occurrence: found.len() + 1,
            word_start: first.index,
            word_end: last.index,
            start_us: first.start_us,
            end_us: last.end_us,
        });
        start += query.len();
    }
    if found.is_empty() {
        return Err(AlignError::PhraseNotFound(describe_mismatch(
            &query,
            &normalized,
        )));
    }
    Ok(found)
}

/// Equality is the rule. The single tolerated shape is an elided article in
/// front of a number ("all'18" timing vs "18" phrase); the digits must match.
fn tokens_match(query_token: &str, timing_token: &str) -> bool {
    if query_token == timing_token {
        return true;
    }
    if query_token.is_empty() || timing_token.is_empty() || !is_numeric_token(query_token) {
        return false;
    }
    match timing_token.strip_suffix(query_token) {
        Some(tail) if !tail.is_empty() && tail.ends_with('\'') => true,
        _ => false,
    }
}

fn is_numeric_token(token: &str) -> bool {
    let mut seen_dot = false;
    let mut seen_digit = false;
    for r in token.chars() {
        match r {
            '0'..='9' => seen_digit = true,
            '.' | ',' if !seen_dot => seen_dot = true,
            _ => return false,
        }
    }
    seen_digit
}

fn describe_mismatch(query: &[String], normalized: &[String]) -> String {
    let limit = query.len().min(normalized.len());
    for i in 0..limit {
        if query[i] == normalized[i] {
            continue;
        }
        let lo = i.saturating_sub(2);
        let hi = (i + 3).min(query.len());
        let nhi = (i + 3).min(normalized.len());
        return format!(
            "first mismatch at token {i}: phrase[{}] vs timing[{}]",
            query[lo..hi].join(" "),
            normalized[lo..nhi].join(" ")
        );
    }
    format!(
        "query={} tokens, timing={} tokens (shared prefix, no full alignment)",
        query.len(),
        normalized.len()
    )
}

fn phrase_tokens(phrase: &str) -> Vec<String> {
    let mut out = Vec::new();
    for field in phrase.split_whitespace() {
        out.extend(fragment_tokens(field));
    }
    out
}

fn fragment_tokens(field: &str) -> Vec<String> {
    field
        .split(|r: char| r.is_whitespace() || is_dash_separator(r))
        .filter_map(normalize_token)
        .collect()
}

fn normalize_token(fragment: &str) -> Option<String> {
    let nfc: String = fragment.nfc().collect();
    let mut folded = String::with_capacity(nfc.len());
    for r in nfc.chars().flat_map(char::to_lowercase) {
        folded.push(fold_apostrophe(r));
    }
    let trimmed = folded.trim_matches(is_unicode_punctuation);
    let trimmed = trimmed.trim_end_matches('\'');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn fold_apostrophe(r: char) -> char {
    match r {
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{02BC}' => '\'',
        other => other,
    }
}

fn is_dash_separator(r: char) -> bool {
    matches!(
        r,
        '-' | '\u{2010}'
            | '\u{2011}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{2015}'
            | '\u{2212}'
    )
}

/// Unicode punctuation used by the canonical token edge trim. Rust's std does
/// not expose a stable Unicode general-category predicate, so this covers the
/// ASCII/Latin/GPP/CJK punctuation ranges the pipeline actually receives.
fn is_unicode_punctuation(r: char) -> bool {
    matches!(
        r,
        '!'..='#'
            | '%'..='*'
            | ','..='/'
            | ':'..='@'
            | '['..=']'
            | '_'
            | '{'..='~'
            | '\u{00A1}'
            | '\u{00AB}'
            | '\u{00B7}'
            | '\u{00BB}'
            | '\u{00BF}'
            | '\u{2010}'..='\u{2027}'
            | '\u{2030}'..='\u{205E}'
            | '\u{3001}'..='\u{303F}'
            | '\u{FF01}'..='\u{FF0F}'
            | '\u{FF1A}'..='\u{FF20}'
            | '\u{FF3B}'..='\u{FF40}'
            | '\u{FF5B}'..='\u{FF65}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(items: &[(&str, i64, i64)]) -> Vec<SpeechWord> {
        items
            .iter()
            .enumerate()
            .map(|(index, (text, start, end))| SpeechWord {
                index: index as i64,
                text: (*text).to_string(),
                start_us: *start,
                end_us: *end,
            })
            .collect()
    }

    #[test]
    fn aligns_exact_phrase_with_punctuation_and_case() {
        let timing = words(&[
            ("La", 0, 100),
            ("pena,", 100, 200),
            ("divenne", 200, 300),
            ("definitiva.", 300, 450),
        ]);
        let found = locate_phrase(&timing, "la pena divenne definitiva").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].word_start, 0);
        assert_eq!(found[0].word_end, 3);
        assert_eq!(found[0].start_us, 0);
        assert_eq!(found[0].end_us, 450);
        assert_eq!(found[0].occurrence, 1);
    }

    #[test]
    fn numbers_every_non_overlapping_occurrence() {
        let timing = words(&[
            ("go", 0, 100),
            ("go", 100, 200),
            ("stop", 200, 300),
            ("go", 300, 400),
        ]);
        let found = locate_phrase(&timing, "go").unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[1].occurrence, 2);
        assert_eq!(found[2].word_start, 3);
    }

    #[test]
    fn splits_dash_joined_words_and_collapses_whitespace() {
        let timing = words(&[("interpretation—in", 0, 200), ("practice", 200, 300)]);
        let found = locate_phrase(&timing, "interpretation in practice").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].word_start, 0);
        assert_eq!(found[0].word_end, 1);
    }

    #[test]
    fn matches_elided_article_before_number() {
        let timing = words(&[("all'18", 0, 200), ("percento", 200, 300)]);
        let found = locate_phrase(&timing, "18 percento").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].start_us, 0);
    }

    #[test]
    fn folds_typographic_apostrophes_and_accents() {
        let timing = words(&[("l’Italia", 0, 100), ("è", 100, 200), ("unità", 200, 300)]);
        let found = locate_phrase(&timing, "l'italia è unità").unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn fails_closed_when_interior_word_is_missing() {
        let timing = words(&[("one", 0, 100), ("three", 100, 200)]);
        let err = locate_phrase(&timing, "one two three").unwrap_err();
        assert!(matches!(err, AlignError::PhraseNotFound(_)));
    }

    #[test]
    fn fails_closed_on_empty_phrase_and_empty_timing() {
        assert!(locate_phrase(&[], "anything").is_err());
        assert!(locate_phrase(&words(&[("one", 0, 100)]), "   ").is_err());
    }

    #[test]
    fn reports_first_divergent_token_without_dumping_arrays() {
        let timing = words(&[("alpha", 0, 100), ("delta", 100, 200), ("beta", 200, 300)]);
        let err = locate_phrase(&timing, "alpha gamma beta").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("first mismatch at token 1"));
        assert!(text.contains("gamma"));
    }
}
