//! Localized identity matching.
//!
//! A translated scene is analysed on its own text, so its entity surface is
//! whatever the translator wrote ("Mike’a Tysona"). The canonical identity —
//! id, type, image binding — belongs to the SOURCE annotation and must not be
//! re-minted from that surface. This module grounds every source entity in the
//! translated text and returns rune spans; the caller projects the identity.
//!
//! Matching is token-based and case-insensitive. A person name additionally
//! accepts one closed-list grammatical suffix for pl/de/tr, so an inflected
//! surface cannot mint a second identity. The ru/tr/pl ordered fallback is
//! deliberately narrow: it only completes people when the translated NER has
//! exactly one non-overlapping candidate per still-unmatched source person.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceEntity {
    pub kind: String,
    #[serde(default)]
    pub canonical_name: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VisualPerson {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IdentityMatch {
    pub source_index: usize,
    pub kind: String,
    pub start_rune: usize,
    pub end_rune: usize,
    pub surface: String,
}

#[derive(Debug, Clone, Copy)]
struct NameToken<'a> {
    start_byte: usize,
    end_byte: usize,
    text: &'a str,
}

fn fold(value: &str) -> String {
    value.nfc().collect::<String>().to_lowercase()
}

fn fold_eq(left: &str, right: &str) -> bool {
    fold(left) == fold(right)
}

fn rune_offset(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

fn span(text: &str, first: NameToken<'_>, last: NameToken<'_>) -> (usize, usize, String) {
    (
        rune_offset(text, first.start_byte),
        rune_offset(text, last.end_byte),
        text[first.start_byte..last.end_byte].to_string(),
    )
}

/// Splits text into word tokens (letters/digits, with an internal apostrophe
/// kept) while preserving byte offsets so the span can be projected back onto
/// rune coordinates in the original text.
fn name_tokens(text: &str) -> Vec<NameToken<'_>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (offset, character) in text.char_indices() {
        let inside = start.is_some();
        if character.is_alphabetic()
            || character.is_numeric()
            || ((character == '\'' || character == '’') && inside)
        {
            if !inside {
                start = Some(offset);
            }
            continue;
        }
        if let Some(begin) = start.take() {
            out.push(NameToken {
                start_byte: begin,
                end_byte: offset,
                text: &text[begin..offset],
            });
        }
    }
    if let Some(begin) = start {
        out.push(NameToken {
            start_byte: begin,
            end_byte: text.len(),
            text: &text[begin..],
        });
    }
    out
}

fn is_name_token(token: &str) -> bool {
    let mut characters = token.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_uppercase() {
        return false;
    }
    token
        .chars()
        .all(|character| character.is_alphabetic() || matches!(character, '\'' | '’' | '-' | '.'))
}

/// Complete proper-name suffixes of one normalized identity, longest first.
/// The caller tries them against the translated text after the full identity.
fn person_name_runs(value: &str) -> Vec<Vec<String>> {
    let mut runs = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let flush = |runs: &mut Vec<Vec<String>>, current: &mut Vec<String>| {
        if !current.is_empty() {
            runs.push(std::mem::take(current));
        }
    };
    for word in value.split_whitespace() {
        let token = word.trim_matches(|character: char| {
            matches!(
                character,
                '.' | ','
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '"'
                    | '“'
                    | '”'
                    | '\''
                    | '’'
            )
        });
        if is_name_token(token) {
            current.push(token.to_string());
        } else {
            flush(&mut runs, &mut current);
        }
    }
    flush(&mut runs, &mut current);
    runs
}

/// Removes the editorial lead-in and a possessive suffix that the source
/// mention may carry, without changing the canonical spelling.
fn normalize_person_name(value: &str) -> String {
    let mut value = value.trim();
    for prefix in ["While ", "while "] {
        if let Some(stripped) = value.strip_prefix(prefix) {
            value = stripped.trim();
            break;
        }
    }
    for suffix in ["'s", "’s"] {
        if value.len() > suffix.len()
            && value
                .get(value.len() - suffix.len()..)
                .map(|tail| fold_eq(tail, suffix))
                .unwrap_or(false)
        {
            return value[..value.len() - suffix.len()].trim().to_string();
        }
    }
    value.to_string()
}

fn find_exact_name_span(text: &str, candidate: &str) -> Option<(usize, usize, String)> {
    let want = name_tokens(candidate);
    let have = name_tokens(text);
    if want.is_empty() || have.len() < want.len() {
        return None;
    }
    for start in 0..=have.len() - want.len() {
        if want
            .iter()
            .zip(&have[start..start + want.len()])
            .all(|(left, right)| fold_eq(left.text, right.text))
        {
            let first = have[start];
            let last = have[start + want.len() - 1];
            return Some(span(text, first, last));
        }
    }
    None
}

