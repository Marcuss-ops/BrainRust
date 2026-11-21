//! Candidate construction: object lexicon, noun phrases and the POS visual gate.

use super::location::{has_dotted_initial_prefix, is_phrase_breaking_byte, starts_uppercase};
use super::names::{
    candidate_from_tokens, curated_name_span, is_heading_surface, nested_institution_places,
    organization_name_runs, place_splits_run, proper_name_runs,
};
use super::pos::is_stop_token;
use super::stopwords::{is_generic_phrase, is_stop_phrase};
use super::tokenizer::Token;
use super::values::{classify_value_type, Candidate};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const FAMILY_RELATIONS: &str = include_str!("../../../lexicon/family_relations.json");
static FAMILY_CUE_HEADS: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();

#[derive(serde::Deserialize)]
struct FamilyLexicon {
    languages: BTreeMap<String, Vec<FamilyCue>>,
}

#[derive(serde::Deserialize)]
struct FamilyCue {
    surface: String,
}

fn family_cue_heads() -> &'static BTreeMap<String, BTreeSet<String>> {
    FAMILY_CUE_HEADS.get_or_init(|| {
        let parsed: FamilyLexicon = serde_json::from_str(FAMILY_RELATIONS)
            .expect("family relation lexicon must be valid JSON");
        parsed
            .languages
            .into_iter()
            .map(|(language, cues)| {
                let heads = cues
                    .into_iter()
                    .filter_map(|cue| cue.surface.split_whitespace().next().map(str::to_lowercase))
                    .collect();
                (language, heads)
            })
            .collect()
    })
}

