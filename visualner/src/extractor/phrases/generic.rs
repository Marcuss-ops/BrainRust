//! Generic noun-phrase scanning.

use super::super::location::{
    has_dotted_initial_prefix, is_phrase_breaking_byte, starts_uppercase,
};
use super::super::names::{is_heading_surface, place_splits_run, proper_name_runs};
use super::super::pos::is_stop_token;
use super::super::stopwords::{is_generic_phrase, is_stop_phrase};
use super::super::tokenizer::Token;
use super::super::values::{classify_value_type, Candidate};
use super::super::verb;

/// Generic phrase scan: maximal title-case or lower-case runs, bounded by
/// punctuation, verbs and the POS gates.
pub(super) fn generic_phrase_candidates(
    tokens: &[Token],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let bytes = source_text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if is_artifact_token(&tokens[i], language) {
            i += 1;
            continue;
        }
        let phrase_start = i;
        if starts_uppercase(&tokens[i].text) && has_dotted_initial_prefix(tokens, bytes, i) {
            i += 1;
            continue;
        }
        let j = if starts_uppercase(&tokens[i].text) {
            proper_phrase_end(tokens, bytes, source_text, language, phrase_start)
        } else {
            common_phrase_end(tokens, source_text, language, phrase_start)
        };
        let mut phrase_end = j;
        // "Visita Pavia" is a clause opener plus a city, not one surface:
        // when a known place closes a title-case phrase whose opener carries
        // no name evidence, the place is scanned as its own phrase (so it
        // survives as a LOCATION) and a one-token fragment before it is the
        // clause opener, not a concept.
        let split_place = if starts_uppercase(&tokens[phrase_start].text) {
            place_splits_run(tokens, language, phrase_start, phrase_end)
        } else {
            None
        };
        if let Some(split) = split_place {
            phrase_end = split;
        }
        if split_place.is_some() && phrase_end - phrase_start < 2 {
            i = phrase_end;
            continue;
        }
        if is_ordinal_fragment(tokens, phrase_start, phrase_end) {
            i = skip_ordinal_fragment(tokens, language, phrase_end);
            continue;
        }
        if !phrase_name_runs_allowed(tokens, bytes, language, phrase_start, phrase_end) {
            i = phrase_end;
            continue;
        }
        let start = tokens[phrase_start].start;
        let end = tokens[phrase_end - 1].end;
        let text = source_text[start..end].to_string();
        let normalized = text.to_lowercase();
        // Reject phrases whose normalized surface is a stop phrase, a
        // generic phrase, or a contextual verbal predicate ("e sorge nella
        // pianura"): GEN-01 keeps verbs out of the entity set without a
        // lexical blacklist, so nominal readings of the same surface
        // ("La guarda del castello") stay extractable.
        if !is_stop_phrase(&normalized)
            && !is_generic_phrase(&normalized)
            && !is_heading_surface(&text)
            && !verb::is_verbal_predicate(tokens, phrase_start, phrase_end, language)
        {
            out.push(Candidate {
                text,
                start,
                end,
                entity_type: None,
            });
        }
        i = phrase_end;
        // The finite verb that broke the phrase is not candidate material.
        if i < tokens.len() && verb::is_verbal_continuation(tokens, i, language) {
            i += 1;
        }
    }
    out
}

/// A lone capital letter between periods is a tokenization artifact of dotted
/// initialisms (e.g. U.S.), not an entity span; a stop token is never phrase
/// material either.
fn is_artifact_token(token: &Token, language: &str) -> bool {
    is_stop_token(token, language)
        || (token.text.len() == 1 && token.text.as_bytes()[0].is_ascii_uppercase())
}

