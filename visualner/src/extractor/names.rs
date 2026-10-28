//! Proper-name runs, curated gazetteer matching and heading detection.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::location::{
    gap_has_period, is_phrase_breaking_byte, is_uppercase_initial, starts_uppercase,
};
use super::pos::is_stop_token;
use super::stopwords::is_stop_word;
use super::tokenizer::Token;
use super::values::{institution_cue_kind, Candidate};

/// First word → whitespace-split curated names, built once per process from
/// the immutable `'static` gazetteer. The matcher then examines only the
/// names whose first word equals the current token (a name can only match at
/// `index` when its first word matches there, so this is the same match set
/// as scanning every name), and every name is split exactly once instead of
/// once per token position.
static CURATED_NAME_INDEX: OnceLock<BTreeMap<String, Vec<Vec<String>>>> = OnceLock::new();

fn curated_name_index() -> &'static BTreeMap<String, Vec<Vec<String>>> {
    CURATED_NAME_INDEX.get_or_init(|| {
        let gazetteer = &semantic_core::lexicon::lexicon().gazetteer;
        let names = gazetteer
            .works
            .iter()
            .chain(gazetteer.persons.iter())
            .chain(gazetteer.brands.iter())
            .chain(gazetteer.product_names.iter())
            .chain(gazetteer.locations.iter());
        let mut index: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
        for name in names {
            let words: Vec<String> = name.split_whitespace().map(str::to_string).collect();
            let Some(first) = words.first().cloned() else {
                continue;
            };
            index.entry(first).or_default().push(words);
        }
        index
    })
}

/// End token index of the longest curated gazetteer name (work, person,
/// brand, product, place) that starts exactly at `tokens[index]`. Such a span
/// is an entity even when one of its words is a function word
/// ("Will Always Love You" ends in the pronoun "you").
///
/// `token_lower` is the per-token case fold computed once per extraction;
/// name words compare against it exactly like the historical
/// `token.text.to_lowercase() == word` probe, without re-folding per name.
pub(crate) fn curated_name_span(
    token_lower: &[String],
    tokens: &[Token],
    bytes: &[u8],
    index: usize,
) -> Option<usize> {
    let name_words = curated_name_index().get(token_lower[index].as_str())?;
    let mut longest: Option<usize> = None;
    for words in name_words {
        if index + words.len() > tokens.len() {
            continue;
        }
        let matches = words.iter().enumerate().all(|(offset, word)| {
            token_lower[index + offset] == *word
                && (offset == 0
                    || !bytes[tokens[index + offset - 1].end..tokens[index + offset].start]
                        .iter()
                        .any(|byte| is_phrase_breaking_byte(*byte)))
        });
        if matches {
            longest = Some(longest.map_or(index + words.len(), |end| end.max(index + words.len())));
        }
    }
    longest
}

pub(crate) fn proper_name_runs(
    tokens: &[Token],
    bytes: &[u8],
    language: &str,
) -> Vec<(usize, usize)> {
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
                && !is_stop_token(&tokens[final_index], language)
            {
                runs.push((i, final_index + 1));
                i = final_index + 1;
                continue;
            }
            i += 1;
            continue;
        }
        // An elided article opens the name it precedes ("dell'Università di
        // Torino"): the capitalized segment inside the token is name evidence
        // even though the token itself starts lowercase.
        let opens_elided_name = elided_name_prefix_len(&tokens[i].text) > 0;
        if !opens_elided_name
            && (is_stop_token(&tokens[i], language) || !starts_uppercase(&tokens[i].text))
        {
            i += 1;
            continue;
        }
        let mut start = i;
        let mut end = i + 1;
        while end < tokens.len() && name_run_continues(tokens, bytes, end, language) {
            end += 1;
        }
        // An all-caps run is a heading ("NOME CHE TORNA"), not a person.
        // Keeping it out of the proper-name set stops the heading from
        // becoming a PERSON and from seeding image queries.
        let heading = tokens[start..end]
            .iter()
            .filter(|token| token.text.chars().filter(|ch| ch.is_alphabetic()).count() >= 2)
            .all(|token| is_all_caps_word(&token.text));
        // "Пермь Мария Соколова" is a city followed by a person, not a
        // three-token person name. A known place stays out of the run so its
        // own gazetteer candidate survives instead of being dropped as a
        // fragment of the person.
        if end - start >= 3
            && is_known_place(&tokens[start].text, language)
            && !is_name_particle(&tokens[start + 1].text, language)
        {
            start += 1;
        }
        // "Visita Pavia" is a clause opener plus a city, not a name: a known
        // place closing a run whose opener carries no name evidence is split
        // off so its own gazetteer candidate survives instead of being
        // swallowed by a fused surface.
        if let Some(split) = place_splits_run(tokens, language, start, end) {
            end = split;
        }
        if end - start >= 2 && !heading {
            runs.push((start, end));
        }
        i = end;
    }
    runs
}

