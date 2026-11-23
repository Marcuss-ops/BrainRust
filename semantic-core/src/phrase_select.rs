//! Deterministic important-phrase selection.
//!
//! This is the Rust owner for the selector previously implemented in
//! `internal/capabilities/scripts/phrases`. It is a pure function of the scene
//! text, the caller's blocked rune spans, an optional lexicon profile and the
//! document corpus. No model, network or wall-clock input is read.
//!
//! The algorithm is intentionally a line-by-line port rather than a new
//! ranking policy: short/long overlay budgets and operator hints depend on the
//! exact order produced here.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

mod scoring;
mod selection;
mod tokenize;

use scoring::{is_function_word, resolved_limit, resolved_policy};
use selection::{fragment_candidates, select_ranked, selected_phrase, sentence_candidates};
use tokenize::tokenize;

/// Phrase policy values mirroring the Go `PhraseExtractionPolicy`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhraseExtractionPolicy {
    pub min_words: usize,
    pub max_words: usize,
    pub max_results: usize,
    pub reject_verbs_when_all: bool,
}

impl Default for PhraseExtractionPolicy {
    fn default() -> Self {
        Self {
            min_words: 2,
            max_words: 16,
            max_results: 5,
            reject_verbs_when_all: true,
        }
    }
}

/// Lexicon data the selector needs. The caller owns loading the profile; the
/// decision (candidate generation, scoring, ordering) is made here.
#[derive(Clone, Debug, Default)]
pub struct PhraseProfile {
    pub stop_words: HashSet<String>,
    pub function_words: HashSet<String>,
    pub visual_verbs: HashSet<String>,
    pub policy: PhraseExtractionPolicy,
}

/// One selected phrase with byte and rune coordinates into the trimmed text.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SelectedPhrase {
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_rune: usize,
    pub end_rune: usize,
    pub score: i64,
}

/// Selects editorially important phrases in deterministic order.
///
/// `min_words`/`max_words` are the optional explicit lane override (0 means
/// "use the profile policy"), matching `SelectWithCorpusWordRange` in Go.
/// `blocked_rune_spans` are rune ranges the caller has already grounded as
/// entities; no selected phrase may overlap them.
pub fn select_phrases(
    text: &str,
    blocked_rune_spans: &[(usize, usize)],
    limit: usize,
    profile: Option<&PhraseProfile>,
    corpus: &[String],
    min_words: usize,
    max_words: usize,
) -> Vec<SelectedPhrase> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let Some(policy) = resolved_policy(profile, min_words, max_words) else {
        return Vec::new();
    };
    let limit = resolved_limit(limit, &policy);
    let tokens = tokenize(text);
    if tokens.len() < policy.min_words {
        return Vec::new();
    }
    let document_counts =
        scoring::document_phrase_counts(corpus, policy.min_words, policy.max_words);
    let context = selection::SelectionContext {
        profile,
        policy: &policy,
        min_words,
        max_words,
        blocked_rune_spans,
        document_counts: &document_counts,
    };
    let mut candidates = fragment_candidates(text, &tokens, &context);
    candidates.extend(sentence_candidates(text, &tokens, &context));
    if candidates.is_empty() {
        return Vec::new();
    }
    select_ranked(candidates, limit)
        .into_iter()
        .map(|candidate| selected_phrase(text, candidate))
        .collect()
}

/// Extracts important words from SELECTED phrases, in phrase order. Words come
/// from the exact selected surfaces, so a word overlay cannot show a token the
/// phrase overlay did not cover.
pub fn important_words(
    phrases: &[String],
    limit: usize,
    profile: Option<&PhraseProfile>,
) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let mut words = Vec::with_capacity(limit);
    let mut seen = HashSet::with_capacity(limit);
    for phrase in phrases {
        for token in tokenize(phrase) {
            let word = token.lower;
            if word.chars().count() < 3 || is_function_word(&word, profile) {
                continue;
            }
            if !seen.insert(word.clone()) {
                continue;
            }
            words.push(phrase[token.start..token.end].to_string());
            if words.len() == limit {
                return words;
            }
        }
    }
    words
}

/// True when `value` carries two consecutive capitalised words. All-uppercase
/// tokens of two or more letters are acronyms and do not start a name run.
pub fn contains_proper_name_pair(value: &str) -> bool {
    let mut previous_title = false;
    for raw in value.split_whitespace() {
        let word = raw.trim_matches(|c| {
            matches!(
                c,
                '.' | ','
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '"'
                    | '\''
                    | '’'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
            )
        });
        let current_title =
            !is_acronym_token(word) && word.chars().next().is_some_and(|c| c.is_uppercase());
        if current_title && previous_title {
            return true;
        }
        previous_title = current_title;
    }
    false
}

fn is_acronym_token(word: &str) -> bool {
    let mut letters = 0usize;
    for c in word.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        if !c.is_uppercase() {
            return false;
        }
        letters += 1;
    }
    letters >= 2
}

#[cfg(test)]
mod tests;
