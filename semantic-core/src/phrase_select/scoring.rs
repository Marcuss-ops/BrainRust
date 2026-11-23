//! Policy, word-class and corpus-frequency helpers for the phrase selector.

use std::collections::HashMap;

use super::tokenize::tokenize;
use super::{PhraseExtractionPolicy, PhraseProfile};

/// Resolves the effective policy: the profile's values, the 2..=16 fragment
/// cap, and then the caller's explicit lane override. `None` means the
/// override inverted the window, so no candidate can satisfy it.
pub(super) fn resolved_policy(
    profile: Option<&PhraseProfile>,
    min_words: usize,
    max_words: usize,
) -> Option<PhraseExtractionPolicy> {
    let mut policy = profile
        .map(|profile| profile.policy.clone())
        .unwrap_or_default();
    if policy.min_words < 2 {
        policy.min_words = 2;
    }
    if policy.max_words < policy.min_words {
        policy.max_words = policy.min_words;
    }
    // Arbitrary fragments stay headline-sized; complete sentences have their
    // own path below.
    if policy.max_words > 16 {
        policy.max_words = 16;
    }
    if min_words > 0 {
        policy.min_words = min_words;
    }
    if max_words > 0 {
        policy.max_words = max_words;
    }
    (policy.max_words >= policy.min_words).then_some(policy)
}

/// The result ceiling: the caller's limit, else the policy's, never zero and
/// never above the policy's own maximum.
pub(super) fn resolved_limit(limit: usize, policy: &PhraseExtractionPolicy) -> usize {
    let mut limit = if limit == 0 {
        policy.max_results
    } else {
        limit
    };
    if limit == 0 {
        limit = 3;
    }
    if policy.max_results > 0 && limit > policy.max_results {
        limit = policy.max_results;
    }
    limit
}

pub(super) fn is_function_word(word: &str, profile: Option<&PhraseProfile>) -> bool {
    profile.is_some_and(|profile| {
        profile.stop_words.contains(word) || profile.function_words.contains(word)
    })
}

pub(super) fn is_visual_verb(word: &str, profile: Option<&PhraseProfile>) -> bool {
    profile.is_some_and(|profile| profile.visual_verbs.contains(word))
}

pub(super) fn normalized_phrase_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(super) fn document_phrase_counts(
    corpus: &[String],
    min_words: usize,
    max_words: usize,
) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    if corpus.is_empty() {
        return counts;
    }
    for document in corpus {
        let tokens = tokenize(document);
        for start in 0..tokens.len() {
            for word_count in min_words..=max_words {
                let Some(end) = start.checked_add(word_count - 1) else {
                    break;
                };
                if end >= tokens.len() || tokens[start].group != tokens[end].group {
                    break;
                }
                *counts
                    .entry(normalized_phrase_key(
                        &document[tokens[start].start..tokens[end].end],
                    ))
                    .or_insert(0) += 1;
            }
        }
    }
    counts
}

pub(super) fn document_term_boost(occurrences: usize) -> i64 {
    if occurrences < 2 {
        return 0;
    }
    (4.0 * (1.0 + occurrences as f64).ln()).round() as i64
}

pub(super) fn overlaps_any_rune_span(start: usize, end: usize, spans: &[(usize, usize)]) -> bool {
    spans
        .iter()
        .any(|&(span_start, span_end)| start < span_end && span_start < end)
}

pub(super) fn starts_at_phrase_boundary(text: &str, byte_start: usize) -> bool {
    if byte_start == 0 || byte_start > text.len() {
        return true;
    }
    let prefix = text[..byte_start].trim_end();
    let Some(last) = prefix.chars().next_back() else {
        return true;
    };
    matches!(
        last,
        '.' | '!' | '?' | ';' | ',' | ':' | '—' | '–' | '\n' | '\r'
    )
}

pub(super) fn starts_with_interior_capital(text: &str, byte_start: usize) -> bool {
    if byte_start == 0 || byte_start > text.len() {
        return false;
    }
    let previous = text[..byte_start].trim();
    previous.ends_with(',')
}
