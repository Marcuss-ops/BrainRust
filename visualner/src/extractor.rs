//! The deterministic VisualNER extraction pipeline (V1):
//!
//! ```text
//! text
//!  ↓
//! tokenize
//!  ↓
//! noun/noun-phrase candidates
//!  ↓
//! stop phrases
//!  ↓
//! visualness scoring
//!  ↓
//! source evidence validation
//!  ↓
//! rank
//!  ↓
//! top N
//! ```
//!
//! V1 is deterministic: NO ONNX, NO LLM, NO GPU. The killer rule is
//! `NO EVIDENCE → NO ENTITY`: every returned entity must be a verbatim
//! substring of the source text, with byte offsets proving the evidence.

mod location;
mod pos;
mod stopwords;
use location::*;
use pos::{is_stop_token, tag_tokens};
use stopwords::*;

use std::collections::HashMap;

use crate::types::{ExtractOptions, VisualEntity};

mod ranking;
mod values;

use ranking::{
    entity_identity_key, is_generic_phrase, is_subject_phrase, is_visual_object_hint,
    named_entity_rank, score_entity,
};
use values::{classify_value_type, value_candidates, Candidate};

/// Extract the top-N source-grounded visual entities from `source_text`.
/// This is the single canonical entry point. Returns entities in score-desc
/// order (ties broken by earliest source position).
pub fn extract(source_text: &str, options: &ExtractOptions) -> Result<Vec<VisualEntity>, String> {
    let language = options.language.trim();
    if language.is_empty() {
        return Err("VisualNER language is required".to_string());
    }
    let tokens = tokenize(source_text, language)?;
    let mut candidates = noun_phrase_candidates(&tokens, source_text);
    candidates.extend(value_candidates(source_text));
    mark_contextual_locations(&mut candidates, &tokens, source_text);
    let spelled_candidates = crate::spellout::candidates(source_text, language)?;
    let spelled_ranges: Vec<(usize, usize)> = spelled_candidates
        .iter()
        .map(|number| (number.start, number.end))
        .collect();
    candidates.extend(spelled_candidates.into_iter().map(|number| {
        let text = source_text[number.start..number.end].to_string();
        Candidate {
            normalized: text.to_lowercase(),
            text,
            start: number.start,
            end: number.end,
            entity_type: Some(number.entity_type.to_string()),
        }
    }));
    let mut scored: Vec<VisualEntity> = candidates
        .into_iter()
        .filter(|candidate| {
            !spelled_ranges.iter().any(|(start, end)| {
                candidate.start < *end
                    && *start < candidate.end
                    && !(candidate.start == *start && candidate.end == *end)
            })
        })
        .filter_map(|c| validate_evidence(c, source_text))
        .map(score_entity)
        .collect();
    // Rank: highest score first; ties broken by earliest start offset so
    // the order is deterministic across 100/100 runs.
    scored.sort_by(|a, b| {
        named_entity_rank(a)
            .cmp(&named_entity_rank(b))
            .then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.start.cmp(&b.start))
            .then(a.text.cmp(&b.text))
    });
    // A possessive mention is the same identity as its non-possessive form.
    // Deduplicate before applying top-N so "Michael Jordan" and
    // "Michael Jordan's" cannot consume two entity slots.
    //
    // The identity key is hashed into an index map: the previous scan
    // recomputed `entity_identity_key` (a `to_lowercase` + `format!`) for every
    // already-accepted candidate on every incoming entity, which is O(N²)
    // String allocations per extraction call.
    let mut unique: Vec<VisualEntity> = Vec::with_capacity(scored.len());
    let mut index_by_key: HashMap<String, usize> = HashMap::with_capacity(scored.len());
    for entity in scored {
        let key = entity_identity_key(&entity);
        if let Some(&existing_idx) = index_by_key.get(&key) {
            // Prefer the shorter non-possessive surface when both mentions
            // occur; otherwise retain the first deterministic evidence span.
            let existing = &mut unique[existing_idx];
            if existing.r#type == "PERSON" && entity.text.len() < existing.text.len() {
                *existing = entity;
            }
            continue;
        }
        index_by_key.insert(key, unique.len());
        unique.push(entity);
    }
    Ok(reserve_numeric_slots(unique, options.top_n()))
}