/// Finds capitalized institution heads followed by a proper name, such as
/// "Institut Aurora" or "Comitato Stella". UDPipe often tags the generic
/// head as a common noun, so the ordinary proper-name run starts one token too
/// late. The multilingual organization vocabulary supplies the head; the
/// following capitalized token supplies the identity. This is a general
/// linguistic pattern and does not depend on a specific organization name.
pub(crate) fn organization_name_runs(
    tokens: &[Token],
    bytes: &[u8],
    language: &str,
) -> Vec<(usize, usize)> {
    let key = language.split(['-', '_']).next().unwrap_or("");
    let Some(markers) = semantic_core::lexicon::lexicon()
        .gazetteer
        .organization_markers
        .get(key)
    else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    for (start, token) in tokens.iter().enumerate() {
        if !starts_uppercase(&token.text) {
            continue;
        }
        for marker in markers {
            let marker_words: Vec<String> =
                marker.split_whitespace().map(str::to_lowercase).collect();
            let end = start + marker_words.len();
            if marker_words.is_empty()
                || end >= tokens.len()
                || tokens[start..end]
                    .iter()
                    .zip(&marker_words)
                    .any(|(actual, expected)| !actual.text.eq_ignore_ascii_case(expected))
            {
                continue;
            }
            // A locative particle belongs to the institution's name when a
            // capitalized non-stop identity follows it: "Università di
            // Torino" names the institution, not just its city. The particle
            // inventory is the language's curated one (`name_particles`), so
            // a language without it keeps the tight head+name rule.
            let mut head = end;
            if is_name_particle(&tokens[head].text, language) {
                head += 1;
            }
            if head >= tokens.len()
                || !starts_uppercase(&tokens[head].text)
                || is_stop_token(&tokens[head], language)
            {
                continue;
            }
            // The name continues only across whitespace: a sentence period
            // or a comma starts a new name ("Museo Civico. Aurora Sport" is
            // two identities, "Museo Civico" and "Aurora Sport").
            let mut run_end = head + 1;
            while run_end < tokens.len()
                && starts_uppercase(&tokens[run_end].text)
                && !is_stop_token(&tokens[run_end], language)
                && !bytes[tokens[run_end - 1].end..tokens[run_end].start]
                    .iter()
                    .any(|byte| is_phrase_breaking_byte(*byte))
            {
                run_end += 1;
            }
            runs.push((start, run_end));
        }
    }
    runs
}

/// Whether the token at `index` extends the name run that starts before it.
/// A lowercase surname particle is allowed only between name tokens, so
/// "Emma de Vries" is one name while a trailing "de" stays out.
fn name_run_continues(tokens: &[Token], bytes: &[u8], index: usize, language: &str) -> bool {
    let gap_breaks = |left: usize, right: usize| {
        bytes[tokens[left].end..tokens[right].start]
            .iter()
            .any(|b| is_phrase_breaking_byte(*b))
    };
    let token = &tokens[index];
    if starts_uppercase(&token.text)
        && !is_stop_token(token, language)
        && !(token.text.len() == 1 && token.text.as_bytes()[0].is_ascii_uppercase())
        && !gap_breaks(index - 1, index)
    {
        return true;
    }
    is_name_particle(&token.text, language)
        && index + 1 < tokens.len()
        && starts_uppercase(&tokens[index + 1].text)
        && !is_stop_token(&tokens[index + 1], language)
        && !gap_breaks(index - 1, index)
        && !gap_breaks(index, index + 1)
}