/// Type a single proper name as PERSON when a family-relation cue introduces
/// it. This relies on the relation lexicon rather than a list of given names.
pub(crate) fn type_relation_introduced_people(
    candidates: &mut [Candidate],
    tokens: &[Token],
    source: &str,
    language: &str,
) {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let Some(cues) = family_cue_heads().get(base) else {
        return;
    };
    for candidate in candidates.iter_mut().filter(|candidate| {
        candidate.entity_type.is_none() && candidate.text.split_whitespace().count() == 1
    }) {
        let Some(index) = tokens
            .iter()
            .position(|token| token.start == candidate.start && token.end == candidate.end)
        else {
            continue;
        };
        if tokens[index].upos.as_deref() != Some("PROPN") || index == 0 {
            continue;
        }
        let sentence_id = tokens[index].sentence_id;
        let from = index.saturating_sub(4);
        let introduced = (from..index).rev().any(|cue_index| {
            if tokens[cue_index].sentence_id != sentence_id {
                return false;
            }
            let gap = &source[tokens[cue_index].end..tokens[index].start];
            if gap
                .chars()
                .any(|ch| matches!(ch, '.' | ';' | ':' | '!' | '?'))
            {
                return false;
            }
            cues.contains(&tokens[cue_index].text.to_lowercase())
        });
        if introduced {
            candidate.entity_type = Some("PERSON".to_string());
        }
    }
}
use super::verb;

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
        let max_end = (start_index + 3).min(tokens.len());
        for end_index in start_index..max_end {
            if end_index > start_index {
                let gap = &source_text[tokens[end_index - 1].end..tokens[end_index].start];
                if gap.is_empty() || !gap.chars().all(char::is_whitespace) {
                    break;
                }
                normalized.push(' ');
            }
            normalized.push_str(&token_lower[end_index]);
            let mut object_start = tokens[start_index].start;
            let lexicon_match = term_set.contains(normalized.as_str());
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

/// noun_phrase_candidates groups adjacent non-stopword tokens into maximal
/// noun phrases. A phrase is one or more consecutive tokens whose lowercase
/// surface is not a stopword, AND that are separated only by whitespace
/// (a comma, period, semicolon or other punctuation between two tokens is a
/// hard phrase boundary). Multi-word phrases ("feta cheese",
/// "olive oil") score higher than singletons, which is what makes concrete
/// multi-word objects win over generic singletons.
pub(crate) fn noun_phrase_candidates(
    tokens: &[Token],
    token_lower: &[String],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let bytes = source_text.as_bytes();
    let mut out = Vec::new();
    // Named people/organizations are bounded to the contiguous title-case run.
    // This prevents lower-case lead-ins such as "figures like Phil Jackson"
    // from becoming a PERSON entity; the grounded candidate is "Phil Jackson".
    let mut name_runs = proper_name_runs(tokens, bytes, language);
    let organization_runs = organization_name_runs(tokens, bytes, language);
    name_runs.retain(|(start, end)| {
        !organization_runs
            .iter()
            .any(|(org_start, org_end)| org_start <= start && end <= org_end)
    });
    name_runs.extend(organization_runs);
    name_runs.sort_unstable();
    name_runs.dedup();
    for (start, end) in name_runs {
        let mut candidate = candidate_from_tokens(tokens, source_text, start, end);
        let capitalized_words = tokens[start..end]
            .iter()
            .filter(|token| starts_uppercase(&token.text))
            .count();
        if capitalized_words >= 2
            && semantic_core::lexicon::is_curated_given_name(language, &tokens[start].text)
            && !semantic_core::lexicon::lexicon().is_place(&candidate.text, language)
            && super::values::institution_cue_kind(&candidate.text, language).is_none()
            && !super::names::opens_organization_name(&tokens[start].text, language)
        {
            // Some UDPipe models tag real names as a verb/adjective/determiner
            // (for example, Turkish Fatma Demir and Polish Oliwia Szymańska).
            // A curated given name followed by another title-cased name token
            // is enough to type this already grounded full run as PERSON.
            candidate.entity_type = Some("PERSON".to_string());
        }
        out.push(candidate);
    }
    // In a coordinated list of proper-noun subjects, a single-token member
    // is a person mention even though it is too short to establish an
    // identity on its own. Require POS evidence for each member and a verbal
    // predicate after the list; this avoids promoting sentence-initial words,
    // headings, or an unstructured list of place names.
    for person in coordinated_person_list_candidates(tokens, source_text, language) {
        if let Some(existing) = out
            .iter_mut()
            .find(|candidate| candidate.start == person.start && candidate.end == person.end)
        {
            // In a coordinated subject list, the local syntactic evidence
            // resolves a single-token homograph that the gazetteer typed as
            // a place.
            existing.entity_type = Some("PERSON".to_string());
        } else {
            out.push(person);
        }
    }
    // The ORG span must never consume a known place that closes it.
    nested_institution_places(tokens, source_text, language, &mut out);
    // Curated names are entities even when a function word sits inside them
    // ("Will Always Love You" ends in "you"), so they are emitted verbatim
    // before the generic phrase scan can truncate them. The gazetteer decides
    // the type here: a title that opens with a modal ("Will Always Love You")
    // must not be read as a clause by the POS gate.
    for index in 0..tokens.len() {
        if let Some(end) = curated_name_span(token_lower, tokens, bytes, index) {
            let mut candidate = candidate_from_tokens(tokens, source_text, index, end);
            candidate.entity_type = Some(classify_value_type(
                &candidate.text,
                true,
                false,
                true,
                language,
            ));
            out.push(candidate);
        }
    }
    // Preserve exact single-token GeoNames matches even when UDPipe tags a
    // city as a common noun and the surrounding clause would otherwise make
    // the maximal noun phrase fail the POS gate (notably in Turkish).
    for (index, token) in tokens.iter().enumerate() {
        if !starts_uppercase(&token.text) || super::location::is_month_name(&token.text) {
            continue;
        }
        // Do not let a gazetteer homograph steal the head of a common-noun
        // phrase (for example the ordinal in "Seconda guerra mondiale").
        if matches!(token.upos.as_deref(), Some("ADJ" | "NUM"))
            && tokens.get(index + 1).is_some_and(|next| {
                !starts_uppercase(&next.text)
                    && matches!(next.upos.as_deref(), Some("NOUN" | "PROPN"))
            })
            || token
                .dependency
                .as_deref()
                .is_some_and(|relation| relation == "amod")
                && token.head.is_some_and(|head| {
                    tokens.iter().any(|head_token| {
                        head_token.id == Some(head)
                            && head_token.start > token.end
                            && head_token.upos.as_deref() == Some("NOUN")
                    })
                })
        {
            continue;
        }
        let mut candidate = candidate_from_tokens(tokens, source_text, index, index + 1);
        let kind = super::values::classify_value_type(
            &candidate.text,
            true,
            token.upos.as_deref() == Some("PROPN"),
            true,
            language,
        );
        if kind != "VISUAL_CONCEPT" {
            candidate.entity_type = Some(kind);
            out.push(candidate);
        }
    }
    let mut i = 0;
    while i < tokens.len() {
        // A single uppercase initial between periods is a tokenization
        // artifact of dotted initialisms (e.g. U.S.), not an entity span.
        if is_stop_token(&tokens[i], language)
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
                    // A common noun that closes an institution/place phrase
                    // cannot absorb the following proper name in apposition:
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
        } else {
            while j < tokens.len() && !is_stop_token(&tokens[j], language) {
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
                    // A finite verbal predicate continues the clause, not
                    // the noun phrase ("I tifosi festeggiano sugli spalti"):
                    // the verb breaks the phrase here and is skipped below,
                    // so it can never glue to a subject or become an entity.
                    if verb::is_verbal_continuation(tokens, j, language) {
                        break;
                    }
                }
                j += 1;
            }
        }
        let mut phrase_end = j; // exclusive
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
        let start = tokens[phrase_start].start;
        let end = tokens[phrase_end - 1].end;
        // A title-cased ordinal/adjective followed by a lowercase noun is a
        // phrase fragment, not a place mention ("Seconda guerra mondiale").
        if phrase_end == phrase_start + 1
            && starts_uppercase(&tokens[phrase_start].text)
            && matches!(tokens[phrase_start].upos.as_deref(), Some("ADJ" | "NUM"))
            && tokens.get(phrase_end).is_some_and(|next| {
                !starts_uppercase(&next.text)
                    && matches!(next.upos.as_deref(), Some("NOUN" | "PROPN"))
            })
        {
            i = phrase_end;
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
            continue;
        }
        let text = source_text[start..end].to_string();
        let normalized = text.to_lowercase();
        // The run decomposition depends only on the phrase, so it is computed
        // ONCE here instead of twice (the previous form called
        // proper_name_runs twice, each allocating a fresh Vec).
        let phrase_runs = proper_name_runs(&tokens[phrase_start..phrase_end], bytes, language);
        let has_inner_run = phrase_runs.iter().any(|(run_start, _)| *run_start > 0);
        let is_full_run = phrase_runs
            .iter()
            .any(|(run_start, run_end)| *run_start == 0 && *run_end == phrase_end - phrase_start);
        if has_inner_run && !is_full_run {
            i = phrase_end;
            continue;
        }
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

/// Delimited editorial titles are strong WORK evidence. Extract the text
/// inside italics/guillemets before ordinary noun phrases can split title
/// words into people, places, or visual concepts.
pub(crate) fn delimited_work_candidates(source: &str) -> Vec<Candidate> {
    let mut spans = Vec::new();
    for (open, close) in [('*', '*'), ('«', '»'), ('“', '”')] {
        let mut opening = None;
        for (offset, ch) in source.char_indices() {
            if ch == open {
                if opening.is_none() {
                    opening = Some(offset + ch.len_utf8());
                } else if open == close {
                    let start = opening.take().unwrap();
                    spans.push((start, offset));
                }
            } else if ch == close && open != close {
                if let Some(start) = opening.take() {
                    spans.push((start, offset));
                }
            }
        }
    }
    spans
        .into_iter()
        .filter_map(|(start, end)| {
            let delimited = source.get(start..end)?;
            let leading_bytes = delimited.len() - delimited.trim_start().len();
            let raw = delimited.trim();
            let text_start = start + leading_bytes;
            let text_end = text_start + raw.len();
            if raw.is_empty() || !raw.chars().next().is_some_and(char::is_uppercase) {
                return None;
            }
            Some(Candidate {
                text: raw.to_string(),
                start: text_start,
                end: text_end,
                entity_type: Some("WORK".to_string()),
            })
        })
        .collect()
}

fn coordinated_person_list_candidates(
    tokens: &[Token],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for start in 0..tokens.len() {
        let first = &tokens[start];
        if !starts_uppercase(&first.text) || first.upos.as_deref() != Some("PROPN") {
            continue;
        }
        if start > 0 {
            let prior_gap = &source_text[tokens[start - 1].end..first.start];
            if prior_gap.contains(',') || tokens[start - 1].upos.as_deref() == Some("CCONJ") {
                continue;
            }
        }
        let mut members = vec![start];
        let mut commas = 0;
        let mut index = start;
        let mut saw_conjunction = false;
        loop {
            let mut next_index = index + 1;
            while tokens.get(next_index).is_some_and(|token| {
                token.upos.as_deref() == Some("PUNCT")
                    && source_text[tokens[next_index - 1].end..token.start]
                        .chars()
                        .all(char::is_whitespace)
            }) {
                next_index += 1;
            }
            let Some(next) = tokens.get(next_index) else {
                break;
            };
            if next.upos.as_deref() == Some("CCONJ") {
                saw_conjunction = true;
                next_index += 1;
                while tokens
                    .get(next_index)
                    .is_some_and(|token| token.upos.as_deref() == Some("PUNCT"))
                {
                    next_index += 1;
                }
            } else {
                let gap = &source_text[tokens[index].end..next.start];
                if gap.contains(',') {
                    commas += 1;
                } else {
                    break;
                }
            }
            let Some(member) = tokens.get(next_index) else {
                break;
            };
            if starts_uppercase(&member.text) && member.upos.as_deref() == Some("PROPN") {
                members.push(next_index);
                index = next_index;
                continue;
            }
            break;
        }
        // A coordinated subject list needs at least three single-name
        // members, at least two explicit commas, a final conjunction, and a
        // following verb. These structural cues provide evidence without a
        // per-name allowlist.
        if members.len() < 3
            || commas < 2
            || !saw_conjunction
            || !verb::is_verbal_continuation(tokens, index + 1, language)
        {
            continue;
        }
        for member in members {
            let mut candidate = candidate_from_tokens(tokens, source_text, member, member + 1);
            candidate.entity_type = Some("PERSON".to_string());
            out.push(candidate);
        }
    }
    out
}

#[cfg(test)]
mod editorial_title_tests {
    use super::delimited_work_candidates;

    #[test]
    fn italic_and_guillemet_delimiters_protect_work_titles() {
        let source = "*Quel treno da Vienna* e «La torre di Babele».";
        let works = delimited_work_candidates(source);
        assert_eq!(works.len(), 2, "{works:?}");
        for title in ["Quel treno da Vienna", "La torre di Babele"] {
            let start = source.find(title).unwrap();
            assert!(works.iter().any(|work| {
                work.text == title
                    && work.entity_type.as_deref() == Some("WORK")
                    && work.start == start
                    && work.end == start + title.len()
            }));
        }
    }
}
