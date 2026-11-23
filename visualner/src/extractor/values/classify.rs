//! Typing of grounded surfaces: entity gazetteer and institutional cues.

use super::{contains, has_organization_marker, institution_cue_kind, media_outlet_type};

pub(super) fn classify_type(
    text: &str,
    gazetteer_eligible: bool,
    proper_noun: bool,
    name_evidence: bool,
    language: &str,
) -> String {
    if let Some(kind) = media_outlet_type(text, proper_noun, language) {
        return kind;
    }
    let lower = text.to_lowercase();
    let lexicon = semantic_core::lexicon::lexicon();
    let gazetteer = &lexicon.gazetteer;
    if let Some(kind) = curated_typed_entity(text) {
        // GeoNames contains homographic multiword strings. An explicit
        // institutional head in the same span is stronger evidence than a
        // place alias (for example an orchestra name ending in a Rai acronym).
        if kind != "LOCATION" || !has_organization_marker(text, proper_noun, language) {
            return kind;
        }
    }
    if has_organization_marker(text, proper_noun, language) {
        return "ORG".to_string();
    }
    // A concrete noun followed by a known brand is one visual product
    // surface, not a two-token person name. Preserve the exact source span
    // while typing the full compound before the generic proper-name fallback.
    if let Some((object_phrase, brand)) = lower.rsplit_once(' ') {
        let brand = brand.trim_matches(|ch: char| !ch.is_alphanumeric());
        let object_phrase = object_phrase.trim();
        let has_brand =
            contains(&gazetteer.brands, brand) || contains(&gazetteer.product_names, brand);
        let has_object = semantic_core::lexicon::lexicon()
            .object_terms(language)
            .iter()
            .any(|term| {
                object_phrase == term
                    || object_phrase
                        .strip_suffix(term)
                        .is_some_and(|prefix| prefix.ends_with(' '))
            });
        if has_brand && has_object {
            return "PRODUCT".to_string();
        }
    }
    if contains(&gazetteer.object_words, &lower) {
        return "OBJECT".to_string();
    }
    // Additive visual/documentary types are lexical data. They run after the
    // existing OBJECT vocabulary so an older consumer keeps its original type
    // for terms it already knew.
    if let Some(kind) = semantic_core::entity_type_terms::classify(text, language) {
        return kind.to_string();
    }
    let last_token = lower
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if contains(&gazetteer.company_suffixes, last_token) {
        return "ORGANIZATION".to_string();
    }
    if has_organization_marker(text, proper_noun, language) {
        return "ORG".to_string();
    }
    if contains(&gazetteer.generic_visual_nouns, &lower) {
        return "VISUAL_CONCEPT".to_string();
    }
    if contains(&gazetteer.brands, &lower) {
        return "BRAND".to_string();
    }
    if contains(&gazetteer.works, &lower) {
        return "WORK".to_string();
    }
    if contains(&gazetteer.persons, &lower)
        || ((proper_noun || name_evidence)
            && text.split_whitespace().count() == 1
            && semantic_core::lexicon::is_curated_given_name(language, text))
    {
        return "PERSON".to_string();
    }
    // The hand-curated seed is explicit and always wins, even when a tagger
    // mislabels the city ("Cagliari" as NOUN): it is high-precision data.
    // Keep lowercase common nouns ("potenza") out of LOCATION in cased scripts.
    if (!lexicon.language_profiles.is_cased(language)
        || text.chars().next().is_some_and(char::is_uppercase))
        && contains(&gazetteer.locations, &lower)
    {
        return "LOCATION".to_string();
    }
    let facility_head = lexicon
        .facility
        .facility_nouns
        .iter()
        .any(|noun| lower.split_whitespace().next() == Some(noun.as_str()));
    if facility_head && name_evidence && text.split_whitespace().count() >= 2 {
        return "FACILITY".to_string();
    }
    if contains(&gazetteer.product_names, &lower)
        || gazetteer
            .product_substrings
            .iter()
            .any(|product| lower.contains(product.as_str()))
    {
        return "PRODUCT".to_string();
    }
    // An institution is never a person: "Corte Suprema" and "Cassazione" are
    // offices, and the curated legal cue decides their kind. The head keeps
    // its capital when it names the office ("Il Tribunale", "Procura di
    // Pavia"); a bare lowercase head ("tribunale") is a common noun and stays
    // a visual concept instead of becoming a fake organization.
    if let Some(kind) = institution_cue_kind(text, language) {
        let head_capitalized = text.chars().next().is_some_and(char::is_uppercase);
        if kind == "ORG"
            && (head_capitalized
                || !lexicon
                    .language_profiles
                    .uppercase_is_name_evidence(language))
        {
            return "ORG".to_string();
        }
        return "VISUAL_CONCEPT".to_string();
    }
    // Consult the broad GeoNames index only after exact entity and institutional
    // cues. Single-token aliases such as "Sinistra" can otherwise steal a
    // token from a known organization name.
    let acronym_letters: Vec<char> = text.chars().filter(|ch| ch.is_alphabetic()).collect();
    let acronym = (2..=5).contains(&acronym_letters.len())
        && acronym_letters.iter().all(|ch| ch.is_uppercase());
    if gazetteer_eligible && !acronym && lexicon.is_place(text, language) {
        return "LOCATION".to_string();
    }
    // An acronym-led compound is evidence or an office, not a person: "DNA
    // di Chiara" is a biological trace and "GUP di Pavia" is a court
    // office. The acronym carries no name evidence, so the generic
    // title-case fallback below must not promote the phrase.
    let opens_with_acronym = text
        .split_whitespace()
        .next()
        .map(|word| {
            let letters: Vec<char> = word.chars().filter(|ch| ch.is_alphabetic()).collect();
            (2..=5).contains(&letters.len()) && letters.iter().all(|ch| ch.is_uppercase())
        })
        .unwrap_or(false);
    // Capitalization is name evidence only where the orthography makes it
    // meaningful: not in Han/Arabic/Thai (no case), and not in German (every
    // noun is capitalized, so "Die Neue Halle" is not a person).
    let title_tokens = text
        .split_whitespace()
        .filter(|word| word.chars().next().map(char::is_uppercase).unwrap_or(false))
        .count();
    let case_is_evidence = lexicon
        .language_profiles
        .uppercase_is_name_evidence(language);
    // A curated commercial head word closes an organization name even when no
    // legal suffix does: "Aurora Sport" and "Lumen Energia" are companies in
    // the same way "Nordwasser GmbH" is, and a two-token title case run would
    // otherwise satisfy the person rule. The head list is data
    // (`entity_gazetteer.json.company_head_words`), so a new head is an entry,
    // never a code branch, and a surname never appears in it.
    let head_word = lower
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_matches(|ch: char| !ch.is_alphanumeric());
    if text.split_whitespace().count() >= 2
        && title_tokens >= 2
        && contains(&gazetteer.company_head_words, head_word)
    {
        return "ORGANIZATION".to_string();
    }
    if text.split_whitespace().count() >= 2
        && title_tokens >= 2
        && contains(&gazetteer.event_head_words, head_word)
    {
        return "EVENT".to_string();
    }
    // A compound that links a head noun to a known place with a relational
    // preposition names a place, not a person: "Città Vecchia di Gerusalemme"
    // and "Piazza del Campo di Siena" are locations. The link is what keeps
    // this rule away from people whose surname is also a place ("Michael
    // Jordan" carries no `di`), so the generic person fallback can stay broad.
    if case_is_evidence {
        const RELATIONAL_HEAD_LINKS: [&str; 7] =
            ["di", "del", "dello", "della", "dei", "degli", "delle"];
        let words: Vec<&str> = text.split_whitespace().collect();
        let place_link = words.windows(2).any(|pair| {
            let link = pair[0]
                .trim_matches(|ch: char| !ch.is_alphabetic())
                .to_lowercase();
            let target = pair[1].trim_matches(|ch: char| !ch.is_alphabetic());
            RELATIONAL_HEAD_LINKS.contains(&link.as_str())
                && target.chars().next().is_some_and(char::is_uppercase)
                && lexicon.is_place(target, language)
        });
        if place_link {
            return "LOCATION".to_string();
        }
    }
    if case_is_evidence
        && name_evidence
        && !opens_with_acronym
        && title_tokens >= 2
        && text.split_whitespace().count() >= 2
    {
        return "PERSON".to_string();
    }
    // In German the tagger's PROPN substitutes for the case signal that the
    // orthography destroys: it restores person recall without letting every
    // capitalized noun phrase become a PERSON. In a caseless script PROPN only
    // says "proper noun" (a country, an organization, a person), so it is not
    // evidence of PERSON and the surface stays a visual concept.
    if lexicon.language_profiles.capitalized_nouns(language)
        && proper_noun
        && !opens_with_acronym
        && text.split_whitespace().count() >= 2
    {
        return "PERSON".to_string();
    }
    "VISUAL_CONCEPT".to_string()
}

/// Exact type for a curated multiword entity. The same lookup is applied after
/// contextual cues so those cues cannot split one known name into conflicting
/// identities (for example, one FACILITY and one LOCATION for Trump Tower).
pub(crate) fn curated_typed_entity(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    semantic_core::lexicon::lexicon()
        .gazetteer
        .typed_entities
        .iter()
        .find(|(kind, names)| {
            matches!(
                kind.as_str(),
                "FACILITY"
                    | "STRUCTURE"
                    | "LOCATION"
                    | "ORGANIZATION"
                    | "ORG"
                    | "BRAND"
                    | "PRODUCT"
                    | "WORK"
                    | "EVENT"
            ) && names.iter().any(|name| {
                // All-capital lexicon entries represent acronyms (NATO, WHO).
                // Matching those case-insensitively turns sentence-initial
                // participles such as “Nato a Roma” into organizations.
                let acronym = name.chars().any(char::is_alphabetic)
                    && name
                        .chars()
                        .filter(|ch| ch.is_alphabetic())
                        .all(char::is_uppercase);
                if acronym {
                    text == name
                } else {
                    name.eq_ignore_ascii_case(&lower)
                }
            })
        })
        .map(|(kind, _)| kind.clone())
}