/// Extends a title-case run, stopping at an organization→person transition,
/// at a proper name in apposition after a common noun, and at punctuation.
fn proper_phrase_end(
    tokens: &[Token],
    bytes: &[u8],
    source_text: &str,
    language: &str,
    phrase_start: usize,
) -> usize {
    let mut j = phrase_start;
    while j < tokens.len() && starts_uppercase(&tokens[j].text) {
        if j > phrase_start {
            let prefix = &source_text[tokens[phrase_start].start..tokens[j - 1].end];
            let suffix_end = (j..tokens.len())
                .take_while(|index| starts_uppercase(&tokens[*index].text))
                .last()
                .map_or(j, |index| index + 1);
            let suffix = &source_text[tokens[j].start..tokens[suffix_end - 1].end];
            let prefix_kind = classify_value_type(prefix, true, true, true, language);
            let suffix_kind = classify_value_type(suffix, true, true, true, language);
            if prefix_kind == "ORG" && suffix_kind == "PERSON" {
                break;
            }
            // A common noun that closes an institution/place phrase cannot
            // absorb the following proper name in apposition:
            // "Repubblica Sergio Mattarella" is two spans.
            if tokens[phrase_start].upos.as_deref() != Some("PROPN")
                && tokens[j].upos.as_deref() == Some("PROPN")
            {
                break;
            }
            let gap = &bytes[tokens[j - 1].end..tokens[j].start];
            if gap.iter().any(|b| is_phrase_breaking_byte(*b)) {
                break;
            }
        }
        j += 1;
    }
    j
}

/// Extends a lower-case run until punctuation, a title-cased token or a finite
/// verbal predicate.
fn common_phrase_end(
    tokens: &[Token],
    source_text: &str,
    language: &str,
    phrase_start: usize,
) -> usize {
    let bytes = source_text.as_bytes();
    let mut j = phrase_start;
    while j < tokens.len() && !is_stop_token(&tokens[j], language) {
        if j > phrase_start {
            let gap = &bytes[tokens[j - 1].end..tokens[j].start];
            if gap.iter().any(|b| is_phrase_breaking_byte(*b)) || starts_uppercase(&tokens[j].text)
            {
                // A lower-case clause/predicate must not absorb the next
                // title-cased name into a VISUAL_CONCEPT span (e.g.
                // "discussed Tesla"). Let the proper-name candidate own the
                // capitalized token instead.
                break;
            }
            // A finite verbal predicate continues the clause, not the noun
            // phrase ("I tifosi festeggiano sugli spalti"): the verb breaks
            // the phrase here and is skipped below, so it can never glue to a
            // subject or become an entity.
            if verb::is_verbal_continuation(tokens, j, language) {
                break;
            }
        }
        j += 1;
    }
    j
}

fn is_ordinal_fragment(tokens: &[Token], phrase_start: usize, phrase_end: usize) -> bool {
    phrase_end == phrase_start + 1
        && starts_uppercase(&tokens[phrase_start].text)
        && matches!(tokens[phrase_start].upos.as_deref(), Some("ADJ" | "NUM"))
        && tokens.get(phrase_end).is_some_and(|next| {
            !starts_uppercase(&next.text) && matches!(next.upos.as_deref(), Some("NOUN" | "PROPN"))
        })
}

/// Skips the lowercase noun the ordinal fragment belongs to, plus the verb
/// that would otherwise be re-scanned.
fn skip_ordinal_fragment(tokens: &[Token], language: &str, phrase_end: usize) -> usize {
    let mut i = phrase_end;
    while i < tokens.len()
        && !starts_uppercase(&tokens[i].text)
        && !is_stop_token(&tokens[i], language)
        && !verb::is_verbal_continuation(tokens, i, language)
    {
        i += 1;
    }
    if i == phrase_end && i < tokens.len() {
        i += 1;
    }
    i
}

/// A run that contains an inner proper-name run but is not itself a full name
/// run is a clause fragment, not a candidate.
fn phrase_name_runs_allowed(
    tokens: &[Token],
    bytes: &[u8],
    language: &str,
    phrase_start: usize,
    phrase_end: usize,
) -> bool {
    let phrase_runs = proper_name_runs(&tokens[phrase_start..phrase_end], bytes, language);
    let has_inner_run = phrase_runs.iter().any(|(run_start, _)| *run_start > 0);
    let is_full_run = phrase_runs
        .iter()
        .any(|(run_start, run_end)| *run_start == 0 && *run_end == phrase_end - phrase_start);
    !(has_inner_run && !is_full_run)
}
