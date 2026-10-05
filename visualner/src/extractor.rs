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
mod tests {
    use super::*;

    fn top3(text: &str) -> Vec<VisualEntity> {
        extract(
            text,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 3,
            },
        )
        .unwrap()
    }

    fn texts(entities: &[VisualEntity]) -> Vec<String> {
        entities.iter().map(|e| e.text.clone()).collect()
    }

    // TestEntitiesRequireSourceEvidence — the killer rule.
    // "Imagine the" / "ready" must NEVER become entities because they are
    // stopword-seeded and never produce candidates in the first place.
    #[test]
    fn imagine_the_and_ready_are_rejected() {
        let entities = top3("Imagine the vibrant world of Greek cuisine, ready to discover...");
        let surfaces: Vec<&str> = entities.iter().map(|e| e.text.as_str()).collect();
        assert!(
            !surfaces
                .iter()
                .any(|s| s.to_lowercase().contains("imagine")),
            "Imagine the must not become an entity: {:?}",
            surfaces
        );
        assert!(
            !surfaces.iter().any(|s| s.to_lowercase() == "ready"),
            "ready must not become an entity: {:?}",
            surfaces
        );
        assert!(
            !surfaces
                .iter()
                .any(|s| s.to_lowercase().contains("discover")),
            "discover must not become an entity: {:?}",
            surfaces
        );
    }

    // TestExactEntityLimit — requesting 3 returns exactly 3 when the text
    // has ≥3 candidates.
    #[test]
    fn exactly_three_entities_returned() {
        let entities = top3(
            "Greek salad combines fresh tomatoes, cucumbers, olives, feta cheese and olive oil.",
        );
        assert_eq!(
            entities.len(),
            3,
            "expected exactly 3 entities, got {entities:?}"
        );
    }

    // Greek salad regression — the canonical golden-fixture input.
    // Expected: feta cheese, tomatoes, olives (top 3 by visual score).
    #[test]
    fn greek_salad_returns_feta_tomatoes_olives() {
        let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
        let surfaces = texts(&entities);
        assert!(
            surfaces.iter().any(|s| s.to_lowercase() == "feta cheese"),
            "feta cheese missing: {surfaces:?}"
        );
        assert!(
            surfaces.iter().any(|s| s.to_lowercase() == "tomatoes"),
            "tomatoes missing: {surfaces:?}"
        );
        assert!(
            surfaces.iter().any(|s| s.to_lowercase() == "olives"),
            "olives missing: {surfaces:?}"
        );
        // All 3 must be source-grounded.
        for e in &entities {
            assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
            assert_eq!(e.evidence, e.text, "evidence must equal text verbatim");
        }
    }

    // Hummus regression — the second golden-fixture segment.
    // Expected candidates include hummus, chickpeas, tahini, lemon juice, olive oil;
    // top 3 must all be source-grounded.
    #[test]
    fn hummus_returns_source_grounded_entities() {
        let entities =
            top3("Hummus is traditionally made with chickpeas, tahini, lemon juice and olive oil.");
        // All returned entities must be source-grounded (NO EVIDENCE → NO ENTITY).
        for e in &entities {
            assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
            assert_eq!(
                e.evidence, e.text,
                "evidence must equal text verbatim for {}",
                e.text
            );
        }
        // The top 3 must come from the expected candidate set.
        let expected = ["hummus", "chickpeas", "tahini", "lemon juice", "olive oil"];
        for e in &entities {
            let lower = e.text.to_lowercase();
            assert!(
                expected.iter().any(|s| *s == lower),
                "unexpected entity {}: not in {expected:?}",
                e.text
            );
        }
    }

    #[test]
    fn trump_is_retained_as_a_person_in_a_dense_scene() {
        let text = "Donald Trump's trajectory offers a profound case study into the interwoven nature of business success, intense media visibility, and political leadership within American public life. His career has consistently demonstrated how these three elements feed one another, creating a defining narrative arc for Donald Trump. Understanding Donald Trump's influence requires recognizing his public impact.";
        let entities = extract(
            text,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 5,
            },
        )
        .unwrap();
        let trump = entities.iter().find(|entity| {
            entity.r#type == "PERSON" && entity.text.to_lowercase().contains("trump")
        });
        assert!(
            trump.is_some(),
            "Donald Trump must survive the top-N visual entity bound: {entities:?}"
        );
        let trump = trump.unwrap();
        assert_eq!(trump.text, "Donald Trump");
        assert_eq!(trump.evidence, trump.text);
    }

    #[test]
    fn person_runs_are_not_prefixed_or_suffixed_by_sentence_text() {
        let text = "Michael Jordan transformed basketball. Phil Jackson designed the offense. Scottie Pippen supplied versatile defense.";
        let entities = extract(
            text,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 5,
            },
        )
        .unwrap();
        let names: Vec<&str> = entities
            .iter()
            .filter(|entity| entity.r#type == "PERSON")
            .map(|entity| entity.text.as_str())
            .collect();
        assert!(
            names.contains(&"Michael Jordan"),
            "Michael Jordan missing: {entities:?}"
        );
        assert!(
            names.contains(&"Phil Jackson"),
            "Phil Jackson missing: {entities:?}"
        );
        assert!(
            names.contains(&"Scottie Pippen"),
            "Scottie Pippen missing: {entities:?}"
        );
        assert!(
            !names
                .iter()
                .any(|name| name.contains("designed") || name.contains("supplied")),
            "predicate leaked into person: {names:?}"
        );
    }

    #[test]
    fn stopword_leads_and_known_locations_do_not_consume_person_slots() {
        let text = "Michael Jordan studied at the University of North Carolina with Dean Smith. When Jordan entered the game, Scottie Pippen and Phil Jackson supported him.";
        let entities = extract(
            text,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 5,
            },
        )
        .unwrap();
        let persons: Vec<&str> = entities
            .iter()
            .filter(|entity| entity.r#type == "PERSON")
            .map(|entity| entity.text.as_str())
            .collect();
        assert_eq!(
            persons,
            vec![
                "Michael Jordan",
                "Dean Smith",
                "Scottie Pippen",
                "Phil Jackson"
            ]
        );
        assert!(
            !entities.iter().any(|entity| entity.text == "When Jordan"),
            "stopword-prefixed false person must be rejected: {entities:?}"
        );
        assert!(
            !persons.contains(&"North Carolina"),
            "known location must not be typed as PERSON: {entities:?}"
        );
    }

    #[test]
    fn company_suffixes_are_classified_as_organizations() {
        let source = "Acme Inc. reported revenue.";
        let entities = extract(
            source,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        let company = entities
            .iter()
            .find(|entity| entity.text == "Acme Inc")
            .unwrap_or_else(|| panic!("organization missing: {entities:?}"));
        assert_eq!(company.r#type, "ORGANIZATION");
        assert_eq!(&source[company.start..company.end], company.text);
    }

    #[test]
    fn person_and_brand_mentions_are_not_absorbed_into_predicate_concepts() {
        let source = "Elon Musk discussed Tesla.";
        let entities = extract(
            source,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        assert!(entities
            .iter()
            .any(|entity| entity.text == "Elon Musk" && entity.r#type == "PERSON"));
        assert!(entities
            .iter()
            .any(|entity| entity.text == "Tesla" && entity.r#type == "BRAND"));
        assert!(!entities.iter().any(|entity| {
            entity.text == "discussed Tesla" && entity.r#type == "VISUAL_CONCEPT"
        }));
        for entity in &entities {
            assert_eq!(&source[entity.start..entity.end], entity.text);
            assert_eq!(entity.evidence, entity.text);
        }
    }

    #[test]
    fn dotted_initialism_fragments_are_not_emitted_as_entities() {
        let entities = extract(
            "U.S. sales increased.",
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        assert!(!entities
            .iter()
            .any(|entity| entity.text == "U" || entity.text == "S"));
    }

    #[test]
    fn dotted_person_initials_keep_the_full_grounded_name() {
        let source = "J. D. Vance visited Berlin.";
        let entities = extract(
            source,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        let vance = entities.iter().find(|entity| entity.text == "J. D. Vance");
        assert_eq!(
            vance.map(|entity| entity.r#type.as_str()),
            Some("PERSON"),
            "{entities:?}"
        );
        let vance = vance.unwrap();
        assert_eq!(
            source.get(vance.start..vance.end),
            Some(vance.text.as_str())
        );
        assert!(!entities
            .iter()
            .any(|entity| entity.text == "J" || entity.text == "D" || entity.text == "Vance"));
    }

    #[test]
    fn multilingual_scene_function_words_and_named_places_are_typed_safely() {
        for (language, source, expected_place) in [
            (
                "en",
                "The president Emmanuel Macron visited Berlin.",
                "Berlin",
            ),
            ("en", "J. D. Vance visited Berlin.", "Berlin"),
            (
                "it",
                "Il presidente Emmanuel Macron ha visitato Parigi.",
                "Parigi",
            ),
            ("es", "El presidente Emmanuel Macron visitó París.", "París"),
            (
                "pt",
                "O presidente Emmanuel Macron visitou Lisboa.",
                "Lisboa",
            ),
            (
                "fr",
                "Le président Emmanuel Macron a visité Paris.",
                "Paris",
            ),
            (
                "de",
                "Der Präsident Emmanuel Macron besuchte Berlin.",
                "Berlin",
            ),
        ] {
            let entities = extract(
                source,
                &ExtractOptions {
                    language: language.to_string(),
                    entity_count: 20,
                },
            )
            .unwrap();
            assert!(
                entities
                    .iter()
                    .any(|entity| (entity.text == "Emmanuel Macron"
                        || entity.text == "J. D. Vance")
                        && entity.r#type == "PERSON"),
                "{language}: {entities:?}"
            );
            assert!(
                entities
                    .iter()
                    .any(|entity| entity.text == expected_place && entity.r#type == "LOCATION"),
                "{language}: {entities:?}"
            );
            assert!(
                !entities.iter().any(|entity| matches!(
                    entity.text.to_lowercase().as_str(),
                    "il" | "el"
                        | "o"
                        | "le"
                        | "der"
                        | "presidente"
                        | "président"
                        | "visited"
                        | "visitó"
                        | "visitato"
                        | "visitou"
                        | "visité"
                        | "besuchte"
                )),
                "function word leaked for {language}: {entities:?}"
            );
            for entity in &entities {
                assert_eq!(
                    source.get(entity.start..entity.end),
                    Some(entity.text.as_str())
                );
                assert_eq!(entity.evidence, entity.text);
            }
        }
    }

    #[test]
    fn generic_company_nouns_are_not_organizations() {
        let entities = extract(
            "Apple Inc. employed 2,000 people.",
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        assert!(
            entities
                .iter()
                .any(|entity| entity.text == "Apple Inc" && entity.r#type == "ORGANIZATION"),
            "{entities:?}"
        );
        assert!(
            !entities
                .iter()
                .any(|entity| entity.text.eq_ignore_ascii_case("company")
                    || entity.text.eq_ignore_ascii_case("corporation")),
            "{entities:?}"
        );
    }

    #[test]
    fn entity_candidates_remain_verbatim_spans_and_keep_numeric_values() {
        let source = "Tesla reported $2.5 billion in revenue and employed 5,000 workers.";
        let entities = extract(
            source,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        assert!(entities.iter().any(|entity| entity.text == "Tesla"));
        assert!(entities
            .iter()
            .any(|entity| entity.r#type == "MONEY" && entity.text == "$2.5 billion"));
        assert!(entities
            .iter()
            .any(|entity| entity.r#type == "NUMBER" && entity.text == "5,000"));
        for entity in &entities {
            assert_eq!(&source[entity.start..entity.end], entity.text);
            assert_eq!(entity.evidence, entity.text);
        }
    }

    #[test]
    fn typed_entities_use_shared_vocabulary() {
        let entities = extract(
            "Gerard Butler spoke at an event in London. OpenAI released an iPhone.",
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 8,
            },
        )
        .unwrap();
        let find = |name: &str| entities.iter().find(|entity| entity.text == name);
        assert_eq!(
            find("Gerard Butler").map(|entity| entity.r#type.as_str()),
            Some("PERSON")
        );
        assert_eq!(
            find("London").map(|entity| entity.r#type.as_str()),
            Some("LOCATION")
        );
        assert_eq!(
            find("OpenAI").map(|entity| entity.r#type.as_str()),
            Some("BRAND")
        );
        assert_eq!(
            find("iPhone").map(|entity| entity.r#type.as_str()),
            Some("PRODUCT")
        );
    }

    #[test]
    fn extracts_typed_brands_metrics_money_and_dates_with_exact_spans() {
        let source = "OpenAI reported 25% growth, $2.5 million in revenue, 42 orders on March 5, 2026, November 22, 1986 and 03/06/2026.";
        let entities = extract(
            source,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 30,
            },
        )
        .unwrap();
        let find = |kind: &str, text: &str| {
            entities
                .iter()
                .find(|entity| entity.r#type == kind && entity.text == text)
        };
        assert!(
            find("BRAND", "OpenAI").is_some(),
            "brand missing: {entities:?}"
        );
        assert!(
            find("PERCENT", "25%").is_some(),
            "percentage missing: {entities:?}"
        );
        assert!(
            find("MONEY", "$2.5 million in revenue").is_none(),
            "money span should stop at the currency unit: {entities:?}"
        );
        assert!(
            find("MONEY", "$2.5 million").is_some(),
            "money missing: {entities:?}"
        );
        assert!(
            find("NUMBER", "42 orders").is_some(),
            "metric missing: {entities:?}"
        );
        assert!(
            find("DATE", "March 5, 2026").is_some(),
            "month date missing: {entities:?}"
        );
        assert!(
            find("DATE", "03/06/2026").is_some(),
            "numeric date missing: {entities:?}"
        );
        assert!(
            find("DATE", "November 22, 1986").is_some(),
            "month date with multiple spaces must retain the exact year: {entities:?}"
        );
        for entity in entities.iter().filter(|entity| {
            matches!(
                entity.r#type.as_str(),
                "BRAND" | "PERCENT" | "MONEY" | "NUMBER" | "DATE"
            )
        }) {
            assert_eq!(&source[entity.start..entity.end], entity.text);
            assert_eq!(entity.evidence, entity.text);
        }
    }

    #[test]
    fn spelled_numbers_use_locale_rules_units_and_exact_utf8_evidence() {
        for (language, source, surface, kind) in [
            (
                "en",
                "Growth reached twenty-five percent.",
                "twenty-five percent",
                "PERCENT",
            ),
            (
                "it",
                "Vendite pari a venticinque percento.",
                "venticinque percento",
                "PERCENT",
            ),
            (
                "fr",
                "La croissance atteint vingt-cinq pour cent.",
                "vingt-cinq pour cent",
                "PERCENT",
            ),
            (
                "ru",
                "Показатель вырос на двадцать пять процентов.",
                "двадцать пять процентов",
                "PERCENT",
            ),
            (
                "de",
                "Die Firma meldete dreißig Bestellungen.",
                "dreißig Bestellungen",
                "NUMBER",
            ),
        ] {
            let entities = extract(
                source,
                &ExtractOptions {
                    language: language.to_string(),
                    entity_count: 30,
                },
            )
            .unwrap();
            let entity = entities
                .iter()
                .find(|entity| entity.r#type == kind && entity.text == surface);
            assert!(
                entity.is_some(),
                "{language} did not extract {surface:?}: {entities:?}"
            );
            let entity = entity.unwrap();
            assert_eq!(source.get(entity.start..entity.end), Some(surface));
            assert_eq!(entity.evidence, surface);
        }
    }

    #[test]
    fn spelled_number_parser_rejects_ambiguous_words_and_partial_matches() {
        let source =
            "One day later, a runner reached the finish, while twenty-five percent of sales rose.";
        let entities = extract(
            source,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 30,
            },
        )
        .unwrap();
        assert!(
            !entities.iter().any(
                |entity| matches!(entity.r#type.as_str(), "NUMBER" | "ORDINAL")
                    && matches!(entity.text.as_str(), "One" | "one" | "a" | "One day")
            ),
            "ambiguous small words and duration phrases must not be typed as numbers: {entities:?}"
        );
        assert!(
            !entities.iter().any(|entity| entity.text == "twenty-five"),
            "partial parse before a unit must not leak: {entities:?}"
        );
        assert!(
            entities
                .iter()
                .any(|entity| entity.r#type == "PERCENT" && entity.text == "twenty-five percent"),
            "quantity surface missing: {entities:?}"
        );
    }

    #[test]
    fn spelled_number_spans_do_not_absorb_leading_prose() {
        for (language, source, number) in [
            (
                "it",
                "Vendite pari a venticinque percento oggi.",
                "venticinque percento",
            ),
            (
                "en",
                "Revenue reached twenty-five percent today.",
                "twenty-five percent",
            ),
        ] {
            let candidates = crate::spellout::candidates(source, language).unwrap();
            assert_eq!(
                candidates.len(),
                1,
                "unexpected spellout candidates for {language}: {candidates:?}"
            );
            let candidate = &candidates[0];
            assert_eq!(source.get(candidate.start..candidate.end), Some(number));
            assert_eq!(candidate.entity_type, "PERCENT");
        }
    }

    #[test]
    fn language_is_required_and_unsupported_rules_fail_closed() {
        assert!(extract(
            "twenty-five orders",
            &ExtractOptions {
                language: String::new(),
                entity_count: 3
            }
        )
        .is_err());
        let unsupported = extract(
            "Some text.",
            &ExtractOptions {
                language: "zz-ZZ".to_string(),
                entity_count: 3,
            },
        );
        assert!(
            unsupported.is_err(),
            "invalid language tags should fail explicitly"
        );
    }

    #[test]
    fn euro_prefix_keeps_utf8_boundaries_and_june_year_is_a_date() {
        let source = "Revenue was €80,000 in June 1988.";
        let entities = extract(
            source,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 20,
            },
        )
        .unwrap();
        assert!(
            entities
                .iter()
                .any(|entity| entity.r#type == "MONEY" && entity.text == "€80,000"),
            "money span missing: {entities:?}"
        );
        assert!(
            entities
                .iter()
                .any(|entity| entity.r#type == "DATE" && entity.text == "1988"),
            "year missing: {entities:?}"
        );
        assert!(
            entities
                .iter()
                .all(|entity| source.get(entity.start..entity.end) == Some(entity.text.as_str())),
            "invalid UTF-8 evidence span: {entities:?}"
        );
    }

    #[test]
    fn las_vegas_is_a_location_not_a_person() {
        let entities = top3("Mike Tyson fought in Las Vegas.");
        let las_vegas = entities.iter().find(|entity| entity.text == "Las Vegas");
        assert_eq!(
            las_vegas.map(|entity| entity.r#type.as_str()),
            Some("LOCATION")
        );
    }

    #[test]
    fn named_location_survives_a_dense_top_five() {
        let text = "On November 22, 1986, in Las Vegas, he faced Trevor Berbick for the WBC heavyweight title. Tyson won by second-round technical knockout and became the youngest heavyweight world champion.";
        let entities = extract(
            text,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 5,
            },
        )
        .unwrap();
        assert!(
            entities
                .iter()
                .any(|entity| entity.text == "Las Vegas" && entity.r#type == "LOCATION"),
            "a named location must not be crowded out by visual-concept phrases: {entities:?}"
        );
    }

    #[test]
    fn atlantic_city_is_a_location_not_a_person() {
        let entities = top3("In June 1988, Tyson met Michael Spinks in Atlantic City.");
        let atlantic_city = entities
            .iter()
            .find(|entity| entity.text == "Atlantic City");
        assert_eq!(
            atlantic_city.map(|entity| entity.r#type.as_str()),
            Some("LOCATION")
        );
    }

    #[test]
    fn portuguese_geographic_cues_extract_city_and_neighborhood() {
        let text = "Isabelle Caracristi morreu em Fortaleza, no bairro Aldeota.";
        let entities = extract(
            text,
            &ExtractOptions {
                language: "pt".to_string(),
                entity_count: 8,
            },
        )
        .unwrap();
        for place in ["Fortaleza", "Aldeota"] {
            let found = entities
                .iter()
                .find(|entity| entity.text == place)
                .unwrap_or_else(|| panic!("missing {place}: {entities:?}"));
            assert_eq!(found.r#type, "LOCATION", "{found:?}");
            assert_eq!(text.get(found.start..found.end), Some(place));
        }
    }

    #[test]
    fn month_after_location_preposition_is_not_a_place() {
        let entities = extract(
            "The meeting was in May.",
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 8,
            },
        )
        .unwrap();
        assert!(!entities
            .iter()
            .any(|entity| entity.text == "May" && entity.r#type == "LOCATION"));
    }

    #[test]
    fn dolly_places_and_song_titles_do_not_consume_person_slots() {
        let text = "Dolly Parton was born in Sevier County, Tennessee, in the Great Smoky Mountains. Porter Wagoner worked with Dolly Parton. I Will Always Love You became a song.";
        let entities = extract(
            text,
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 10,
            },
        )
        .unwrap();
        let find = |name: &str| entities.iter().find(|entity| entity.text == name);
        assert_eq!(
            find("Dolly Parton").map(|entity| entity.r#type.as_str()),
            Some("PERSON")
        );
        assert_eq!(
            find("Porter Wagoner").map(|entity| entity.r#type.as_str()),
            Some("PERSON")
        );
        assert_eq!(
            find("Sevier County").map(|entity| entity.r#type.as_str()),
            Some("LOCATION")
        );
        assert_eq!(
            find("Great Smoky Mountains").map(|entity| entity.r#type.as_str()),
            Some("LOCATION")
        );
        assert_eq!(
            find("Will Always Love You").map(|entity| entity.r#type.as_str()),
            Some("WORK")
        );
        assert!(!entities.iter().any(|entity| {
            entity.r#type == "PERSON"
                && matches!(
                    entity.text.as_str(),
                    "Sevier County"
                        | "Great Smoky Mountains"
                        | "Will Always Love"
                        | "Will Always Love You"
                )
        }));
    }

    // TestEntitiesRequireSourceEvidence — explicit: an entity not in the
    // source text must never be returned. Synthetic check.
    #[test]
    fn no_entity_without_evidence() {
        let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
        for e in &entities {
            // evidence must be a non-empty verbatim slice of the source.
            assert!(e.start < e.end, "empty span for {}", e.text);
            assert!(!e.evidence.is_empty(), "empty evidence for {}", e.text);
            assert_eq!(
                e.evidence, e.text,
                "evidence must equal text verbatim for {}",
                e.text
            );
        }
    }

    // Determinism: 100 runs must produce the same winner set.
    #[test]
    fn deterministic_across_runs() {
        let text = "Greek salad contains tomatoes, feta cheese and olives.";
        let first = top3(text);
        for _ in 0..99 {
            let again = top3(text);
            assert_eq!(again, first, "non-deterministic extraction result");
        }
    }
}
