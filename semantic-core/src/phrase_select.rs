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

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug)]
struct Token {
    lower: String,
    start: usize,
    end: usize,
    group: usize,
}

#[derive(Clone, Debug)]
struct SentenceSpan {
    byte_start: usize,
    byte_end: usize,
    token_start: usize,
    token_end: usize,
}

#[derive(Clone, Debug)]
struct Candidate {
    text: String,
    start: usize,
    token_start: usize,
    token_end: usize,
    score: i64,
    sentence_count: usize,
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
    if policy.max_words < policy.min_words {
        return Vec::new();
    }

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

    let tokens = tokenize(text);
    if tokens.len() < policy.min_words {
        return Vec::new();
    }
    let document_counts = document_phrase_counts(corpus, policy.min_words, policy.max_words);
    let mut candidates = Vec::new();

    for start in 0..tokens.len() {
        for word_count in policy.min_words..=policy.max_words {
            let Some(end) = start.checked_add(word_count - 1) else {
                break;
            };
            if end >= tokens.len() {
                break;
            }
            if tokens[start].group != tokens[end].group {
                break;
            }
            let first = &tokens[start].lower;
            let last = &tokens[end].lower;
            let leading_function = is_function_word(first, profile);
            if is_function_word(last, profile)
                || (leading_function && !starts_at_phrase_boundary(text, tokens[start].start))
            {
                continue;
            }

            let mut content_words = 0usize;
            let mut visual_verbs = 0usize;
            let mut function_words = 0usize;
            let mut all_visual_verbs = true;
            for token in &tokens[start..=end] {
                let word = token.lower.as_str();
                if is_function_word(word, profile) {
                    function_words += 1;
                    continue;
                }
                if word.chars().count() < 3 {
                    continue;
                }
                content_words += 1;
                if is_visual_verb(word, profile) {
                    visual_verbs += 1;
                } else {
                    all_visual_verbs = false;
                }
            }
            if function_words * 3 > word_count
                || content_words < 2
                || (policy.reject_verbs_when_all && all_visual_verbs && content_words > 0)
            {
                continue;
            }
            if starts_with_interior_capital(text, tokens[start].start) {
                continue;
            }

            let byte_start = tokens[start].start;
            let byte_end = tokens[end].end;
            let candidate_text = &text[byte_start..byte_end];
            if contains_proper_name_pair(candidate_text) {
                continue;
            }
            let rune_start = text[..byte_start].chars().count();
            let rune_end = rune_start + candidate_text.chars().count();
            if overlaps_any_rune_span(rune_start, rune_end, blocked_rune_spans) {
                continue;
            }

            let mut score = content_words as i64 * 4 + visual_verbs as i64 * 5 - word_count as i64;
            if profile.is_some() {
                score += word_count as i64 * 2;
            }
            score += document_term_boost(
                document_counts
                    .get(&normalized_phrase_key(candidate_text))
                    .copied()
                    .unwrap_or(0),
            );
            candidates.push(Candidate {
                text: candidate_text.to_string(),
                start: byte_start,
                token_start: start,
                token_end: end,
                score,
                sentence_count: 0,
            });
        }
    }

    let sentences = sentence_token_spans(text, &tokens);
    for i in 0..sentences.len() {
        for width in 1..=2 {
            if i + width > sentences.len() {
                break;
            }
            let first = &sentences[i];
            let last = &sentences[i + width - 1];
            let word_count = last.token_end - first.token_start + 1;
            if (min_words > 0 && word_count < min_words)
                || (max_words > 0 && word_count > max_words)
            {
                continue;
            }
            if width == 2 && word_count <= policy.max_words {
                continue;
            }
            let span_text = text[first.byte_start..last.byte_end].trim().to_string();
            let mut span = Candidate {
                text: span_text,
                start: first.byte_start,
                token_start: first.token_start,
                token_end: last.token_end,
                score: 0,
                sentence_count: width,
            };
            let at_scene_end = width == 1 && i == sentences.len() - 1;
            let terminated = is_terminated_sentence_text(&span.text);
            if !terminated && !at_scene_end {
                continue;
            }
            if word_count > 30 {
                continue;
            }
            if !terminated && word_count > 16 {
                continue;
            }
            if sentence_candidate_allowed(&span, profile, blocked_rune_spans, text) {
                span.score = phrase_candidate_score(&span.text, profile, &document_counts);
                candidates.push(span);
            }
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }
    candidates.sort_by(|a, b| {
        let long_a = a.token_end - a.token_start + 1 >= 8;
        let long_b = b.token_end - b.token_start + 1 >= 8;
        long_b
            .cmp(&long_a)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| b.sentence_count.cmp(&a.sentence_count))
            .then_with(|| a.start.cmp(&b.start))
            .then_with(|| a.token_end.cmp(&b.token_end))
    });

