/// Candidate is a grounded surface with its byte span in the source text.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub(super) text: String,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) entity_type: Option<String>,
}

/// Reject a fused subtitle artifact such as `140ome` before it can be typed as
/// a number or promoted into a visual-name candidate. Short standard forms
/// such as `3D`, `4K`, `5G`, and `F16` remain eligible.
pub(super) fn has_suspicious_asr_digit_fusion(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let prefix_digits = token.chars().take_while(char::is_ascii_digit).count();
        if prefix_digits < 2 {
            return false;
        }
        let suffix = &token[prefix_digits..];
        let letters = suffix.chars().filter(|ch| ch.is_alphabetic()).count();
        letters >= 2
            && !matches!(
                suffix.to_ascii_lowercase().as_str(),
                "st" | "nd" | "rd" | "th"
            )
    })
}

pub(super) fn classify_value_type(
    text: &str,
    gazetteer_eligible: bool,
    proper_noun: bool,
    name_evidence: bool,
    language: &str,
) -> String {
    if text.is_empty() {
        return "VISUAL_CONCEPT".to_string();
    }
    if let Some(kind) = media_outlet_type(text, proper_noun, language) {
        return kind;
    }
    if has_organization_marker(text, proper_noun, language) {
        return "ORG".to_string();
    }
    let currency_words = [
        "dollar", "dollars", "dollaro", "dollari", "dólar", "dólares", "dolares", "euro", "euros",
        "pound", "pounds", "libra", "libras", "yen", "yenes", "iene", "usd", "eur", "gbp", "jpy",
    ];
    let has_currency_word = text
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::to_lowercase)
        .any(|word| currency_words.contains(&word.as_str()));
    if text.chars().any(|c| matches!(c, '$' | '£' | '€' | '¥')) || has_currency_word {
        return "MONEY".to_string();
    }
    if text.contains('%') || text.to_lowercase().contains("percent") {
        return "PERCENT".to_string();
    }
    if text.chars().any(|c| c.is_ascii_digit()) {
        let lower = text.to_lowercase();
        let year = text
            .trim()
            .parse::<u32>()
            .ok()
            .is_some_and(|year| (1000..=2099).contains(&year));
        let has_month = semantic_core::lexicon::lexicon()
            .month_typing_names
            .iter()
            .any(|month| lower.contains(month.as_str()));
        if year || has_month || text.contains('/') {
            return "DATE".to_string();
        }
        return "NUMBER".to_string();
    }
    classify_type(
        text,
        gazetteer_eligible,
        proper_noun,
        name_evidence,
        language,
    )
}

/// True when `text` is a curated facility head ("Stadio", "Stadion",
/// "Estádio"). A facility name keeps a trailing known place as its nested
/// place ("Stadio Aurora di Torino"), exactly like an institution name.
/// The noun list is language-independent data (`facility_facts.json`).
pub(super) fn opens_facility_name(text: &str) -> bool {
    let head = text.to_lowercase();
    semantic_core::lexicon::lexicon()
        .facility
        .facility_nouns
        .iter()
        .any(|noun| noun == &head)
}

/// The kind of the curated institution or role cue that opens `text`, if any
/// ("Corte di Cassazione" -> ORG, "GUP" -> ROLE). The vocabulary is the shared
/// legal pack's, so a new court is a data entry, never a code branch.
pub(super) fn institution_cue_kind(text: &str, language: &str) -> Option<String> {
    let pack = semantic_core::legal_packs::get(language)?;
    // The cue head is the first or last *substantive* word: a leading
    // article or particle ("De Rechtbank") is not the institution's head,
    // and Italian inverts the same office ("Suprema Corte" / "Corte
    // Suprema"). The POS tagger may promote such a word to PROPN inside a
    // name context, so the skip cannot rely on the tag; the curated
    // function inventory decides.
    let words: Vec<String> = text
        .split_whitespace()
        .filter(|word| !super::stopwords::is_stop_word(word, language))
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();
    let first = words.first()?;
    let last = words.last()?;
    pack.entity_cues
        .iter()
        .find(|cue| {
            cue.cue.split_whitespace().next().is_some_and(|word| {
                let head = word.to_lowercase();
                head == *first || head == *last
            })
        })
        .map(|cue| cue.kind.clone())
}