/// Applies the top-N cap while keeping a minimum share of numeric value
/// entities (NUMBER/PERCENT/MONEY). Without this, a low cap ranks every value
/// below the identities and the chart loses its numbers entirely. The share is
/// a third of the cap (at least one slot); numeric entities beyond the cap
/// replace the lowest-ranked non-numeric entities only when fewer numerics
/// made the cut. Surviving entities keep their original rank order.
fn reserve_numeric_slots(ranked: Vec<VisualEntity>, top_n: usize) -> Vec<VisualEntity> {
    if ranked.len() <= top_n {
        return ranked;
    }
    let is_numeric = |e: &VisualEntity| matches!(e.r#type.as_str(), "NUMBER" | "PERCENT" | "MONEY");
    let reserve = (top_n / 3).max(1).min(top_n);
    let kept_numeric = ranked[..top_n].iter().filter(|e| is_numeric(e)).count();
    let spare: Vec<usize> = (top_n..ranked.len()).filter(|&i| is_numeric(&ranked[i])).collect();
    let needed = reserve.saturating_sub(kept_numeric).min(spare.len());
    let mut keep = vec![true; top_n];
    let mut evicted = 0;
    for pos in (0..top_n).rev() {
        if evicted == needed {
            break;
        }
        if !is_numeric(&ranked[pos]) {
            keep[pos] = false;
            evicted += 1;
        }
    }
    let mut selected: Vec<usize> = (0..top_n).filter(|&pos| keep[pos]).chain(spare.into_iter().take(needed)).collect();
    selected.sort_unstable();
    let mut slots: Vec<Option<VisualEntity>> = ranked.into_iter().map(Some).collect();
    selected
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}

/// Token is a maximal run of letters (ASCII + accented Latin) and
/// apostrophes/hyphens inside a word, with its byte offsets into the
/// source text. Non-letter chars are separators.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    start: usize,
    end: usize,
    /// Universal POS tag from UDPipe, when a model for the language is installed.
    upos: Option<String>,
}

/// tokenize segments `source_text` into words with ICU's word-break rules for
/// `language`. ICU handles scripts without spaces (Chinese, Japanese, Thai)
/// and language-specific apostrophes, so no per-language lists are needed.
/// Hyphenated and apostrophe-joined compounds (Jean-Luc) become one token.
/// ICU offsets are UTF-16 units; they are converted to UTF-8 byte offsets.
fn tokenize(source_text: &str, language: &str) -> Result<Vec<Token>, String> {
    let breaks = rust_icu_ubrk::UBreakIterator::try_new(
        rust_icu_sys::UBreakIteratorType::UBRK_WORD,
        language,
        source_text,
    )
    .map_err(|error| format!("ICU word segmentation failed for {language:?}: {error}"))?;

    let mut utf16_to_byte: Vec<usize> = Vec::with_capacity(source_text.len() + 1);
    for (offset, ch) in source_text.char_indices() {
        for _ in 0..ch.len_utf16() {
            utf16_to_byte.push(offset);
        }
    }
    utf16_to_byte.push(source_text.len());

    // (byte_start, byte_end, is_word) for every ICU segment, in order.
    let mut segments: Vec<(usize, usize, bool)> = Vec::new();
    let mut start16 = breaks.first();
    loop {
        let end16 = breaks.following(start16);
        if end16 < 0 {
            break;
        }
        // UBRK_WORD_NONE_LIMIT is 100: rule status >= 100 marks word-like text.
        let is_word = breaks.get_rule_status() >= 100;
        segments.push((
            utf16_to_byte[start16 as usize],
            utf16_to_byte[end16 as usize],
            is_word,
        ));
        start16 = end16;
    }

    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < segments.len() {
        let (start, end, is_word) = segments[index];
        index += 1;
        if !is_word {
            continue;
        }
        let mut token_end = end;
        // Join word, joiner, word where the joiner is a hyphen or apostrophe.
        while index + 1 < segments.len() {
            let (_, joiner_end, joiner_word) = segments[index];
            let (next_start, next_end, next_word) = segments[index + 1];
            let joiner = &source_text[token_end..joiner_end];
            let is_joiner = joiner == "-" || joiner == "'" || joiner == "\u{2019}";
            if is_joiner && !joiner_word && next_word && next_start == joiner_end {
                token_end = next_end;
                index += 2;
            } else {
                break;
            }
        }
        tokens.push(Token {
            text: source_text[start..token_end].to_string(),
            start,
            end: token_end,
            upos: None,
        });
    }
    tag_tokens(source_text, language, &mut tokens);
    Ok(tokens)
}