    let mut selected = Vec::with_capacity(limit);
    let mut seen = HashSet::with_capacity(limit);
    for candidate in candidates {
        let key = normalized_phrase_key(&candidate.text);
        if !seen.insert(key) {
            continue;
        }
        if selected.iter().any(|prior: &Candidate| {
            candidate.token_start <= prior.token_end && prior.token_start <= candidate.token_end
        }) {
            continue;
        }
        selected.push(candidate);
        if selected.len() == limit {
            break;
        }
    }

    selected
        .into_iter()
        .map(|candidate| {
            let start_rune = text[..candidate.start].chars().count();
            let end_rune = start_rune + candidate.text.chars().count();
            SelectedPhrase {
                end_byte: candidate.start + candidate.text.len(),
                text: candidate.text,
                start_byte: candidate.start,
                start_rune,
                end_rune,
                score: candidate.score,
            }
        })
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

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut group = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let (start, c) = chars[i];
        if !is_word_char(c) {
            i += 1;
            continue;
        }
        let mut end = start + c.len_utf8();
        i += 1;
        loop {
            if i >= chars.len() {
                break;
            }
            let (pos, ch) = chars[i];
            if is_word_char(ch) {
                end = pos + ch.len_utf8();
                i += 1;
                continue;
            }
            if (ch == '\'' || ch == '’') && i + 1 < chars.len() && is_word_char(chars[i + 1].1) {
                end = pos + ch.len_utf8();
                i += 1;
                continue;
            }
            break;
        }
        if let Some(previous) = tokens.last() {
            if phrase_boundary_between(&text[previous.end..start]) {
                group += 1;
            }
        }
        tokens.push(Token {
            lower: text[start..end].to_lowercase(),
            start,
            end,
            group,
        });
    }
    tokens
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || is_combining_mark(c)
}

fn is_combining_mark(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

fn phrase_boundary_between(gap: &str) -> bool {
    gap.chars().any(|c| {
        matches!(
            c,
            '.' | '!' | '?' | ';' | ',' | ':' | '—' | '–' | '\n' | '\r'
        )
    })
}

fn sentence_boundary_between(gap: &str) -> bool {
    gap.chars()
        .any(|c| matches!(c, '.' | '!' | '?' | '\n' | '\r'))
}

fn sentence_token_spans(text: &str, tokens: &[Token]) -> Vec<SentenceSpan> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut sentences = Vec::new();
    let mut start = 0usize;
    for next in 1..tokens.len() {
        let gap_start = tokens[next - 1].end;
        let gap = &text[gap_start..tokens[next].start];
        if !sentence_boundary_between(gap) {
            continue;
        }
        sentences.push(SentenceSpan {
            byte_start: tokens[start].start,
            byte_end: sentence_boundary_end_byte(gap, gap_start),
            token_start: start,
            token_end: next - 1,
        });
        start = next;
    }
    let mut end_byte = text.len();
    while end_byte > tokens[tokens.len() - 1].end {
        let Some(c) = text[..end_byte].chars().next_back() else {
            break;
        };
        if !c.is_whitespace() {
            break;
        }
        end_byte -= c.len_utf8();
    }
    sentences.push(SentenceSpan {
        byte_start: tokens[start].start,
        byte_end: end_byte,
        token_start: start,
        token_end: tokens.len() - 1,
    });
    sentences
}

fn sentence_boundary_end_byte(gap: &str, absolute_gap_start: usize) -> usize {
    for (offset, c) in gap.char_indices() {
        if !matches!(c, '.' | '!' | '?' | '\n' | '\r') {
            continue;
        }
        let mut end = offset + c.len_utf8();
        while end < gap.len() {
            let Some(next) = gap[end..].chars().next() else {
                break;
            };
            if matches!(next, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»') {
                end += next.len_utf8();
                continue;
            }
            break;
        }
        return absolute_gap_start + end;
    }
    absolute_gap_start + gap.len()
}

fn is_terminated_sentence_text(value: &str) -> bool {
    let mut value = value.trim();
    loop {
        let Some(c) = value.chars().next_back() else {
            return false;
        };
        if matches!(c, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»') {
            value = value[..value.len() - c.len_utf8()].trim();
            continue;
        }
        return matches!(c, '.' | '!' | '?');
    }
}