fn contains(list: &[String], value: &str) -> bool {
    list.iter().any(|item| item == value)
}

fn has_organization_marker(text: &str, proper_noun: bool, language: &str) -> bool {
    let key = language.split(['-', '_']).next().unwrap_or("");
    let Some(markers) = semantic_core::lexicon::lexicon()
        .gazetteer
        .organization_markers
        .get(key)
    else {
        return false;
    };
    let head = &text[super::names::elided_name_prefix_len(text).min(text.len())..];
    let words: Vec<String> = head
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    let capitalized_name = head
        .split(|ch: char| ch.is_whitespace() || ch == '-')
        .filter(|word| word.chars().next().is_some_and(char::is_uppercase))
        .count()
        >= 2;
    !words.is_empty()
        && (proper_noun || capitalized_name)
        && markers.iter().any(|marker| {
            let marker_words: Vec<String> = marker
                .to_lowercase()
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_string)
                .collect();
            !marker_words.is_empty()
                && (words.starts_with(&marker_words) || words.ends_with(&marker_words))
        })
}

/// Types a single surface from the curated micro-gazetteer (`entity_gazetteer.json`):
/// brands, cities, works, known people and object words. This table is the
/// seam of a real gazetteer (Wikidata/GeoNames): when that index lands, only
/// this data source changes — never the typing mechanism below.
/// A citable publication is its own entity type, and the decision must run
/// before the institutional marker: "ANSA" and "la Repubblica" would both be
/// claimed by ORG otherwise, and the source-highlight recipe needs to tell a
/// newspaper from any other organization. The name must be a proper noun so a
/// lowercase common noun that happens to match an outlet term never qualifies.
fn media_outlet_type(text: &str, proper_noun: bool, language: &str) -> Option<String> {
    (proper_noun && semantic_core::entity_type_terms::is_media_outlet(text, language))
        .then(|| semantic_core::entity_type_terms::MEDIA_OUTLET.to_string())
}

/// Scans the source for curated outlet domains (`ansa.it`, `notizie.it`).
///
/// Domains are not proper nouns and the tokenizer splits them at the dot, so
/// the noun-phrase pass can never see one whole. The table is curated per
/// language: an arbitrary `example.com` is not a source, and prose such as
/// "ansa" stays an outlet name, never a web citation. Matching is
/// case-insensitive and boundary-checked, and a subdomain (`www.ansa.it`) is
/// accepted because the outlet host is still the match.
pub(super) fn media_outlet_domain_candidates(source_text: &str, language: &str) -> Vec<Candidate> {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let Some(domains) = semantic_core::entity_type_terms::terms()
        .media_outlet_domains
        .get(base)
        .or_else(|| {
            semantic_core::entity_type_terms::terms()
                .media_outlet_domains
                .get("en")
        })
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for domain in domains {
        let needle = domain.as_bytes();
        if needle.is_empty() {
            continue;
        }
        let haystack = source_text.as_bytes();
        if needle.len() > haystack.len() {
            continue;
        }
        for start in 0..=haystack.len() - needle.len() {
            if !source_text.is_char_boundary(start)
                || !source_text.is_char_boundary(start + needle.len())
                || !haystack[start..start + needle.len()].eq_ignore_ascii_case(needle)
            {
                continue;
            }
            let end = start + needle.len();
            let before_ok = source_text[..start]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_' && ch != '@');
            let after_ok = source_text[end..]
                .chars()
                .next()
                .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_' && ch != '-');
            if before_ok && after_ok {
                out.push(Candidate {
                    text: source_text[start..end].to_string(),
                    start,
                    end,
                    entity_type: Some(semantic_core::entity_type_terms::MEDIA_OUTLET.to_string()),
                });
            }
        }
    }
    out.sort_by_key(|candidate| (candidate.start, candidate.end));
    out.dedup_by(|left, right| left.start == right.start && left.end == right.end);
    out
}

