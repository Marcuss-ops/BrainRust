use super::*;

/// Mark a source-grounded proper-name candidate as a place when its local
/// syntax supplies geographic context. The extractor has no gazetteer, so it
/// does not guess from capitalization alone; cues such as "em Fortaleza" and
/// "no bairro Aldeota" provide the evidence for this classification.
pub(super) fn mark_contextual_locations(
    candidates: &mut Vec<Candidate>,
    tokens: &[Token],
    source_text: &str,
) {
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
            // Italian articulated prepositions carry the same geographic cue as
            // the Portuguese/English entries above ("nel Lazio", "della
            // Lombardia", "del Veneto", "sull'Adriatico"). The elided forms
            // keep their apostrophe, because the tokenizer splits
            // "dell'Abruzzo" into the clitic "dell'" and the place "Abruzzo".
            | "a"
            | "nel"
            | "nello"
            | "nella"
            | "nei"
            | "negli"
            | "nelle"
            | "nell"
            | "nell'"
            | "del"
            | "dello"
            | "della"
            | "dei"
            | "degli"
            | "delle"
            | "dell"
            | "dell'"
            | "al"
            | "allo"
            | "alla"
            | "ai"
            | "agli"
            | "alle"
            | "all"
            | "all'"
            | "dal"
            | "dallo"
            | "dalla"
            | "dai"
            | "dagli"
            | "dalle"
            | "dall"
            | "dall'"
            | "sul"
            | "sullo"
            | "sulla"
            | "sui"
            | "sugli"
            | "sulle"
            | "sull"
            | "sull'"
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

pub(super) fn is_month_name(text: &str) -> bool {
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

pub(super) fn is_generic_location_noun(text: &str) -> bool {
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
            // Italian generic place words: after a geographic cue they name a
            // kind of place, not a place ("nella città di Milano").
            | "città"
            | "paese"
            | "comune"
            | "regione"
            | "provincia"
            | "nazione"
            | "stato"
            | "località"
            | "frazione"
    )
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