/// noun_phrase_candidates groups adjacent non-stopword tokens into maximal
/// noun phrases. A phrase is one or more consecutive tokens whose lowercase
/// surface is not a stopword, AND that are separated only by whitespace
/// (a comma, period, semicolon or other punctuation between two tokens is a
/// hard phrase boundary). Multi-word phrases ("feta cheese",
/// "olive oil") score higher than singletons, which is what makes the
/// Mediterranean golden fixture's anchors win over generic singletons.
fn noun_phrase_candidates(tokens: &[Token], source_text: &str) -> Vec<Candidate> {
    let bytes = source_text.as_bytes();
    let mut out = Vec::new();
    // Named people/organizations are bounded to the contiguous title-case run.
    // This prevents lower-case lead-ins such as "figures like Phil Jackson"
    // from becoming a PERSON entity; the grounded candidate is "Phil Jackson".
    for (start, end) in proper_name_runs(tokens, bytes) {
        out.push(candidate_from_tokens(tokens, source_text, start, end));
    }
    let mut i = 0;
    while i < tokens.len() {
        // A single uppercase initial between periods is a tokenization
        // artifact of dotted initialisms (e.g. U.S.), not an entity span.
        if is_stop_token(&tokens[i])
            || (tokens[i].text.len() == 1 && tokens[i].text.as_bytes()[0].is_ascii_uppercase())
        {
            i += 1;
            continue;
        }
        // Proper names are bounded by the title-case run. The old maximal
        // noun-phrase rule incorrectly absorbed predicates and their objects
        // (for example "Floyd Mayweather became one"), producing invalid
        // semantic spans. Lowercase phrases retain the noun-phrase behavior
        // used by the media-object fixtures.
        let phrase_start = i;
        let mut j = i;
        if starts_uppercase(&tokens[i].text) {
            if has_dotted_initial_prefix(tokens, bytes, i) {
                i += 1;
                continue;
            }
            while j < tokens.len() && starts_uppercase(&tokens[j].text) {
                if j > phrase_start {
                    let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                    if gap.iter().any(|b| is_phrase_breaking_byte(*b)) {
                        break;
                    }
                }
                j += 1;
            }
            // Keep the curated multi-word visual subjects intact even when
            // their first token is title-cased at sentence start.
            if j == phrase_start + 1 && j < tokens.len() && !is_stop_token(&tokens[j]) {
                let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                let compound =
                    format!("{} {}", tokens[phrase_start].text, tokens[j].text).to_lowercase();
                if !gap.iter().any(|b| is_phrase_breaking_byte(*b))
                    && (is_visual_object_hint(&compound) || is_subject_phrase(&compound))
                {
                    j += 1;
                }
            }
        } else {
            while j < tokens.len() && !is_stop_token(&tokens[j]) {
                if j > phrase_start {
                    let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                    if gap.iter().any(|b| is_phrase_breaking_byte(*b))
                        || starts_uppercase(&tokens[j].text)
                    {
                        // A lower-case clause/predicate must not absorb the
                        // next title-cased name into a VISUAL_CONCEPT span
                        // (e.g. "discussed Tesla"). Let the proper-name
                        // candidate own the capitalized token instead.
                        break;
                    }
                }
                j += 1;
            }
        }
        let phrase_end = j; // exclusive
        let start = tokens[phrase_start].start;
        let end = tokens[phrase_end - 1].end;
        let text = source_text[start..end].to_string();
        let normalized = text.to_lowercase();
        // The run decomposition depends only on the phrase, so it is computed
        // ONCE here instead of twice (the previous form called
        // proper_name_runs twice, each allocating a fresh Vec).
        let phrase_runs = proper_name_runs(&tokens[phrase_start..phrase_end], bytes);
        let has_inner_run = phrase_runs.iter().any(|(run_start, _)| *run_start > 0);
        let is_full_run = phrase_runs
            .iter()
            .any(|(run_start, run_end)| *run_start == 0 && *run_end == phrase_end - phrase_start);
        if has_inner_run && !is_full_run {
            i = phrase_end;
            continue;
        }
        // Reject phrases whose normalized surface is a stop phrase, a
        // generic phrase, OR a multi-word subject phrase. Multi-word
        // dish names ("greek salad", "grilled sardines", "seafood paella")
        // belong on SceneIR.Profile.Subject, not in Entities; dropping
        // them here lets the concrete ingredients ("feta cheese",
        // "tomatoes", "olives") fill the top-N. Single-word subjects
        // ("hummus", "shakshuka", "paella") are NOT dropped — they are
        // legitimate one-word ingredient/dish candidates.
        if !is_stop_phrase(&normalized)
            && !is_generic_phrase(&normalized)
            && !is_subject_phrase(&normalized)
        {
            out.push(Candidate {
                text,
                normalized,
                start,
                end,
                entity_type: None,
            });
        }
        i = phrase_end;
    }
    out
}

