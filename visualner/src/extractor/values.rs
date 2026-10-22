/// Candidate is a grounded surface with its byte span in the source text.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub(super) text: String,
    #[allow(dead_code)]
    pub(super) normalized: String,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) entity_type: Option<String>,
}

pub(super) fn classify_value_type(
    text: &str,
    gazetteer_eligible: bool,
    proper_noun: bool,
    language: &str,
) -> String {
    if text.is_empty() {
        return "VISUAL_CONCEPT".to_string();
    }
    if text.chars().any(|c| matches!(c, '$' | '£' | '€' | '¥')) || {
        let lower = text.to_lowercase();
        ["dollar", "euro", "pound", "yen"]
            .iter()
            .any(|unit| lower.contains(unit))
    } {
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
    classify_type(text, gazetteer_eligible, proper_noun, language)
}

fn contains(list: &[String], value: &str) -> bool {
    list.iter().any(|item| item == value)
}

/// Types a single surface from the curated micro-gazetteer (`entity_gazetteer.json`):
/// brands, cities, works, known people and object words. This table is the
/// seam of a real gazetteer (Wikidata/GeoNames): when that index lands, only
/// this data source changes — never the typing mechanism below.
fn classify_type(
    text: &str,
    gazetteer_eligible: bool,
    proper_noun: bool,
    language: &str,
) -> String {
    let lower = text.to_lowercase();
    let lexicon = semantic_core::lexicon::lexicon();
    let gazetteer = &lexicon.gazetteer;
    if contains(&gazetteer.object_words, &lower) {
        return "OBJECT".to_string();
    }
    if contains(&gazetteer.generic_visual_nouns, &lower) {
        return "VISUAL_CONCEPT".to_string();
    }
    let last_token = lower
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if contains(&gazetteer.company_suffixes, last_token) {
        return "ORGANIZATION".to_string();
    }
    if contains(&gazetteer.brands, &lower) {
        return "BRAND".to_string();
    }
    // The hand-curated seed is explicit and always wins, even when a tagger
    // mislabels the city ("Cagliari" as NOUN): it is high-precision data.
    if contains(&gazetteer.locations, &lower) {
        return "LOCATION".to_string();
    }
    // The real GeoNames index is consulted only for place names: "Come"
    // (ADV) and "Corte" (PROPN with a lowercase lemma) also exist as place
    // aliases, but they are not places in this text. Without a POS model (no
    // tag) the capitalization guard inside `is_place` still applies.
    if gazetteer_eligible && lexicon.is_place(text, language) {
        return "LOCATION".to_string();
    }
    // Deterministic work/title exceptions prevent song and programme names
    // from satisfying the generic multi-token person rule.
    if contains(&gazetteer.works, &lower) {
        return "WORK".to_string();
    }
    // Recognize single-token people explicitly. Without contextual NER, a
    // generic capitalization rule would mistake sentence-initial nouns for
    // people, so the curated gazetteer decides instead.
    if contains(&gazetteer.persons, &lower) {
        return "PERSON".to_string();
    }
    if contains(&gazetteer.product_names, &lower)
        || gazetteer
            .product_substrings
            .iter()
            .any(|product| lower.contains(product.as_str()))
    {
        return "PRODUCT".to_string();
    }
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
    if case_is_evidence && title_tokens >= 2 && text.split_whitespace().count() >= 2 {
        return "PERSON".to_string();
    }
    // In German the tagger's PROPN substitutes for the case signal that the
    // orthography destroys: it restores person recall without letting every
    // capitalized noun phrase become a PERSON. In a caseless script PROPN only
    // says "proper noun" (a country, an organization, a person), so it is not
    // evidence of PERSON and the surface stays a visual concept.
    if lexicon.language_profiles.capitalized_nouns(language)
        && proper_noun
        && text.split_whitespace().count() >= 2
    {
        return "PERSON".to_string();
    }
    "VISUAL_CONCEPT".to_string()
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
                normalized: text.to_lowercase(),
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
            .find(|unit| tail.starts_with(unit.as_str()));
        let unit = unit.filter(|_| local_scale.is_none());
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
            } else if *unit == "percent" || *unit == "%" {
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
            normalized: text.to_lowercase(),
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
mod tests {
    use super::*;

    #[test]
    fn month_before_day_handles_multibyte_punctuation_before_date() {
        let source = "L’indagine proseguì fino a giugno 1988.";
        let number_start = source.find("1988").unwrap();
        let month_start = month_before_day(source, number_start).unwrap();
        assert_eq!(source.get(month_start..number_start), Some("giugno "));

        let candidates = value_candidates(source);
        assert!(candidates.iter().any(|candidate| {
            candidate.text == "giugno 1988"
                && source.get(candidate.start..candidate.end) == Some("giugno 1988")
        }));
    }

    #[test]
    fn capitalization_is_name_evidence_only_where_the_script_makes_it_so() {
        // Two capitalized words are a person in a cased script that keeps
        // common nouns lowercase.
        assert_eq!(classify_type("Alice Johnson", true, false, "it"), "PERSON");
        // Not in German: every noun is capitalized, so "Neue Halle" proves
        // nothing.
        assert_eq!(
            classify_type("Neue Halle", true, false, "de"),
            "VISUAL_CONCEPT"
        );
        // The tagger's PROPN restores person recall where case cannot, but a
        // single PROPN is not enough: "Deutschland" is a country.
        assert_eq!(classify_type("Alice Johnson", true, true, "de"), "PERSON");
        assert_eq!(
            classify_type("Deutschland", true, true, "de"),
            "VISUAL_CONCEPT"
        );
        // A caseless script never produces the capitalization signal, and a
        // bare PROPN is not evidence of PERSON there (a country is a PROPN).
        assert_eq!(
            classify_type("北京 大学", true, false, "zh"),
            "VISUAL_CONCEPT"
        );
        assert_eq!(classify_type("日本", true, true, "zh"), "VISUAL_CONCEPT");
    }
}