/// True when `text` is a curated organization head for the language
/// ("Università", "Laboratoire", "Istituto"). Such a head names an
/// institution even when its POS tag says common noun.
pub(crate) fn opens_organization_name(text: &str, language: &str) -> bool {
    let key = language.split(['-', '_']).next().unwrap_or("");
    let Some(markers) = semantic_core::lexicon::lexicon()
        .gazetteer
        .organization_markers
        .get(key)
    else {
        return false;
    };
    let head = text.to_lowercase();
    markers.iter().any(|marker| marker == &head)
}

/// True when the gazetteer or place index recognizes `text` as a place.
pub(crate) fn is_known_place(text: &str, language: &str) -> bool {
    semantic_core::lexicon::lexicon().is_place(text, language)
}

/// Name evidence for the token that opens a capitalized run. A common word
/// can be capitalized by sentence position and still not be a name
/// ("Visita Pavia" opens with a NOUN, "Visit London" with a VERB): the POS
/// model knows, the curated word list covers a mistagged common word
/// ("Bezoek Haarlem"), and without a tag capitalization plus a non-stop
/// surface is the only evidence.
pub(crate) fn opens_with_name_evidence(token: &Token, language: &str) -> bool {
    if is_stop_word(&token.text, language) {
        return false;
    }
    // An elided article carries the name segment inside the token
    // ("dell'Università di Torino"): the capitalized segment is the evidence,
    // so a trailing known place stays part of the institution instead of
    // being split off as a clause-opener place.
    if elided_name_prefix_len(&token.text) > 0 {
        return true;
    }
    match token.upos.as_deref() {
        None => starts_uppercase(&token.text),
        Some("PROPN") => true,
        Some(_) => false,
    }
}

/// Token index where a trailing known place begins, when a known place closes
/// a capitalized run whose opener carries no name evidence ("Visita Pavia",
/// "Museo Civico Pavia"). The place keeps its own gazetteer candidate instead
/// of being swallowed by a fused surface, and the fragment before it is a
/// clause opener, not part of the name. A run that is itself a known place
/// ("New York") and a curated institution head ("Procura Pavia") are never
/// split.
pub(crate) fn place_splits_run(
    tokens: &[Token],
    language: &str,
    start: usize,
    end: usize,
) -> Option<usize> {
    if end - start < 2 || opens_with_name_evidence(&tokens[start], language) {
        return None;
    }
    // An institution head keeps its trailing place even when the tagger reads
    // the head as a common noun: "Università di Torino" names the institution,
    // not a clause opener plus a city. The curated per-language organization
    // vocabulary decides, so the rule is data, not a branch per institution.
    if opens_organization_name(&tokens[start].text, language) {
        return None;
    }
    // The cue head is the first substantive word: a leading article the
    // tagger promoted to PROPN ("De Rechtbank") is not the institution head,
    // and the institution keeps its trailing place fused.
    let substantive_surface = tokens[start..end]
        .iter()
        .filter(|token| !is_stop_token(token, language))
        .map(|token| token.text.as_str())
        .collect::<Vec<&str>>()
        .join(" ");
    if institution_cue_kind(&substantive_surface, language).is_some() {
        return None;
    }
    if is_known_place(&run_surface(tokens, start, end), language) {
        return None;
    }
    // Longest known-place suffix first, so "Visita New York" splits at "New
    // York" and not at "York".
    (start + 1..end).find(|&split| is_known_place(&run_surface(tokens, split, end), language))
}

