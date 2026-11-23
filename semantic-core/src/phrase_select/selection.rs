//! Candidate generation, sentence windows and deterministic ranking.

use std::collections::{HashMap, HashSet};

use super::scoring::{
    document_term_boost, is_function_word, is_visual_verb, normalized_phrase_key,
    overlaps_any_rune_span, starts_at_phrase_boundary, starts_with_interior_capital,
};
use super::tokenize::{
    is_terminated_sentence_text, sentence_token_spans, tokenize, SentenceSpan, Token,
};
use super::{contains_proper_name_pair, PhraseExtractionPolicy, PhraseProfile};

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    text: String,
    start: usize,
    token_start: usize,
    token_end: usize,
    score: i64,
    sentence_count: usize,
}

/// Everything both candidate lanes need: the resolved policy, the caller's
/// explicit word window, the grounded spans no phrase may overlap, and the
/// corpus counts. Passing one borrow keeps the stage signatures small.
pub(super) struct SelectionContext<'a> {
    pub(super) profile: Option<&'a PhraseProfile>,
    pub(super) policy: &'a PhraseExtractionPolicy,
    pub(super) min_words: usize,
    pub(super) max_words: usize,
    pub(super) blocked_rune_spans: &'a [(usize, usize)],
    pub(super) document_counts: &'a HashMap<String, usize>,
}

/// Every maximal same-group fragment inside the policy window that clears the
/// phrase gates. Groups are delimited by sentence punctuation, so a candidate
/// never spans two sentences.
pub(super) fn fragment_candidates(
    text: &str,
    tokens: &[Token],
    context: &SelectionContext<'_>,
) -> Vec<Candidate> {
    let SelectionContext {
        profile,
        policy,
        blocked_rune_spans,
        document_counts,
        ..
    } = *context;
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
            let Some((content_words, visual_verbs)) =
                span_word_counts(&tokens[start..=end], word_count, profile, policy)
            else {
                continue;
            };
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

            candidates.push(Candidate {
                text: candidate_text.to_string(),
                start: byte_start,
                token_start: start,
                token_end: end,
                score: fragment_score(
                    content_words,
                    visual_verbs,
                    word_count,
                    profile,
                    document_counts,
                    candidate_text,
                ),
                sentence_count: 0,
            });
        }
    }
    candidates
}

/// Counts the content, visual-verb and function words of one fragment and
/// applies the phrase-material gates. `None` means the run is function-word
/// heavy, too thin, or verb-only while the policy rejects those.
fn span_word_counts(
    tokens: &[Token],
    word_count: usize,
    profile: Option<&PhraseProfile>,
    policy: &PhraseExtractionPolicy,
) -> Option<(usize, usize)> {
    let mut content_words = 0usize;
    let mut visual_verbs = 0usize;
    let mut function_words = 0usize;
    let mut all_visual_verbs = true;
    for token in tokens {
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
        return None;
    }
    Some((content_words, visual_verbs))
}

/// Content density plus the document-frequency boost. A caller-supplied
/// profile also rewards length, which is the Go selector's tie-break.
fn fragment_score(
    content_words: usize,
    visual_verbs: usize,
    word_count: usize,
    profile: Option<&PhraseProfile>,
    document_counts: &HashMap<String, usize>,
    candidate_text: &str,
) -> i64 {
    let mut score = content_words as i64 * 4 + visual_verbs as i64 * 5 - word_count as i64;
    if profile.is_some() {
        score += word_count as i64 * 2;
    }
    score
        + document_term_boost(
            document_counts
                .get(&normalized_phrase_key(candidate_text))
                .copied()
                .unwrap_or(0),
        )
}

/// Whole-sentence and two-sentence candidates. These exist because a complete
/// sentence is often the editorial unit even when it exceeds the fragment cap.
pub(super) fn sentence_candidates(
    text: &str,
    tokens: &[Token],
    context: &SelectionContext<'_>,
) -> Vec<Candidate> {
    let sentences = sentence_token_spans(text, tokens);
    let mut candidates = Vec::new();
    for i in 0..sentences.len() {
        for width in 1..=2 {
            if i + width > sentences.len() {
                break;
            }
            if let Some(candidate) = sentence_window(text, &sentences, i, width, context) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn sentence_window(
    text: &str,
    sentences: &[SentenceSpan],
    index: usize,
    width: usize,
    context: &SelectionContext<'_>,
) -> Option<Candidate> {
    let SelectionContext {
        profile,
        policy,
        min_words,
        max_words,
        blocked_rune_spans,
        document_counts,
    } = *context;
    let first = &sentences[index];
    let last = &sentences[index + width - 1];
    let word_count = last.token_end - first.token_start + 1;
    if (min_words > 0 && word_count < min_words) || (max_words > 0 && word_count > max_words) {
        return None;
    }
    if width == 2 && word_count <= policy.max_words {
        return None;
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
    let at_scene_end = width == 1 && index == sentences.len() - 1;
    let terminated = is_terminated_sentence_text(&span.text);
    if !terminated && !at_scene_end {
        return None;
    }
    if word_count > 30 {
        return None;
    }
    if !terminated && word_count > 16 {
        return None;
    }
    if !sentence_candidate_allowed(&span, profile, blocked_rune_spans, text) {
        return None;
    }
    span.score = phrase_candidate_score(&span.text, profile, document_counts);
    Some(span)
}

/// Orders candidates (long spans first, then score, sentence count and
/// position) and keeps a prefix with no duplicate surface and no overlapping
/// token span.
pub(super) fn select_ranked(mut candidates: Vec<Candidate>, limit: usize) -> Vec<Candidate> {
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
}

/// Projects an internal candidate onto the wire shape, in the trimmed text's
/// byte and rune coordinates.
pub(super) fn selected_phrase(text: &str, candidate: Candidate) -> super::SelectedPhrase {
    let start_rune = text[..candidate.start].chars().count();
    let end_rune = start_rune + candidate.text.chars().count();
    super::SelectedPhrase {
        end_byte: candidate.start + candidate.text.len(),
        text: candidate.text,
        start_byte: candidate.start,
        start_rune,
        end_rune,
        score: candidate.score,
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