fn sentence_candidate_allowed(
    candidate: &Candidate,
    profile: Option<&PhraseProfile>,
    blocked_rune_spans: &[(usize, usize)],
    text: &str,
) -> bool {
    let word_count = candidate.token_end - candidate.token_start + 1;
    if word_count < 2
        || candidate.text.trim().is_empty()
        || contains_proper_name_pair(&candidate.text)
    {
        return false;
    }
    let tokens = tokenize(&candidate.text);
    if tokens.is_empty() {
        return false;
    }
    let first = &tokens[0].lower;
    let last = &tokens[tokens.len() - 1].lower;
    let leading_function = is_function_word(first, profile);
    if is_function_word(last, profile)
        || (leading_function && !starts_at_phrase_boundary(text, candidate.start))
    {
        return false;
    }
    let mut content_words = 0usize;
    let mut function_words = 0usize;
    let mut all_visual_verbs = true;
    for token in &tokens {
        let word = token.lower.as_str();
        if is_function_word(word, profile) {
            function_words += 1;
            continue;
        }
        if word.chars().count() < 3 {
            continue;
        }
        content_words += 1;
        if !is_visual_verb(word, profile) {
            all_visual_verbs = false;
        }
    }
    if function_words * 3 > word_count || content_words < 2 {
        return false;
    }
    let policy = profile
        .map(|profile| profile.policy.clone())
        .unwrap_or_default();
    if policy.reject_verbs_when_all && all_visual_verbs && content_words > 0 {
        return false;
    }
    if starts_with_interior_capital(text, candidate.start) {
        return false;
    }
    let rune_start = text[..candidate.start].chars().count();
    let rune_end = rune_start + candidate.text.chars().count();
    !overlaps_any_rune_span(rune_start, rune_end, blocked_rune_spans)
}

fn phrase_candidate_score(
    text: &str,
    profile: Option<&PhraseProfile>,
    document_counts: &HashMap<String, usize>,
) -> i64 {
    let tokens = tokenize(text);
    let mut content_words = 0usize;
    let mut visual_verbs = 0usize;
    for token in &tokens {
        let word = token.lower.as_str();
        if is_function_word(word, profile) || word.chars().count() < 3 {
            continue;
        }
        content_words += 1;
        if is_visual_verb(word, profile) {
            visual_verbs += 1;
        }
    }
    let word_count = tokens.len();
    let mut score = content_words as i64 * 4 + visual_verbs as i64 * 5 - word_count as i64;
    if profile.is_some() {
        score += word_count as i64 * 2;
    }
    let normalized = normalized_phrase_key(text);
    let trimmed = normalized.trim_matches(|c| {
        matches!(
            c,
            '.' | ','
                | '!'
                | '?'
                | ';'
                | ':'
                | '"'
                | '\''
                | '’'
                | '”'
                | ')'
                | ']'
                | '}'
                | '»'
                | ' '
        )
    });
    let mut boost = document_term_boost(document_counts.get(&normalized).copied().unwrap_or(0));
    if !trimmed.is_empty() && trimmed != normalized {
        boost = boost.max(document_term_boost(
            document_counts.get(trimmed).copied().unwrap_or(0),
        ));
    }
    score + boost
}