/// Byte length of an elided function prefix inside a token ("dell'Università",
/// "l'Istituto", "d'Italia") when a capitalized name segment follows it. The
/// article is not part of the name, so a run that starts with it starts at the
/// name segment. Zero when there is no such prefix, which keeps "McDonald's"
/// and every other apostrophe word intact.
pub(crate) fn elided_name_prefix_len(text: &str) -> usize {
    let Some(index) = text.find(['\'', '’']) else {
        return 0;
    };
    let prefix = &text[..index];
    if prefix.is_empty()
        || !prefix
            .chars()
            .all(|ch| ch.is_lowercase() && ch.is_alphabetic())
    {
        return 0;
    }
    let apostrophe = text[index..].chars().next().map_or(0, char::len_utf8);
    let after = &text[index + apostrophe..];
    if !after.chars().next().is_some_and(char::is_uppercase) {
        return 0;
    }
    index + apostrophe
}

/// The run's surface joined with single spaces, for gazetteer lookups that
/// match whitespace-normalized aliases.
fn run_surface(tokens: &[Token], start: usize, end: usize) -> String {
    tokens[start..end]
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<&str>>()
        .join(" ")
}

/// A known place that closes a curated institution run ("Pengadilan
/// Surabaya", "Landgericht München") keeps its own LOCATION candidate. The
/// ORG span must never consume the nested place — the nested-place rule of
/// "Procura di Pavia", generalized to a fused surface. A person run is never
/// split here: "Elena Messina" is a person whose surname doubles as a city.
pub(crate) fn nested_institution_places(
    tokens: &[Token],
    source_text: &str,
    language: &str,
    candidates: &mut Vec<super::values::Candidate>,
) {
    let bytes = source_text.as_bytes();
    for (start, end) in proper_name_runs(tokens, bytes, language) {
        let surface = run_surface(tokens, start, end);
        if institution_cue_kind(&surface, language).is_none() {
            continue;
        }
        let Some(split) = (start + 1..end)
            .find(|&split| is_known_place(&run_surface(tokens, split, end), language))
        else {
            continue;
        };
        let place_start = tokens[split].start;
        let place_end = tokens[end - 1].end;
        if candidates
            .iter()
            .any(|candidate| candidate.start == place_start && candidate.end == place_end)
        {
            continue;
        }
        let mut place = candidate_from_tokens(tokens, source_text, split, end);
        place.entity_type = Some("LOCATION".to_string());
        candidates.push(place);
    }
}

/// True when `text` is a surname particle declared for the language.
pub(crate) fn is_name_particle(text: &str, language: &str) -> bool {
    let key = language.split(['-', '_']).next().unwrap_or("");
    semantic_core::lexicon::lexicon()
        .gazetteer
        .name_particles
        .get(key)
        .is_some_and(|particles| {
            particles
                .iter()
                .any(|particle| particle.eq_ignore_ascii_case(text))
        })
}

/// True for a token spelled entirely in capitals, ignoring punctuation and
/// digits. Single letters are excluded so initials stay name fragments.
fn is_all_caps_word(text: &str) -> bool {
    let letters: Vec<char> = text.chars().filter(|ch| ch.is_alphabetic()).collect();
    letters.len() >= 2 && letters.iter().all(|ch| ch.is_uppercase())
}

/// True for editorial headings: a multi-word all-caps run, or a long single
/// all-caps word. Short all-caps tokens (DNA, GUP, ANSA, USA) stay ordinary
/// candidates; headings such as "NOME CHE TORNA" or "RITROVAMENTO" do not.
pub fn is_heading_surface(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    // Single-letter function words inside a heading ("LE INDAGINI E I PRIMI
    // PASSAGGI") do not make it a name, so the all-caps test ignores them.
    let substantial: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| word.chars().filter(|ch| ch.is_alphabetic()).count() >= 2)
        .collect();
    if substantial.is_empty() || !substantial.iter().all(|word| is_all_caps_word(word)) {
        return false;
    }
    words.len() >= 2
        || substantial[0]
            .chars()
            .filter(|ch| ch.is_alphabetic())
            .count()
            >= 6
}

