//! Named-entity candidate construction.

use super::super::location::starts_uppercase;
use super::super::names::{
    candidate_from_tokens, curated_name_span, nested_institution_places, organization_name_runs,
    proper_name_runs,
};
use super::super::tokenizer::Token;
use super::super::values::{classify_value_type, Candidate};
use super::super::verb;

/// Named-entity candidates: proper/organization runs, coordinated person
/// lists, institutions nested with places, curated titles and single-token
/// gazetteer matches.
pub(super) fn named_candidates(
    tokens: &[Token],
    token_lower: &[String],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let bytes = source_text.as_bytes();
    let mut out = proper_name_candidates(tokens, bytes, source_text, language);
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
    out.extend(curated_name_candidates(
        tokens,
        token_lower,
        bytes,
        source_text,
        language,
    ));
    out.extend(gazetteer_single_candidates(tokens, source_text, language));
    out
}

/// Named people/organizations bounded to the contiguous title-case run. This
/// prevents lower-case lead-ins such as "figures like Phil Jackson" from
/// becoming a PERSON entity; the grounded candidate is "Phil Jackson".
fn proper_name_candidates(
    tokens: &[Token],
    bytes: &[u8],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
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
    let mut out = Vec::with_capacity(name_runs.len());
    for (start, end) in name_runs {
        let mut candidate = candidate_from_tokens(tokens, source_text, start, end);
        let capitalized_words = tokens[start..end]
            .iter()
            .filter(|token| starts_uppercase(&token.text))
            .count();
        if capitalized_words >= 2
            && semantic_core::lexicon::is_curated_given_name(language, &tokens[start].text)
            && !semantic_core::lexicon::lexicon().is_place(&candidate.text, language)
            && super::super::values::institution_cue_kind(&candidate.text, language).is_none()
            && !super::super::names::opens_organization_name(&tokens[start].text, language)
        {
            // Some UDPipe models tag real names as a verb/adjective/determiner
            // (for example, Turkish Fatma Demir and Polish Oliwia Szymańska).
            // A curated given name followed by another title-cased name token
            // is enough to type this already grounded full run as PERSON.
            candidate.entity_type = Some("PERSON".to_string());
        }
        out.push(candidate);
    }
    out
}

/// Curated names are entities even when a function word sits inside them
/// ("Will Always Love You" ends in "you"), so they are emitted verbatim
/// before the generic phrase scan can truncate them. The gazetteer decides
/// the type here: a title that opens with a modal ("Will Always Love You")
/// must not be read as a clause by the POS gate.
fn curated_name_candidates(
    tokens: &[Token],
    token_lower: &[String],
    bytes: &[u8],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let mut out = Vec::new();
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
    out
}

/// Preserve exact single-token GeoNames matches even when UDPipe tags a city
/// as a common noun and the surrounding clause would otherwise make the
/// maximal noun phrase fail the POS gate (notably in Turkish).
fn gazetteer_single_candidates(
    tokens: &[Token],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !starts_uppercase(&token.text) || super::super::location::is_month_name(&token.text) {
            continue;
        }
        // Do not let a gazetteer homograph steal the head of a common-noun
        // phrase (for example the ordinal in "Seconda guerra mondiale").
        if is_gazetteer_homograph_head(tokens, index) {
            continue;
        }
        let mut candidate = candidate_from_tokens(tokens, source_text, index, index + 1);
        let kind = super::super::values::classify_value_type(
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
    out
}

/// A title-cased ordinal/adjective that only heads a lowercase common noun is
/// not a place mention ("Seconda guerra mondiale").
fn is_gazetteer_homograph_head(tokens: &[Token], index: usize) -> bool {
    let token = &tokens[index];
    (matches!(token.upos.as_deref(), Some("ADJ" | "NUM"))
        && tokens.get(index + 1).is_some_and(|next| {
            !starts_uppercase(&next.text) && matches!(next.upos.as_deref(), Some("NOUN" | "PROPN"))
        }))
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
