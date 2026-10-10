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
    let tokens = tokenize(source_text);
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
    unique.truncate(options.top_n());
    Ok(unique)
}

/// Token is a maximal run of letters (ASCII + accented Latin) and
/// apostrophes/hyphens inside a word, with its byte offsets into the
/// source text. Non-letter chars are separators.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    start: usize,
    end: usize,
}

/// tokenize splits `source_text` into maximal letter-runs, recording byte
/// offsets. Case is preserved in `text`; comparisons use a lowercased copy.
fn tokenize(source_text: &str) -> Vec<Token> {
    let bytes = source_text.as_bytes();
    let mut tokens = Vec::new();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if is_word_byte(bytes[i]) {
            let start = i;
            while i < n && is_word_byte(bytes[i]) {
                i += 1;
            }
            let text = std::str::from_utf8(&bytes[start..i])
                .unwrap_or("")
                .to_string();
            tokens.push(Token {
                text,
                start,
                end: i,
            });
        } else {
            i += 1;
        }
    }
    tokens
}

/// is_word_byte reports whether a byte is part of a word: ASCII letter,
/// apostrophe, hyphen, or a high bit (UTF-8 continuation/lead byte for
/// accented Latin like "à"). This keeps the tokenizer dependency-free.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'\'' || b == b'-' || b >= 0x80
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
        if is_stop_word(&tokens[i].text)
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
            if j == phrase_start + 1 && j < tokens.len() && !is_stop_word(&tokens[j].text) {
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
            while j < tokens.len() && !is_stop_word(&tokens[j].text) {
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
                && !is_stop_word(&tokens[final_index].text)
            {
                runs.push((i, final_index + 1));
                i = final_index + 1;
                continue;
            }
            i += 1;
            continue;
        }
        if is_stop_word(&tokens[i].text) || !starts_uppercase(&tokens[i].text) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        while end < tokens.len()
            && starts_uppercase(&tokens[end].text)
            && !is_stop_word(&tokens[end].text)
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

/// Mark a source-grounded proper-name candidate as a place when its local
/// syntax supplies geographic context. The extractor has no gazetteer, so it
/// does not guess from capitalization alone; cues such as "em Fortaleza" and
/// "no bairro Aldeota" provide the evidence for this classification.
fn mark_contextual_locations(candidates: &mut Vec<Candidate>, tokens: &[Token], source_text: &str) {
    let bytes = source_text.as_bytes();
    let mut contextual = Vec::new();
    for index in 0..tokens.len() {
        if !starts_uppercase(&tokens[index].text)
            || is_month_name(&tokens[index].text)
            || !has_location_context(tokens, bytes, index)
        {
            continue;
        }
        let mut end = index + 1;
        while end < tokens.len() && starts_uppercase(&tokens[end].text) {
            if bytes[tokens[end - 1].end..tokens[end].start]
                .iter()
                .any(|byte| !byte.is_ascii_whitespace())
            {
                break;
            }
            end += 1;
        }
        let mut place = candidate_from_tokens(tokens, source_text, index, end);
        if is_generic_location_noun(&place.text) {
            continue;
        }
        place.entity_type = Some("LOCATION".to_string());
        contextual.push(place);
    }

    for candidate in candidates.iter_mut() {
        if candidate.entity_type.is_some() || !starts_uppercase(&candidate.text) {
            continue;
        }
        let Some(token_index) = tokens
            .iter()
            .position(|token| token.start == candidate.start)
        else {
            continue;
        };
        if has_location_context(tokens, bytes, token_index)
            && !is_month_name(&candidate.text)
            && !is_generic_location_noun(&candidate.text)
        {
            candidate.entity_type = Some("LOCATION".to_string());
        }
    }
    for place in contextual {
        if !candidates
            .iter()
            .any(|candidate| candidate.start == place.start && candidate.end == place.end)
        {
            candidates.push(place);
        }
    }
}

fn has_location_context(tokens: &[Token], bytes: &[u8], token_index: usize) -> bool {
    let Some(previous) = token_index.checked_sub(1).map(|index| &tokens[index]) else {
        return false;
    };
    if bytes[previous.end..tokens[token_index].start]
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
    {
        return false;
    }
    let previous = previous.text.to_lowercase();
    matches!(
        previous.as_str(),
        "em" | "no"
            | "na"
            | "nos"
            | "nas"
            | "in"
            | "at"
            | "near"
            | "bairro"
            | "city"
            | "county"
            | "state"
            | "province"
            | "municipality"
            | "município"
            | "região"
            | "region"
    ) || (matches!(previous.as_str(), "de" | "of")
        && token_index >= 2
        && matches!(
            tokens[token_index - 2].text.to_lowercase().as_str(),
            "cidade"
                | "city"
                | "município"
                | "municipality"
                | "região"
                | "region"
                | "estado"
                | "state"
                | "província"
                | "province"
                | "county"
        ))
}

fn is_month_name(text: &str) -> bool {
    matches!(
        text.to_lowercase().as_str(),
        "january"
            | "february"
            | "march"
            | "april"
            | "may"
            | "june"
            | "july"
            | "august"
            | "september"
            | "october"
            | "november"
            | "december"
            | "janeiro"
            | "fevereiro"
            | "março"
            | "abril"
            | "maio"
            | "junho"
            | "julho"
            | "agosto"
            | "setembro"
            | "outubro"
            | "novembro"
            | "dezembro"
    )
}

fn is_generic_location_noun(text: &str) -> bool {
    matches!(
        text.to_lowercase().as_str(),
        "city"
            | "county"
            | "state"
            | "province"
            | "municipality"
            | "município"
            | "bairro"
            | "cidade"
            | "região"
            | "region"
            | "estado"
            | "província"
    )
}

fn starts_uppercase(text: &str) -> bool {
    text.chars().next().map(char::is_uppercase).unwrap_or(false)
}

fn is_uppercase_initial(text: &str) -> bool {
    text.len() == 1 && text.as_bytes()[0].is_ascii_uppercase()
}

fn has_dotted_initial_prefix(tokens: &[Token], bytes: &[u8], name_index: usize) -> bool {
    if name_index == 0 {
        return false;
    }
    let mut cursor = name_index;
    let mut initials = 0;
    while cursor > 0 {
        let previous = cursor - 1;
        if !is_uppercase_initial(&tokens[previous].text)
            || !gap_has_period(&bytes[tokens[previous].end..tokens[cursor].start])
        {
            break;
        }
        initials += 1;
        cursor = previous;
    }
    initials > 0
}

fn gap_has_period(gap: &[u8]) -> bool {
    gap.first() == Some(&b'.') && gap[1..].iter().all(u8::is_ascii_whitespace)
}

/// is_phrase_breaking_byte reports whether a byte in the gap between two
/// tokens should break a noun phrase. Whitespace (space, tab, newline)
/// does NOT break; any other non-word byte (comma, period, semicolon,
/// colon, dash outside a word, parens) DOES break. This is what keeps
/// "feta cheese" together while splitting "tomatoes, feta".
fn is_phrase_breaking_byte(b: u8) -> bool {
    !b.is_ascii_whitespace()
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

/// Stopwords: single tokens that never start or extend a noun phrase.
/// Includes the LIVE-test hallucination seeds "imagine", "ready", "get",
/// "the", "a", "and", etc. so "Imagine the" / "Get ready" never produce
/// candidates in the first place.
const STOP_WORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "and",
    "or",
    "of",
    "to",
    "in",
    "on",
    "for",
    "with",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "has",
    "have",
    "had",
    "this",
    "that",
    "these",
    "those",
    "it",
    "its",
    "as",
    "by",
    "at",
    "from",
    // hallucination seeds observed in the LIVE test
    "imagine",
    "ready",
    "get",
    "dive",
    "into",
    "discover",
    "let",
    "us",
    // pronouns / generic verbs
    "you",
    "we",
    "they",
    "he",
    "she",
    "i",
    "me",
    "him",
    "her",
    "do",
    "does",
    "did",
    "not",
    "no",
    "yes",
    "about",
    "your",
    "our",
    "their",
    "his",
    // generic spatial/temporal words
    "now",
    "then",
    "here",
    "there",
    "when",
    "where",
    "what",
    "which",
    "who",
    // connector / descriptive verbs that glue non-visual phrases
    // together in the Mediterranean golden-fixture sentences. These are
    // not visual objects, so a phrase that includes them is not a single
    // visual noun phrase; breaking here keeps "feta cheese" separate
    // from "tomatoes" when the sentence reads "combines fresh tomatoes".
    "combines",
    "contains",
    "features",
    "traditionally",
    "made",
    "fresh",
    "prepared",
    "spoke",
    "released",
    // sentence-opening gerund; it must not absorb the following proper name
    // into a false entity such as "Understanding Donald Trump's".
    "understanding",
    // High-frequency function words and titles in the supported generation
    // languages. Keep this deterministic vocabulary conservative: removing a
    // token from candidate construction is safer than emitting it as an entity.
    "el",
    "la",
    "los",
    "las",
    "un",
    "una",
    "unos",
    "unas",
    "y",
    "de",
    "del",
    "en",
    "por",
    "con",
    "para",
    "es",
    "son",
    "fue",
    "han",
    "ha",
    "al",
    "lo",
    "le",
    "les",
    "des",
    "du",
    "un",
    "une",
    "et",
    "est",
    "sont",
    "au",
    "aux",
    "dans",
    "sur",
    "a",
    "o",
    "os",
    "as",
    "um",
    "uma",
    "e",
    "do",
    "da",
    "dos",
    "das",
    "no",
    "na",
    "nos",
    "nas",
    "com",
    "que",
    "il",
    "i",
    "gli",
    "lo",
    "una",
    "uno",
    "è",
    "sono",
    "ha",
    "hanno",
    "nel",
    "nella",
    "di",
    "che",
    "mit",
    "der",
    "die",
    "das",
    "den",
    "dem",
    "ein",
    "eine",
    "einer",
    "eines",
    "und",
    "ist",
    "sind",
    "war",
    "wurde",
    "nach",
    "bei",
    "von",
    "zu",
    "zum",
    "zur",
    "presidente",
    "président",
    "bundeskanzler",
    "kanzler",
    "discussed",
    "visited",
    "visitó",
    "visitou",
    "visitato",
    "visité",
    "besuchte",
    "company",
];

/// Stop phrases: multi-word surfaces that are generic even when none of
/// their tokens are individually stopwords.
const STOP_PHRASES: &[&str] = &["get ready", "let us", "imagine the"];

fn is_stop_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    STOP_WORDS.iter().any(|s| *s == lower)
        || word
            .chars()
            .any(|ch| matches!(ch, 'à' | 'è' | 'é' | 'ì' | 'ò' | 'ù' | 'ü' | 'ö' | 'ä'))
            && NON_ASCII_STOP_WORDS.iter().any(|stop| *stop == lower)
}

const NON_ASCII_STOP_WORDS: &[&str] = &[
    "è",
    "président",
    "présidente",
    "präsident",
    "município",
    "região",
    "província",
];

fn is_stop_phrase(normalized: &str) -> bool {
    STOP_PHRASES.contains(&normalized)
}

// ── Tests ────────────────────────────────────────────────────────────


#[cfg(test)]
#[path = "extractor_tests.rs"]
mod tests;
