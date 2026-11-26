//! Candidate construction: object lexicon, noun phrases and the POS visual gate.

use super::location::starts_uppercase;
use super::tokenizer::Token;
use super::values::Candidate;

mod family;
mod generic;
mod named;
mod works;

pub(crate) use family::type_relation_introduced_people;
pub(crate) use works::{delimited_work_candidates, work_title_candidates};

/// Keeps a generic phrase only when the POS model still sees a noun phrase:
/// a clause that opens with a verb, auxiliary or pronoun ("c'è nulla") or an
/// adverb stack ("ancora oggi") is not a visual subject. Typed candidates
/// (objects, values, names) are not affected, and without a model every
/// candidate is kept, so the model-free path is unchanged.
pub(crate) fn keeps_as_visual_candidate(
    candidate: &Candidate,
    tokens: &[Token],
    language: &str,
) -> bool {
    if candidate.entity_type.is_some() {
        return true;
    }
    // A predicate or function word opens a clause, never a visual phrase.
    // UDPipe tags common names as PRON ("OpenAI"), so a pronoun only counts
    // when the orthography still marks it as a name.
    // A name can be mistagged as a pronoun, but a pronoun cannot be talked
    // into a name: the language pack's function inventory still rules it out,
    // which is what keeps a sentence-initial "Ciò" or "Noi" out while
    // "OpenAI" (absent from every inventory) stays.
    let pronoun_is_name = |token: &Token| {
        if !semantic_core::lexicon::lexicon()
            .language_profiles
            .is_cased(language)
            || !starts_uppercase(&token.text)
        {
            return false;
        }
        let lower = token.text.to_lowercase();
        // A capitalized surface the pack lists as a function word is still a
        // function word: "C'è" and "Ciò" open a sentence, they do not name
        // anything.
        !semantic_core::language_packs::get(language)
            .is_some_and(|pack| pack.is_function_word(&lower) || pack.is_stopword(&lower))
    };
    let opens_a_clause = |token: &Token| match token.upos.as_deref() {
        Some("VERB" | "AUX" | "SCONJ" | "CCONJ" | "INTJ" | "DET" | "ADP") => true,
        Some("PRON") => !pronoun_is_name(token),
        _ => false,
    };
    let span: Vec<&Token> = tokens
        .iter()
        .filter(|token| token.start >= candidate.start && token.end <= candidate.end)
        .collect();
    if span.is_empty() || span.iter().all(|token| token.upos.is_none()) {
        return true;
    }
    if opens_a_clause(span[0]) {
        return false;
    }
    // A predicate inside the phrase means the candidate spans a clause
    // ("anni sarebbe tornato"); a trailing participle is the only form that
    // still modifies a noun ("condanna anticipata"). An interior preposition,
    // article or pronoun is not a clause: "Università di Torino",
    // "Bank of England" and "Carlo il Grosso" are names, and the opener above
    // already rejected a phrase that *starts* with a function word.
    let last = span.len() - 1;
    let predicate_inside = span.iter().enumerate().any(|(index, token)| {
        index != last
            && matches!(
                token.upos.as_deref(),
                Some("VERB" | "AUX" | "SCONJ" | "CCONJ" | "INTJ")
            )
    });
    if predicate_inside {
        return false;
    }
    // An adverb or adjective stack without any noun ("ancora oggi") is not a
    // subject either. Unknown tags (X) and adjectives are kept: UDPipe tags
    // many proper names and brands as X or ADJ, and refusing them here would
    // drop real identities the typers can still recognize later.
    span.iter().any(|token| match token.upos.as_deref() {
        Some("NOUN" | "PROPN" | "NUM" | "ADJ" | "X") => true,
        Some("PRON") => pronoun_is_name(token),
        _ => false,
    })
}
/// Adds typed candidates for a small, high-precision visual-object vocabulary.
/// This lexical backstop catches common objects even when a POS model tags the
/// noun unreliably; it still uses exact tokenizer spans and never rewrites the
/// source surface. The vocabulary is deliberately explicit and expandable
/// from corpus evidence rather than guessing that every noun is an object.
pub(crate) fn object_lexicon_candidates(
    tokens: &[Token],
    token_lower: &[String],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let terms = semantic_core::lexicon::lexicon().object_terms(language);
    if terms.is_empty() {
        return Vec::new();
    }
    // One membership set per call instead of a linear scan of the whole
    // vocabulary for every 1–3 gram.
    let term_set: std::collections::HashSet<&str> = terms.iter().map(String::as_str).collect();
    let mut candidates = Vec::new();
    for start_index in 0..tokens.len() {
        let mut normalized = String::new();
        let mut lemma_normalized = String::new();
        let max_end = (start_index + 3).min(tokens.len());
        for end_index in start_index..max_end {
            if end_index > start_index {
                let gap = &source_text[tokens[end_index - 1].end..tokens[end_index].start];
                if gap.is_empty() || !gap.chars().all(char::is_whitespace) {
                    break;
                }
                normalized.push(' ');
                lemma_normalized.push(' ');
            }
            normalized.push_str(&token_lower[end_index]);
            lemma_normalized.push_str(&object_term_key(&tokens[end_index]));
            let mut object_start = tokens[start_index].start;
            let lexicon_match = term_set.contains(normalized.as_str())
                || term_set.contains(lemma_normalized.as_str());
            let elided_match = if start_index == end_index && !lexicon_match {
                token_lower[start_index]
                    .rfind(['\'', '\u{2019}'])
                    .map(|apostrophe| {
                        let suffix_start = apostrophe
                            + token_lower[start_index][apostrophe..]
                                .chars()
                                .next()
                                .map_or(1, char::len_utf8);
                        let suffix = &token_lower[start_index][suffix_start..];
                        if term_set.contains(suffix) {
                            object_start = tokens[start_index].start + suffix_start;
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false)
            } else {
                false
            };
            if !lexicon_match && !elided_match {
                continue;
            }
            let start = object_start;
            let end = tokens[end_index].end;
            let Some(text) = source_text.get(start..end) else {
                continue;
            };
            candidates.push(Candidate {
                text: text.to_string(),
                start,
                end,
                entity_type: Some("OBJECT".to_string()),
            });
        }
    }
    candidates
}

/// Dictionary key for an object token: UDPipe's lemma when present, otherwise
/// the case-folded surface. The original source span is never rewritten.
fn object_term_key(token: &Token) -> String {
    token
        .lemma
        .as_deref()
        .map(str::to_lowercase)
        .filter(|lemma| !lemma.is_empty() && lemma != "_" && !lemma.contains(char::is_whitespace))
        .unwrap_or_else(|| token.text.to_lowercase())
}

/// A multi-word noun phrase inherits OBJECT from its syntactic head. This
/// covers Italian "sensori di umidità" and "pannelli fotovoltaici" without
/// adding every combination to the vocabulary. The head is the span token
/// whose UDPipe head is outside the span (or the root); without dependency
/// data the candidate is left untouched.
pub(crate) fn type_object_head_nouns(
    candidates: &mut [Candidate],
    tokens: &[Token],
    language: &str,
) {
    let terms = semantic_core::lexicon::lexicon().object_terms(language);
    if terms.is_empty() {
        return;
    }
    for candidate in candidates.iter_mut() {
        if !matches!(
            candidate.entity_type.as_deref(),
            None | Some("VISUAL_CONCEPT")
        ) {
            continue;
        }
        let span: Vec<&Token> = tokens
            .iter()
            .filter(|token| token.start >= candidate.start && token.end <= candidate.end)
            .collect();
        if span.len() < 2 {
            continue;
        }
        let ids: std::collections::HashSet<i32> =
            span.iter().filter_map(|token| token.id).collect();
        let has_object_head = span.iter().any(|token| {
            let Some(head) = token.head else {
                return false;
            };
            if head != 0 && ids.contains(&head) {
                return false;
            }
            if !matches!(token.upos.as_deref(), Some("NOUN" | "PROPN")) {
                return false;
            }
            let lemma = object_term_key(token);
            terms
                .iter()
                .any(|term| term == &lemma || term.eq_ignore_ascii_case(&token.text))
        });
        if has_object_head {
            candidate.entity_type = Some("OBJECT".to_string());
        }
    }
}

/// noun_phrase_candidates groups adjacent non-stopword tokens into maximal
/// noun phrases. A phrase is one or more consecutive tokens whose lowercase
/// surface is not a stopword, AND that are separated only by whitespace
/// (a comma, period, semicolon or other punctuation between two tokens is a
/// hard phrase boundary). Multi-word phrases ("feta cheese",
/// "olive oil") score higher than singletons, which is what makes concrete
/// multi-word objects win over generic singletons.
///
/// The pool is built in two stages that must keep their order: grounded names
/// first, then the generic scan, so the scan can never absorb or truncate a
/// name.
pub(crate) fn noun_phrase_candidates(
    tokens: &[Token],
    token_lower: &[String],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let mut out = named::named_candidates(tokens, token_lower, source_text, language);
    out.extend(generic::generic_phrase_candidates(
        tokens,
        source_text,
        language,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, start: usize, upos: &str, lemma: &str, id: i32, head: i32) -> Token {
        Token {
            text: text.to_string(),
            start,
            end: start + text.len(),
            upos: Some(upos.to_string()),
            lemma: Some(lemma.to_string()),
            features: None,
            dependency: None,
            id: Some(id),
            head: Some(head),
            sentence_id: Some(1),
        }
    }

    #[test]
    fn an_inflected_object_matches_its_lemma() {
        let source = "Siatkę";
        let tokens = vec![token(source, 0, "NOUN", "siatka", 1, 0)];
        let candidates = object_lexicon_candidates(&tokens, &["siatkę".to_string()], source, "pl");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].text, source);
        assert_eq!(candidates[0].entity_type.as_deref(), Some("OBJECT"));
    }

    #[test]
    fn a_multiword_object_inherits_the_type_of_its_head() {
        let source = "sensori di umidità";
        let tokens = vec![
            token("sensori", 0, "NOUN", "sensore", 1, 99),
            token("di", 8, "ADP", "di", 2, 1),
            token("umidità", 11, "NOUN", "umidità", 3, 1),
        ];
        let mut candidates = vec![Candidate {
            text: source.to_string(),
            start: 0,
            end: source.len(),
            entity_type: Some("VISUAL_CONCEPT".to_string()),
        }];
        type_object_head_nouns(&mut candidates, &tokens, "it");
        assert_eq!(candidates[0].entity_type.as_deref(), Some("OBJECT"));
    }

    #[test]
    fn a_span_without_dependency_data_keeps_its_visual_type() {
        let source = "sensori di umidità";
        let tokens = vec![
            token("sensori", 0, "NOUN", "sensore", 1, 1),
            token("di", 8, "ADP", "di", 2, 1),
            token("umidità", 11, "NOUN", "umidità", 3, 1),
        ];
        let mut candidates = vec![Candidate {
            text: source.to_string(),
            start: 0,
            end: source.len(),
            entity_type: Some("VISUAL_CONCEPT".to_string()),
        }];
        type_object_head_nouns(&mut candidates, &tokens, "it");
        assert_eq!(candidates[0].entity_type.as_deref(), Some("VISUAL_CONCEPT"));
    }
}