fn starts_at_phrase_boundary(text: &str, byte_start: usize) -> bool {
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

fn starts_with_interior_capital(text: &str, byte_start: usize) -> bool {
    if byte_start == 0 || byte_start > text.len() {
        return false;
    }
    let previous = text[..byte_start].trim();
    previous.ends_with(',')
}

fn is_function_word(word: &str, profile: Option<&PhraseProfile>) -> bool {
    profile.is_some_and(|profile| {
        profile.stop_words.contains(word) || profile.function_words.contains(word)
    })
}

fn is_visual_verb(word: &str, profile: Option<&PhraseProfile>) -> bool {
    profile.is_some_and(|profile| profile.visual_verbs.contains(word))
}

fn normalized_phrase_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn document_phrase_counts(
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

fn document_term_boost(occurrences: usize) -> i64 {
    if occurrences < 2 {
        return 0;
    }
    (4.0 * (1.0 + occurrences as f64).ln()).round() as i64
}

fn overlaps_any_rune_span(start: usize, end: usize, spans: &[(usize, usize)]) -> bool {
    spans
        .iter()
        .any(|&(span_start, span_end)| start < span_end && span_start < end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(words: &[&str]) -> HashSet<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    fn profile(
        stop: &[&str],
        function: &[&str],
        verbs: &[&str],
        min: usize,
        max: usize,
        reject_verbs_when_all: bool,
    ) -> PhraseProfile {
        PhraseProfile {
            stop_words: set(stop),
            function_words: set(function),
            visual_verbs: set(verbs),
            policy: PhraseExtractionPolicy {
                min_words: min,
                max_words: max,
                max_results: 5,
                reject_verbs_when_all,
            },
        }
    }

    fn texts(selected: &[SelectedPhrase]) -> Vec<&str> {
        selected.iter().map(|phrase| phrase.text.as_str()).collect()
    }

    #[test]
    fn ranks_configured_actions_and_skips_blocked_names() {
        let profile = profile(
            &["they"],
            &["they", "later"],
            &["built", "fix"],
            2,
            16,
            true,
        );
        let text = "Ada Lovelace built props. Later, they fix costumes.";
        let selected = select_phrases(text, &[(0, 12)], 5, Some(&profile), &[], 0, 0);
        assert_eq!(texts(&selected), vec!["built props", "fix costumes"]);
        let limited = select_phrases(text, &[(0, 12)], 1, Some(&profile), &[], 0, 0);
        assert_eq!(texts(&limited), vec!["built props"]);
    }

    #[test]
    fn blocked_spans_are_exclusive_and_rune_based() {
        let profile = profile(&[], &[], &[], 2, 16, true);
        let text = "Ada Lovelace built props.";
        let ungrounded = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
        assert_eq!(texts(&ungrounded), vec!["Lovelace built props"]);
        let name_blocked = select_phrases(text, &[(0, 12)], 5, Some(&profile), &[], 0, 0);
        assert_eq!(texts(&name_blocked), vec!["built props"]);
        let action_blocked = select_phrases(text, &[(13, 24)], 5, Some(&profile), &[], 0, 0);
        assert!(action_blocked.is_empty());
    }

    #[test]
    fn corpus_recurrence_boosts_an_equal_candidate() {
        let profile = profile(&[], &[], &[], 2, 16, true);
        let text = "Gamma delta. Alpha beta.";
        let plain = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
        assert_eq!(texts(&plain), vec!["Gamma delta.", "Alpha beta."]);
        let corpus = vec![text.to_string(), "Alpha beta.".to_string()];
        let boosted = select_phrases(text, &[], 5, Some(&profile), &corpus, 0, 0);
        assert_eq!(texts(&boosted), vec!["Alpha beta.", "Gamma delta."]);
    }

    #[test]
    fn keeps_a_natural_six_word_phrase() {
        let profile = profile(&["o", "da"], &["o", "da"], &[], 2, 6, true);
        let phrase = "O maior arrependimento da minha vida";
        let selected = select_phrases(phrase, &[], 5, Some(&profile), &[], 0, 0);
        assert_eq!(
            selected.first().map(|phrase| phrase.text.as_str()),
            Some(phrase)
        );
    }

    #[test]
    fn hard_caps_fragments_at_sixteen_words() {
        let profile = profile(&[], &[], &[], 2, 24, true);
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";
        let selected = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
        assert!(!selected.is_empty());
        for phrase in &selected {
            assert!(phrase.text.split_whitespace().count() <= 16, "{phrase:?}");
        }
    }

    #[test]
    fn includes_complete_sentence_and_two_sentence_card_without_overlap() {
        let profile = profile(&[], &[], &[], 2, 6, true);
        let text = "We built a garden for the neighbors. The garden brought people together. Everyone found a place.";
        let selected = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
        assert_eq!(
            texts(&selected),
            vec![
                "We built a garden for the neighbors. The garden brought people together.",
                "Everyone found a place."
            ]
        );
    }

    #[test]
    fn keeps_a_long_complete_sentence_to_the_terminator() {
        let profile = profile(&[], &[], &[], 2, 6, true);
        let sentence = "Nella provincia di Pavia, la giovane vittima viene ritrovata senza vita nella sua abitazione il tredici agosto del duemilasette in una vicenda che segna la memoria pubblica italiana.";
        let selected = select_phrases(sentence, &[], 5, Some(&profile), &[], 0, 0);
        assert_eq!(
            selected.first().map(|phrase| phrase.text.as_str()),
            Some(sentence)
        );
    }

    #[test]
    fn rejects_function_word_density_above_one_third() {
        let profile = profile(&["of", "my", "in"], &["of", "my", "in"], &[], 6, 6, true);
        let selected = select_phrases(
            "greatest of my in entire life",
            &[],
            5,
            Some(&profile),
            &[],
            0,
            0,
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn proper_name_pairs_ignore_acronym_runs() {
        for (input, want) in [
            ("Ada Lovelace", true),
            ("built props", false),
            ("the Dolly Parton", true),
            ("fix costumes", false),
            ("", false),
            ("USA NATO", false),
            ("AI ML models", false),
            ("the AI act", false),
            ("IBM GmbH", false),
            ("John F Kennedy", true),
            ("New York", true),
        ] {
            assert_eq!(contains_proper_name_pair(input), want, "{input:?}");
        }
    }

    #[test]
    fn words_are_grounded_in_selected_phrases() {
        let profile = profile(&["the"], &["the"], &[], 2, 16, true);
        let selected = select_phrases(
            "The city builds bridges and parks.",
            &[],
            2,
            Some(&profile),
            &[],
            0,
            0,
        );
        let phrases: Vec<String> = selected.into_iter().map(|phrase| phrase.text).collect();
        assert_eq!(
            important_words(&phrases, 3, Some(&profile)),
            vec!["city", "builds", "bridges"]
        );
    }
}
