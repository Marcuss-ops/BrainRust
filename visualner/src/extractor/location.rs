use super::names::candidate_from_tokens;
use super::tokenizer::Token;
use super::values::Candidate;

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
            !super::stopwords::is_stop_word(text, language) && !is_month_name(text)
        }
    };
    for index in 0..tokens.len() {
        if !name_evidence(&tokens[index].text)
            || is_month_name(&tokens[index].text)
            || is_acronym(&tokens[index].text)
            || !has_location_context(tokens, bytes, index)
        {
            continue;
        }
        let mut end = index + 1;
        while end < tokens.len() && name_evidence(&tokens[end].text) {
            if bytes[tokens[end - 1].end..tokens[end].start]
                .iter()
                .any(|byte| is_phrase_breaking_byte(*byte))
            {
                break;
            }
            end += 1;
        }
        let mut place = candidate_from_tokens(tokens, source_text, index, end);
        if is_generic_location_noun(&place.text)
            || !is_place_name(&tokens[index], &place.text, language, cased)
        {
            continue;
        }
        place.entity_type = Some("LOCATION".to_string());
        contextual.push(place);
    }

    for candidate in candidates.iter_mut() {
        if candidate.entity_type.is_some()
            || !name_evidence(&candidate.text)
            || is_acronym(&candidate.text)
        {
            continue;
        }
        let Some(token_index) = tokens
            .iter()
            .position(|token| token.start == candidate.start)
        else {
            continue;
        };
        if !is_place_name(&tokens[token_index], &candidate.text, language, cased) {
            continue;
        }
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

/// A geographic cue takes a proper name ("presso la Procura di Pavia" names
/// Pavia, not the Procura). Three kinds of evidence decide:
///
/// * the POS tag — a NOUN is never a place;
/// * the lemma — in a multi-word surface a common noun promoted to PROPN by
///   its position keeps a lowercase lemma ("Udienza" -> "udienza"), while a
///   toponym keeps its capital ("Great Smoky" -> "Great Smoky"). A
///   single-token surface is not judged by its lemma: UDPipe lowercases the
///   lemma of many real place names ("Lazio"), so the tag and the
///   institutional vocabulary are the only evidence there;
/// * the shared legal pack — an institution or role cue head ("Corte",
///   "Procura", "Cassazione") names an office, not a point on a map.
///
/// Without a model every tag and lemma is `None` and only the institutional
/// vocabulary and the capitalization guard apply, so the model-free path keeps
/// its behavior.
pub(super) fn is_place_name(token: &Token, surface: &str, language: &str, cased: bool) -> bool {
    // UDPipe leaves some names untagged (X) or bare; the cue still applies.
    if let Some("NOUN") = token.upos.as_deref() {
        return false;
    }
    if cased && surface.contains(char::is_whitespace) {
        if let Some(lemma) = token.lemma.as_deref() {
            if lemma.chars().next().is_some_and(char::is_lowercase) {
                return false;
            }
        }
    }
    !is_institution_name(surface, language)
}

/// True when the surface opens with the head word of a curated institution or
/// role cue ("Corte di Cassazione", "Procura di ", "Cassazione", "GUP"). The
/// vocabulary is the legal pack's, so a new court is a data entry.
fn is_institution_name(text: &str, language: &str) -> bool {
    let Some(pack) = semantic_core::legal_packs::get(language) else {
        return false;
    };
    let head = text
        .split_whitespace()
        .next()
        .unwrap_or(text)
        .to_lowercase();
    if head.is_empty() {
        return false;
    }
    pack.entity_cues.iter().any(|cue| {
        cue.cue
            .split_whitespace()
            .next()
            .is_some_and(|word| word.to_lowercase() == head)
    })
}

/// Short all-caps surfaces are acronyms (DNA, GUP, ANSA), never place names
/// reached through a geographic cue such as "del" or "in".
fn is_acronym(text: &str) -> bool {
    let letters: Vec<char> = text.chars().filter(|ch| ch.is_alphabetic()).collect();
    (2..=5).contains(&letters.len()) && letters.iter().all(|ch| ch.is_uppercase())
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
        .any(|byte| is_phrase_breaking_byte(*byte))
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
/// tokens should break a noun phrase. Horizontal whitespace (space, tab)
/// does NOT break; a newline DOES, because an all-caps heading followed by a
/// blank line and a capitalized sentence ("IL NOME CHE TORNA\n\nIn una
/// storia") is not one name. Any other non-word byte (comma, period,
/// semicolon, colon, dash outside a word, parens) also breaks. This keeps
/// "feta cheese" together while splitting "tomatoes, feta".
pub(super) fn is_phrase_breaking_byte(b: u8) -> bool {
    b == b'\n' || b == b'\r' || !b.is_ascii_whitespace()
}