fn inflected_token_matches(language: &str, canonical: &str, surface: &str) -> bool {
    let canonical = fold(canonical);
    let surface = fold(surface);
    if canonical == surface {
        return true;
    }
    if canonical.chars().count() < 3 || !surface.starts_with(&canonical) {
        return false;
    }
    let suffix = &surface[canonical.len()..];
    match language.trim().to_lowercase().as_str() {
        "pl" => matches!(
            suffix,
            "a" | "ie"
                | "y"
                | "i"
                | "u"
                | "owi"
                | "em"
                | "ą"
                | "ę"
                | "om"
                | "ami"
                | "ach"
                | "ów"
                | "’a"
                | "'a"
        ),
        "de" => suffix == "s",
        "tr" => matches!(
            suffix,
            "'ın"
                | "'in"
                | "'un"
                | "'ün"
                | "'a"
                | "'e"
                | "'ı"
                | "'i"
                | "'u"
                | "'ü"
                | "ın"
                | "in"
                | "un"
                | "ün"
                | "a"
                | "e"
                | "ı"
                | "i"
                | "u"
                | "ü"
        ),
        _ => false,
    }
}

fn find_inflected_person_span(
    text: &str,
    canonical: &str,
    language: &str,
) -> Option<(usize, usize, String)> {
    let want = name_tokens(canonical);
    let have = name_tokens(text);
    if want.len() < 2 || have.len() < want.len() {
        return None;
    }
    for start in 0..=have.len() - want.len() {
        if want
            .iter()
            .zip(&have[start..start + want.len()])
            .all(|(left, right)| inflected_token_matches(language, left.text, right.text))
        {
            let first = have[start];
            let last = have[start + want.len() - 1];
            return Some(span(text, first, last));
        }
    }
    None
}

fn first_non_empty<'a>(left: &'a str, right: &'a str) -> &'a str {
    if !left.trim().is_empty() {
        left
    } else {
        right
    }
}

/// Grounds every source entity in the translated text. An entity that does not
/// occur produces no match; nothing is invented.
pub fn match_localized_sources(
    text: &str,
    language: &str,
    sources: &[SourceEntity],
) -> Vec<IdentityMatch> {
    let mut out = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let kind = source.kind.trim();
        if kind.is_empty() {
            continue;
        }
        let identity = first_non_empty(&source.canonical_name, &source.text).trim();
        if identity.is_empty() {
            continue;
        }
        let mut aliases: Vec<String> = vec![identity.to_string()];
        if kind.eq_ignore_ascii_case("PERSON") {
            let identity = normalize_person_name(identity);
            aliases = vec![identity.clone()];
            for run in person_name_runs(&identity) {
                for start in 1..run.len() {
                    aliases.push(run[start..].join(" "));
                }
            }
        }
        for alias in aliases {
            let mut found = find_exact_name_span(text, &alias);
            if found.is_none()
                && kind.eq_ignore_ascii_case("PERSON")
                && matches!(language.trim().to_lowercase().as_str(), "pl" | "de" | "tr")
            {
                found = find_inflected_person_span(text, &alias, language);
            }
            if let Some((start_rune, end_rune, surface)) = found {
                if surface.trim().is_empty() {
                    continue;
                }
                out.push(IdentityMatch {
                    source_index: index,
                    kind: kind.to_string(),
                    start_rune,
                    end_rune,
                    surface,
                });
                break;
            }
        }
    }
    out
}

fn overlaps(left: &IdentityMatch, right: &IdentityMatch) -> bool {
    left.start_rune < right.end_rune && right.start_rune < left.end_rune
}