fn proper_name_runs(tokens: &[Token], bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        // Dotted initials belong to the following title-cased name (J. D.
        // Vance), but dotted initialisms such as U.S. have no trailing name
        // and must not create fragments. Keep the boundary evidence verbatim.
        if is_uppercase_initial(&tokens[i].text) {
            let mut final_index = i;
            while final_index + 1 < tokens.len()
                && gap_has_period(&bytes[tokens[final_index].end..tokens[final_index + 1].start])
            {
                final_index += 1;
                if !is_uppercase_initial(&tokens[final_index].text) {
                    break;
                }
            }
            if final_index > i
                && !is_uppercase_initial(&tokens[final_index].text)
                && starts_uppercase(&tokens[final_index].text)
                && !is_stop_token(&tokens[final_index])
            {
                runs.push((i, final_index + 1));
                i = final_index + 1;
                continue;
            }
            i += 1;
            continue;
        }
        if is_stop_token(&tokens[i]) || !starts_uppercase(&tokens[i].text) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        while end < tokens.len()
            && starts_uppercase(&tokens[end].text)
            && !is_stop_token(&tokens[end])
            && !(tokens[end].text.len() == 1 && tokens[end].text.as_bytes()[0].is_ascii_uppercase())
            && !bytes[tokens[end - 1].end..tokens[end].start]
                .iter()
                .any(|b| is_phrase_breaking_byte(*b))
        {
            end += 1;
        }
        if end - start >= 2 {
            runs.push((start, end));
        }
        i = end;
    }
    runs
}

fn candidate_from_tokens(
    tokens: &[Token],
    source_text: &str,
    start: usize,
    end: usize,
) -> Candidate {
    let byte_start = tokens[start].start;
    let byte_end = tokens[end - 1].end;
    let text = source_text[byte_start..byte_end].to_string();
    Candidate {
        normalized: text.to_lowercase(),
        text,
        start: byte_start,
        end: byte_end,
        entity_type: None,
    }
}

/// validate_evidence enforces NO EVIDENCE → NO ENTITY. A candidate is
/// grounded when its byte span `[start, end)` is a verbatim substring of
/// `source_text` (which it is by construction, since the span was sliced
/// from the source). The function additionally guards against accidental
/// spans that don't round-trip (defensive: catches a future refactor bug
/// where offsets drift from the text). Returns `None` for an ungrounded
/// candidate.
fn validate_evidence(c: Candidate, source_text: &str) -> Option<VisualEntity> {
    if c.start >= c.end || c.end > source_text.len() {
        return None;
    }
    let evidence = source_text.get(c.start..c.end)?;
    // The evidence must equal the candidate text verbatim. A candidate
    // whose text drifted from its span (e.g. due to a normalization step
    // that rewrote the surface) fails NO EVIDENCE → NO ENTITY.
    if evidence != c.text {
        return None;
    }
    Some(VisualEntity {
        text: c.text.clone(),
        r#type: c
            .entity_type
            .unwrap_or_else(|| classify_value_type(&c.text)),
        score: 0.0,
        start: c.start,
        end: c.end,
        evidence: evidence.to_string(),
    })
}

// ── Stopword + blocklist tables ──────────────────────────────────────
//
// V1 uses curated, dependency-free tables. The tables are intentionally
// small and explicit so the extractor stays deterministic and auditable.
// Adding a word here is a one-line change; no model retraining.

#[cfg(test)]
#[path = "extractor_tests.rs"]
mod tests;