fn classify_type(
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
pub(super) fn curated_typed_entity(text: &str) -> Option<String> {
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

// Scale words, units and currencies live in the shared lexicon
// (`scale_words.json`); this module keeps only the numeric mechanism.
/// True when a scale word (`million`, `milioni`, `Millionen`, ...) follows
/// `end` after optional whitespace, so the number before it is a scaled quantity.
pub(super) fn scale_word_follows(source_text: &str, end: usize) -> bool {
    let tail = source_text[end..].trim_start().to_lowercase();
    let scale = &semantic_core::lexicon::lexicon().scale_words;
    scale
        .words
        .iter()
        .chain(scale.english_scale_words.iter())
        .any(|word| tail.starts_with(word.as_str()) && has_word_boundary(&tail, word.len()))
}

/// Find source-grounded numeric spans ignored by the noun-phrase tokenizer.
pub(super) fn value_candidates(source_text: &str) -> Vec<Candidate> {
    let bytes = source_text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        if let Some((start, end)) = numeric_date_span(source_text, i) {
            let text = source_text[start..end].to_string();
            out.push(Candidate {
                text,
                start,
                end,
                entity_type: Some("DATE".to_string()),
            });
            i = end;
            continue;
        }
        let number_start = i;
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        while i + 1 < bytes.len()
            && matches!(bytes[i], b',' | b'.')
            && bytes[i + 1].is_ascii_digit()
        {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        let number_end = i;
        let mut start = number_start;
        let mut end = number_end;
        let mut kind = "NUMBER";
        let mut prefix = start;
        while prefix > 0 && bytes[prefix - 1].is_ascii_whitespace() {
            prefix -= 1;
        }
        // Match the whole preceding character: a single UTF-8 byte is not a
        // currency sign ("ł" ends in 0x82, the euro sign's continuation byte).
        if let Some(currency) = source_text[..prefix]
            .chars()
            .next_back()
            .filter(|ch| matches!(ch, '$' | '£' | '€'))
        {
            start = prefix - currency.len_utf8();
            kind = "MONEY";
        }
        let mut suffix = end;
        while suffix < bytes.len() && bytes[suffix].is_ascii_whitespace() {
            suffix += 1;
        }
        let tail = source_text[suffix..].to_lowercase();
        let local_scale = semantic_core::lexicon::lexicon()
            .scale_words
            .words
            .iter()
            .find(|word| tail.starts_with(word.as_str()) && has_word_boundary(&tail, word.len()));
        let unit = semantic_core::lexicon::lexicon()
            .scale_words
            .unit_words
            .iter()
            .find(|unit| tail.starts_with(unit.as_str()) && has_word_boundary(&tail, unit.len()));
        let unit = unit.filter(|_| local_scale.is_none());
        // A physical measurement ("12 km", "20 °C") is a quantity, not a
        // count: it gets its own type so charts and bullets can tell it apart.
        let measurement = if unit.is_none() && local_scale.is_none() {
            semantic_core::lexicon::lexicon()
                .scale_words
                .measurement_units
                .iter()
                .find(|unit| {
                    tail.starts_with(unit.as_str()) && has_word_boundary(&tail, unit.len())
                })
        } else {
            None
        };
        // The unit is part of the quantity's type, not its span: the number
        // keeps the same byte range it has as a NUMBER, so numeric facts and
        // charts that read the number are unaffected.
        if measurement.is_some() {
            kind = "MEASUREMENT";
        }
        if let Some(scale) = local_scale {
            end = suffix + scale.len();
        }
        if let Some(unit) = unit.filter(|unit| has_word_boundary(&tail, unit.len())) {
            end = suffix + unit.len();
            if unit.contains("dollar")
                || unit.contains("euro")
                || unit.contains("pound")
                || *unit == "yen"
            {
                kind = "MONEY";
            } else if semantic_core::lexicon::lexicon()
                .scale_words
                .percentage_words
                .iter()
                .any(|percentage| percentage == unit)
                || *unit == "%"
            {
                kind = "PERCENT";
            } else if matches!(unit.as_str(), "million" | "billion" | "trillion") {
                let remainder = source_text[end..].trim_start();
                let lower = remainder.to_lowercase();
                if let Some(currency) = semantic_core::lexicon::lexicon()
                    .scale_words
                    .currency_words
                    .iter()
                    .find(|currency| {
                        lower.starts_with(currency.as_str())
                            && has_word_boundary(&lower, currency.len())
                    })
                {
                    let whitespace = source_text[end..].len() - remainder.len();
                    end += whitespace + currency.len();
                    kind = "MONEY";
                }
            }
        }
        let number = &source_text[number_start..number_end];
        if kind == "NUMBER" && number.len() == 4 {
            if let Ok(year) = number.parse::<u32>() {
                if (1000..=2099).contains(&year) {
                    kind = "DATE";
                }
            }
        }
        let is_year_date = kind == "DATE" && number.len() == 4;
        if kind == "NUMBER" || is_year_date {
            if let Some(month_start) = month_before_day(source_text, number_start) {
                start = month_start;
                kind = "DATE";
                let suffix_text = source_text[number_end..].trim_start();
                if let Some(after_comma) = suffix_text.strip_prefix(',') {
                    let leading_spaces = after_comma.len() - after_comma.trim_start().len();
                    let year_text = after_comma.trim_start();
                    let year_len = year_text.bytes().take_while(u8::is_ascii_digit).count();
                    if year_len == 4 {
                        end = number_end
                            + (source_text[number_end..].len() - suffix_text.len())
                            + 1
                            + leading_spaces
                            + year_len;
                    }
                }
            } else if let Some(month_end) = month_after_day(source_text, number_end) {
                end = month_end;
                kind = "DATE";
            }
        }
        let text = source_text[start..end].to_string();
        out.push(Candidate {
            text,
            start,
            end,
            entity_type: Some(kind.to_string()),
        });
        if kind == "DATE" && start < number_start {
            i = end;
        }
    }
    out
}

fn has_word_boundary(text: &str, end: usize) -> bool {
    text.get(end..)
        .and_then(|tail| tail.chars().next())
        .map(|next| !next.is_alphanumeric())
        .unwrap_or(true)
}

fn numeric_date_span(source: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = start;
    let mut parts = Vec::new();
    loop {
        let part_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if cursor == part_start {
            return None;
        }
        parts.push(&source[part_start..cursor]);
        if cursor >= bytes.len() || !matches!(bytes[cursor], b'/' | b'-') {
            break;
        }
        cursor += 1;
        if parts.len() >= 3 || cursor >= bytes.len() || !bytes[cursor].is_ascii_digit() {
            return None;
        }
    }
    if parts.len() < 2 || parts.iter().any(|part| part.len() > 4) {
        return None;
    }
    let has_year = parts.iter().any(|part| part.len() == 4);
    if !has_year {
        let first = parts[0].parse::<u32>().ok()?;
        let second = parts[1].parse::<u32>().ok()?;
        if first > 12 || second > 31 {
            return None;
        }
    }
    Some((start, cursor))
}

fn month_before_day(source: &str, number_start: usize) -> Option<usize> {
    let prefix = source.get(..number_start)?;
    let trimmed = prefix.trim_end();
    let start = trimmed
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_alphabetic())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    let month = trimmed.get(start..)?.to_lowercase();
    semantic_core::lexicon::lexicon()
        .month_typing_names
        .iter()
        .any(|name| name == &month)
        .then_some(start)
}

fn month_after_day(source: &str, number_end: usize) -> Option<usize> {
    let tail = source[number_end..].trim_start();
    let lower = tail.to_lowercase();
    let month = semantic_core::lexicon::lexicon()
        .month_typing_names
        .iter()
        .find(|month| lower.starts_with(month.as_str()))?;
    if !has_word_boundary(&lower, month.len()) {
        return None;
    }
    Some(number_end + source[number_end..].len() - tail.len() + month.len())
}

#[cfg(test)]
#[path = "values_tests.rs"]
mod tests;