/// Completes person identities whose translated writing system changed. The
/// caller passes the still-unmatched source people and the translated NER
/// PERSON surfaces; completion happens only when the counts match, so the
/// fallback cannot invent a person.
pub fn complete_ordered_person_matches(
    text: &str,
    language: &str,
    sources: &[SourceEntity],
    visual: &[VisualPerson],
    existing: &[IdentityMatch],
) -> Vec<IdentityMatch> {
    if !matches!(language.trim().to_lowercase().as_str(), "ru" | "tr" | "pl") {
        return existing.to_vec();
    }
    let mut people: Vec<usize> = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        if !source.kind.trim().eq_ignore_ascii_case("PERSON") {
            continue;
        }
        if existing.iter().any(|matched| matched.source_index == index) {
            continue;
        }
        people.push(index);
    }
    if people.is_empty() {
        return existing.to_vec();
    }

    let mut candidates: Vec<(usize, usize, String)> = Vec::new();
    for person in visual {
        let text_value = person.text.trim();
        if text_value.is_empty() {
            continue;
        }
        let Some((start_rune, end_rune, surface)) = find_exact_name_span(text, text_value) else {
            continue;
        };
        if end_rune <= start_rune {
            continue;
        }
        let candidate = IdentityMatch {
            source_index: 0,
            kind: "PERSON".to_string(),
            start_rune,
            end_rune,
            surface,
        };
        if existing.iter().any(|matched| overlaps(&candidate, matched)) {
            continue;
        }
        candidates.push((start_rune, end_rune, candidate.surface));
    }
    if candidates.len() != people.len() {
        return existing.to_vec();
    }
    candidates.sort_by_key(|candidate| candidate.0);

    let mut out = existing.to_vec();
    for (position, (start_rune, end_rune, surface)) in candidates.into_iter().enumerate() {
        out.push(IdentityMatch {
            source_index: people[position],
            kind: "PERSON".to_string(),
            start_rune,
            end_rune,
            surface,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: &str, canonical: &str) -> SourceEntity {
        SourceEntity {
            kind: kind.to_string(),
            canonical_name: canonical.to_string(),
            text: canonical.to_string(),
        }
    }

    #[test]
    fn exact_match_is_case_and_punctuation_insensitive() {
        let matches = match_localized_sources(
            "Der Trainer: mike TYSON, heute.",
            "de",
            &[source("PERSON", "Mike Tyson")],
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].surface, "mike TYSON");
        assert_eq!(matches[0].start_rune, 13);
        assert_eq!(matches[0].end_rune, 23);
    }

    #[test]
    fn exact_matching_is_available_for_every_shared_language() {
        for language in ["de", "en", "es", "fr", "it", "pt"] {
            let matches = match_localized_sources(
                "Mike Tyson was here.",
                language,
                &[source("PERSON", "Mike Tyson")],
            );
            assert_eq!(matches.len(), 1, "{language} exact match");
            assert_eq!(matches[0].surface, "Mike Tyson");
        }
    }

    #[test]
    fn polish_inflection_is_a_surface_of_the_same_person() {
        let matches = match_localized_sources(
            "To opowieść o Mike’a Tysona.",
            "pl",
            &[source("PERSON", "Mike Tyson")],
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].surface, "Mike’a Tysona");
    }

    #[test]
    fn turkish_case_suffix_is_a_surface_of_the_same_person() {
        let matches = match_localized_sources(
            "Bu hikaye Mike Tyson'a ait.",
            "tr",
            &[source("PERSON", "Mike Tyson")],
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].surface, "Mike Tyson'a");
    }

    #[test]
    fn german_possessive_suffix_is_a_surface_of_the_same_person() {
        let matches = match_localized_sources(
            "Das ist Mike Tysons Geschichte.",
            "de",
            &[source("PERSON", "Mike Tyson")],
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].surface, "Mike Tysons");
    }

    #[test]
    fn a_single_token_name_is_never_inflected() {
        let matches = match_localized_sources("Tysona.", "pl", &[source("PERSON", "Tyson")]);
        assert!(matches.is_empty());
    }

    #[test]
    fn non_person_kinds_match_exactly_only() {
        let matches = match_localized_sources(
            "New York e New Yorka.",
            "it",
            &[source("LOCATION", "New York")],
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].surface, "New York");
    }

    #[test]
    fn ordered_fallback_completes_people_only_when_counts_match() {
        let sources = [
            source("PERSON", "Mike Tyson"),
            source("PERSON", "Ivan Petrov"),
        ];
        let visual = [
            VisualPerson {
                text: "Майк Тайсон".to_string(),
            },
            VisualPerson {
                text: "Ивана Петрова".to_string(),
            },
        ];
        let matches = complete_ordered_person_matches(
            "Майк Тайсон встретил Ивана Петрова.",
            "ru",
            &sources,
            &visual,
            &[],
        );
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].surface, "Майк Тайсон");
        assert_eq!(matches[1].surface, "Ивана Петрова");
    }

    #[test]
    fn ordered_fallback_refuses_a_count_mismatch() {
        let sources = [
            source("PERSON", "Mike Tyson"),
            source("PERSON", "Ivan Petrov"),
        ];
        let visual = [VisualPerson {
            text: "Майк Тайсон".to_string(),
        }];
        let matches = complete_ordered_person_matches(
            "Майк Тайсон встретил кого-то.",
            "ru",
            &sources,
            &visual,
            &[],
        );
        assert!(matches.is_empty());
    }

    #[test]
    fn ordered_fallback_skips_already_matched_people() {
        let sources = [source("PERSON", "Mike Tyson")];
        let visual = [VisualPerson {
            text: "Майк Тайсон".to_string(),
        }];
        let existing = vec![IdentityMatch {
            source_index: 0,
            kind: "PERSON".to_string(),
            start_rune: 0,
            end_rune: 10,
            surface: "Майк Тайсон".to_string(),
        }];
        let matches =
            complete_ordered_person_matches("Майк Тайсон.", "ru", &sources, &visual, &existing);
        assert_eq!(matches, existing);
    }
}
