use super::*;

use semantic_core::lexicon::lexicon;

/// Mark a source-grounded proper-name candidate as a place when its local
/// syntax supplies geographic context. The extractor has no gazetteer, so it
/// does not guess from capitalization alone; cues such as "em Fortaleza" and
/// "no bairro Aldeota" provide the evidence for this classification. The cue
/// vocabularies live in the shared lexicon (`location_cues.json`).
pub(super) fn mark_contextual_locations(
    candidates: &mut Vec<Candidate>,
    tokens: &[Token],
    source_text: &str,
    language: &str,
) {
    let bytes = source_text.as_bytes();
    let mut contextual = Vec::new();
    // A capitalized initial is name evidence only in a cased script. In Han,
    // Japanese, Arabic or Thai there is no uppercase, so the cue alone must
    // be able to ground the following word ("在 北京"); stopwords and month
    // names still keep common words out.
    let cased = lexicon().language_profiles.is_cased(language);
    let name_evidence = |text: &str| {
        if cased {
            starts_uppercase(text)
        } else {
            !super::stopwords::is_stop_word(text) && !is_month_name(text)
        }
    };
    for index in 0..tokens.len() {
        if !name_evidence(&tokens[index].text)
            || is_month_name(&tokens[index].text)
            || !has_location_context(tokens, bytes, index)
        {
            continue;
        }
        let mut end = index + 1;
        while end < tokens.len() && name_evidence(&tokens[end].text) {
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
        if candidate.entity_type.is_some() || !name_evidence(&candidate.text) {
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

/// True when the token before `tokens[token_index]` is a geographic cue and
/// the two tokens are adjacent ("em Fortaleza"), or the cue chain is
/// genitive + container noun ("cidade de", "city of").
pub(super) fn has_location_context(tokens: &[Token], bytes: &[u8], token_index: usize) -> bool {
    let Some(previous) = token_index.checked_sub(1).map(|index| &tokens[index]) else {
        return false;
    };
    if bytes[previous.end..tokens[token_index].start]
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
    {
        return false;
    }
    let cues = &lexicon().location_cues;
    let previous = previous.text.to_lowercase();
    cues.context_prepositions.iter().any(|cue| cue == &previous)
        || (cues
            .genitive_prepositions
            .iter()
            .any(|cue| cue == &previous)
            && token_index >= 2
            && cues
                .container_nouns
                .iter()
                .any(|cue| cue == &tokens[token_index - 2].text.to_lowercase()))
}

pub(super) fn is_month_name(text: &str) -> bool {
    let lower = text.to_lowercase();
    lexicon()
        .month_typing_names
        .iter()
        .any(|month| month == &lower)
}

pub(super) fn is_generic_location_noun(text: &str) -> bool {
    let lower = text.to_lowercase();
    lexicon()
        .location_cues
        .generic_location_nouns
        .iter()
        .any(|noun| noun == &lower)
}

pub(super) fn starts_uppercase(text: &str) -> bool {
    text.chars().next().map(char::is_uppercase).unwrap_or(false)
}

pub(super) fn is_uppercase_initial(text: &str) -> bool {
    text.len() == 1 && text.as_bytes()[0].is_ascii_uppercase()
}

pub(super) fn has_dotted_initial_prefix(tokens: &[Token], bytes: &[u8], name_index: usize) -> bool {
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

pub(super) fn gap_has_period(gap: &[u8]) -> bool {
    gap.first() == Some(&b'.') && gap[1..].iter().all(u8::is_ascii_whitespace)
}

/// is_phrase_breaking_byte reports whether a byte in the gap between two
/// tokens should break a noun phrase. Whitespace (space, tab, newline)
/// does NOT break; any other non-word byte (comma, period, semicolon,
/// colon, dash outside a word, parens) DOES break. This is what keeps
/// "feta cheese" together while splitting "tomatoes, feta".
pub(super) fn is_phrase_breaking_byte(b: u8) -> bool {
    !b.is_ascii_whitespace()
}
