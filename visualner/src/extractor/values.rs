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
    let surface_words: Vec<&str> = head.split_whitespace().collect();
    !words.is_empty()
        && markers.iter().any(|marker| {
            let marker_words: Vec<String> = marker
                .to_lowercase()
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_string)
                .collect();
            if marker_words.is_empty() {
                return false;
            }
            let at_start = words.starts_with(&marker_words);
            let at_end = words.ends_with(&marker_words);
            // A lowercase head ("estudio Bermejo") is name evidence only
            // when a capitalized identity follows it. The common-noun use
            // ("el estudio duró dos años") has no such identity.
            let name_after_marker = at_start
                && surface_words
                    .iter()
                    .skip(marker_words.len())
                    .any(|word| word.chars().next().is_some_and(char::is_uppercase));
            (at_start || at_end) && (proper_noun || capitalized_name || name_after_marker)
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

mod classify;

use classify::classify_type;
pub(super) use classify::curated_typed_entity;

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
        let number_end = scan_number_end(bytes, number_start);
        i = number_end;
        let (candidate, month_first) = quantity_candidate(source_text, number_start, number_end);
        let candidate_end = candidate.end;
        out.push(candidate);
        if month_first {
            i = candidate_end;
        }
    }
    out
}

/// Scans the digits and grouped separators of a number.
fn scan_number_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    while i + 1 < bytes.len() && matches!(bytes[i], b',' | b'.') && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    i
}

/// A currency sign immediately before the number widens the span and type.
fn currency_prefix(source_text: &str, bytes: &[u8], number_start: usize) -> (usize, &'static str) {
    let mut prefix = number_start;
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
        return (prefix - currency.len_utf8(), "MONEY");
    }
    (number_start, "NUMBER")
}

/// Scale words, units and measurements extend the number's span and type.
fn quantity_suffix(
    source_text: &str,
    bytes: &[u8],
    end: usize,
    kind: &'static str,
) -> (usize, &'static str) {
    let mut end = end;
    let mut kind = kind;
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
            .find(|unit| tail.starts_with(unit.as_str()) && has_word_boundary(&tail, unit.len()))
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
    (end, kind)
}

/// Resolves four-digit years and month-first/month-after-day dates.
fn calendar_span(
    source_text: &str,
    number_start: usize,
    number_end: usize,
    start: usize,
    end: usize,
    kind: &'static str,
) -> (usize, usize, &'static str) {
    let mut start = start;
    let mut end = end;
    let mut kind = kind;
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
    (start, end, kind)
}

/// Builds one number candidate and reports whether the caller must advance
/// past it (month-first dates consume the whole date; plain numbers do not).
fn quantity_candidate(
    source_text: &str,
    number_start: usize,
    number_end: usize,
) -> (Candidate, bool) {
    let bytes = source_text.as_bytes();
    let (start, kind) = currency_prefix(source_text, bytes, number_start);
    let (end, kind) = quantity_suffix(source_text, bytes, number_end, kind);
    let (start, end, kind) = calendar_span(source_text, number_start, number_end, start, end, kind);
    let month_first = kind == "DATE" && start < number_start;
    (
        Candidate {
            text: source_text[start..end].to_string(),
            start,
            end,
            entity_type: Some(kind.to_string()),
        },
        month_first,
    )
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