pub(crate) fn candidate_from_tokens(
    tokens: &[Token],
    source_text: &str,
    start: usize,
    end: usize,
) -> Candidate {
    let byte_start = tokens[start].start;
    let byte_end = tokens[end - 1].end;
    let text = source_text[byte_start..byte_end].to_string();
    Candidate {
        text,
        start: byte_start,
        end: byte_end,
        entity_type: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(words: &[(&str, Option<&str>)]) -> Vec<Token> {
        let mut cursor = 0usize;
        words
            .iter()
            .map(|(text, upos)| {
                let start = cursor;
                cursor += text.len() + 1;
                Token {
                    text: text.to_string(),
                    start,
                    end: start + text.len(),
                    upos: upos.map(str::to_string),
                    lemma: None,
                }
            })
            .collect()
    }

    fn source_of(words: &[(&str, Option<&str>)]) -> String {
        words
            .iter()
            .map(|(text, _)| *text)
            .collect::<Vec<&str>>()
            .join(" ")
    }

    #[test]
    fn a_locative_particle_belongs_to_the_institution_name() {
        // "Università di Torino" names the institution, not just its city: the
        // particle keeps the run together instead of ending it before "di".
        let words = tokens(&[
            ("Università", Some("PROPN")),
            ("di", Some("ADP")),
            ("Torino", Some("PROPN")),
        ]);
        let source = source_of(&[
            ("Università", Some("PROPN")),
            ("di", Some("ADP")),
            ("Torino", Some("PROPN")),
        ]);
        assert_eq!(
            organization_name_runs(&words, source.as_bytes(), "it"),
            vec![(0, 3)]
        );
        // A lowercase continuation is not an identity: the particle rule does
        // not fuse "Università di questo tempo".
        let words = tokens(&[
            ("Università", Some("PROPN")),
            ("di", Some("ADP")),
            ("questo", Some("DET")),
        ]);
        assert!(organization_name_runs(&words, source.as_bytes(), "it").is_empty());
        // Without a particle the tight head+name rule is unchanged.
        let words = tokens(&[("Istituto", Some("PROPN")), ("Aurora", Some("PROPN"))]);
        let source = source_of(&[("Istituto", Some("PROPN")), ("Aurora", Some("PROPN"))]);
        assert_eq!(
            organization_name_runs(&words, source.as_bytes(), "it"),
            vec![(0, 2)]
        );
    }

    #[test]
    fn an_organization_name_never_crosses_a_sentence_break() {
        // "Museo Civico. Aurora Sport" is two identities: the run stops at the
        // period instead of merging the next sentence's subject.
        let words = tokens(&[
            ("Museo", Some("PROPN")),
            ("Civico", Some("PROPN")),
            ("Aurora", Some("PROPN")),
            ("Sport", Some("PROPN")),
        ]);
        let source = "Museo Civico. Aurora Sport";
        assert_eq!(
            organization_name_runs(&words, source.as_bytes(), "it"),
            vec![(0, 2)]
        );
    }

    #[test]
    fn an_elided_article_opens_the_name_it_precedes() {
        assert_eq!(elided_name_prefix_len("dell'Università"), 5);
        assert_eq!(elided_name_prefix_len("l'Istituto"), 2);
        // A one-letter lowercase clitic counts too (the name segment is what
        // opens the name); an uppercase initial does not, so the curated
        // "L'Aquila" keeps its own span and typing.
        assert_eq!(elided_name_prefix_len("d'Italia"), 2);
        assert_eq!(elided_name_prefix_len("L'Aquila"), 0);
        // A word whose apostrophe is part of the name keeps its whole surface.
        assert_eq!(elided_name_prefix_len("O'Brien"), 0);
        assert_eq!(elided_name_prefix_len("McDonald's"), 0);
    }
}
