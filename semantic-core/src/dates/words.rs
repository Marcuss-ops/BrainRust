//! Word-level date evidence that is not a month token: spelled-number spans
//! (ICU spellout), the closed-class connectors that may join date parts, and
//! standalone spelled years.

use super::Era::Ce;
use super::{RawFact, SpelledNumber};

/// True for a word that can sit between the parts of a written date
/// ("12 de março", "marzo del 2008"). It must be a closed-class word of the
/// document language, so no per-language connector table lives here.
fn is_date_connector(word: &str, language: &str) -> bool {
    let lower = word.to_lowercase();
    crate::language_packs::get(language).is_some_and(|pack| pack.is_stopword(&lower))
}

/// Start offset of a date connector immediately before `end`, if any.
pub(super) fn connector_before(source: &str, end: usize, language: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut word_end = end;
    while word_end > 0 && bytes[word_end - 1].is_ascii_whitespace() {
        word_end -= 1;
    }
    let mut word_start = word_end;
    while word_start > 0
        && (bytes[word_start - 1].is_ascii_alphabetic() || bytes[word_start - 1] == b'\'')
    {
        word_start -= 1;
    }
    if word_start == word_end || !is_date_connector(&source[word_start..word_end], language) {
        return None;
    }
    bytes
        .get(word_start.wrapping_sub(1))
        .is_some_and(|byte| byte.is_ascii_whitespace())
        .then_some(word_start)
}

/// Cursor after at most two date connectors starting at `cursor`.
pub(super) fn skip_connectors_after(source: &str, position: usize, language: &str) -> usize {
    let bytes = source.as_bytes();
    let mut position = position;
    for _ in 0..2 {
        let mut start = position;
        while start < bytes.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphabetic() || bytes[end] == b'\'') {
            end += 1;
        }
        if end == start || !is_date_connector(&source[start..end], language) {
            break;
        }
        position = end;
    }
    position
}
/// `(value, start)` for a spelled number immediately before `position`,
/// skipping whitespace. The spans come from the locale's ICU spellout rules,
/// so no language table lives in this module.
pub(super) fn spelled_number_before(
    source: &str,
    words: &[SpelledNumber],
    position: usize,
    min: i64,
    max: i64,
) -> Option<(u32, usize)> {
    let word = words
        .iter()
        .filter(|word| word.end <= position && (min..=max).contains(&word.value))
        .max_by_key(|word| word.end)?;
    if !gap_is_whitespace(source, word.end, position) {
        return None;
    }
    Some((word.value as u32, word.start))
}

/// `(value, end)` for a spelled number immediately after `position`.
pub(super) fn spelled_number_after(
    source: &str,
    words: &[SpelledNumber],
    position: usize,
    min: i64,
    max: i64,
) -> Option<(i64, usize)> {
    let word = words
        .iter()
        .filter(|word| word.start >= position && (min..=max).contains(&word.value))
        .min_by_key(|word| word.start)?;
    if !gap_is_whitespace(source, position, word.start) {
        return None;
    }
    Some((word.value, word.end))
}

fn gap_is_whitespace(source: &str, start: usize, end: usize) -> bool {
    source
        .get(start..end)
        .is_some_and(|gap| gap.chars().all(char::is_whitespace))
}

/// Standalone spelled years ("duemilaventicinque", "due mila sette") not
/// attached to a month. Nested year spans are dropped by `resolve` when a
/// wider day-month-year fact covers them.
pub(super) fn collect_spelled_year_facts(source: &str, words: &[SpelledNumber]) -> Vec<RawFact> {
    words
        .iter()
        .filter(|word| (1000..=2099).contains(&word.value))
        .filter(|word| {
            source
                .get(word.start..word.end)
                .is_some_and(|surface| !surface.is_empty())
        })
        .map(|word| RawFact::bare(word.start, word.end, word.value as i32, Ce))
        .collect()
}
