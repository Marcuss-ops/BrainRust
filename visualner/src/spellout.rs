//! ICU4C/CLDR rule-based parsing for numbers written in words.
//!
//! ICU explicitly does not ship spellout rules for every locale. This module
//! accepts only formatters whose resolved data locale matches the requested
//! language, requires complete-input parsing, and returns original UTF-8 byte
//! spans rather than normalized text.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use rust_icu_sys::{ULocDataLocaleType, UNumberFormatAttribute, UNumberFormatStyle};
use rust_icu_uloc::ULoc;
use rust_icu_unum::UNumberFormat;

#[derive(Debug, Clone)]
pub(crate) struct WordNumberCandidate {
    pub start: usize,
    pub end: usize,
    pub value: f64,
    pub entity_type: &'static str,
}

#[derive(Debug, Clone)]
struct WordToken {
    start: usize,
    end: usize,
}

struct LocaleFormatters {
    locale: ULoc,
    spellout: Option<UNumberFormat>,
    ordinal: Option<UNumberFormat>,
}

/// Every normalized spelling the locale's own formatters can produce for a
/// small, representative number set: 0..=1000, the spoken thousands, the
/// powers of ten and the ordinals a script may use. ICU RBNF parsing is the
/// only correct oracle, but calling it for every 1..=8-token window of prose
/// costs seconds per long scene. This vocabulary is a cheap *prefilter*: a
/// window is parsed only when it contains one of these words (or starts with
/// a spoken thousands prefix such as `duemila`). The parser still decides the
/// value and the exact span, so the prefilter can only skip windows that
/// cannot possibly be numbers.
struct NumberVocabulary {
    words: BTreeSet<String>,
    thousands: Vec<String>,
}

fn number_vocabulary(language: &str) -> Result<Arc<NumberVocabulary>, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<NumberVocabulary>>>> = OnceLock::new();
    let key = language.trim().to_ascii_lowercase();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(vocabulary) = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
    {
        return Ok(vocabulary.clone());
    }
    let formatters = formatters(language)?;
    let mut words = BTreeSet::new();
    let mut thousands = Vec::new();
    if let Some(formatter) = &formatters.spellout {
        for value in 0..=1000i64 {
            if let Ok(spelled) = formatter.format_i64(value) {
                words.insert(normalize_spellout(&spelled));
            }
        }
        for multiplier in 1..=99i64 {
            if let Ok(spelled) = formatter.format_i64(multiplier * 1000) {
                let normalized = normalize_spellout(&spelled);
                thousands.push(normalized.clone());
                words.insert(normalized);
            }
        }
        // Scale words and their small multiples ("one million", "due milioni").
        for exponent in 6..=12u32 {
            let Some(base) = 10i64.checked_pow(exponent) else {
                break;
            };
            for multiplier in 1..=9i64 {
                if let Ok(spelled) = formatter.format_i64(base * multiplier) {
                    words.insert(normalize_spellout(&spelled));
                }
            }
        }
    }
    if let Some(formatter) = &formatters.ordinal {
        for value in 0..=100i64 {
            if let Ok(spelled) = formatter.format_i64(value) {
                words.insert(normalize_spellout(&spelled));
            }
        }
    }
    let vocabulary = Arc::new(NumberVocabulary { words, thousands });
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, vocabulary.clone());
    Ok(vocabulary)
}

/// A window is worth an RBNF parse only when a plausible number word opens
/// near it. The parser may legitimately start a phrase with a determiner
/// ("a hundred"), so the first three tokens are inspected.
fn plausible_number_start(
    source: &str,
    tokens: &[WordToken],
    start_index: usize,
    vocabulary: &NumberVocabulary,
) -> bool {
    let end = (start_index + 3).min(tokens.len());
    tokens[start_index..end].iter().any(|token| {
        let Some(surface) = source.get(token.start..token.end) else {
            return false;
        };
        if surface.chars().any(|character| character.is_ascii_digit()) {
            return false;
        }
        let normalized = normalize_spellout(surface);
        vocabulary.words.contains(&normalized)
            || vocabulary
                .thousands
                .iter()
                .any(|prefix| normalized.starts_with(prefix.as_str()))
    })
}

fn available_languages() -> &'static BTreeSet<String> {
    static LANGUAGES: OnceLock<BTreeSet<String>> = OnceLock::new();
    LANGUAGES.get_or_init(|| {
        ULoc::get_available_locales()
            .into_iter()
            .filter_map(|locale| locale.language())
            .map(|language| language.to_ascii_lowercase())
            .collect()
    })
}

fn formatters(language: &str) -> Result<LocaleFormatters, String> {
    let locale = ULoc::for_language_tag(language)
        .map_err(|error| format!("invalid VisualNER language {language:?}: {error}"))?;
    let requested_language = locale.language().unwrap_or_default().to_ascii_lowercase();
    if requested_language.is_empty() || !available_languages().contains(&requested_language) {
        return Err(format!(
            "ICU has no locale data for VisualNER language {language:?}"
        ));
    }

    let open = |style| {
        let mut formatter = UNumberFormat::try_new_with_style(style, &locale).ok()?;
        formatter.set_attribute(UNumberFormatAttribute::UNUM_PARSE_ALL_INPUT, 1);
        let resolved = formatter
            .get_locale_by_type(ULocDataLocaleType::ULOC_VALID_LOCALE)
            .or_else(|_| formatter.get_locale_by_type(ULocDataLocaleType::ULOC_ACTUAL_LOCALE))
            .ok()?;
        let resolved = ULoc::try_from(resolved).ok()?;
        if resolved.language().unwrap_or_default().to_ascii_lowercase() != requested_language {
            return None;
        }
        // Refuse decimal fallback masquerading as spellout data.
        let sample = formatter.format_i64(123).ok()?;
        if sample.chars().any(|character| character.is_numeric())
            && style != UNumberFormatStyle::UNUM_ORDINAL
        {
            return None;
        }
        Some(formatter)
    };

    let spellout = open(UNumberFormatStyle::UNUM_SPELLOUT);
    let ordinal = open(UNumberFormatStyle::UNUM_ORDINAL);
    Ok(LocaleFormatters {
        locale,
        spellout,
        ordinal,
    })
}

/// Return ICU's installed language coverage for usable spellout rules. This
/// is runtime data, not a promise that every rule set is complete or that all
/// grammatical contexts are recognized.
pub fn supported_spellout_language_count() -> usize {
    static COUNT: OnceLock<usize> = OnceLock::new();
    *COUNT.get_or_init(|| {
        available_languages()
            .iter()
            .filter(|language| {
                formatters(language)
                    .map(|formats| formats.spellout.is_some())
                    .unwrap_or(false)
            })
            .count()
    })
}

pub(crate) fn candidates(source: &str, language: &str) -> Result<Vec<WordNumberCandidate>, String> {
    let formatters = formatters(language)?;
    if source.is_empty() {
        return Ok(Vec::new());
    }
    if formatters.spellout.is_none() && formatters.ordinal.is_none() {
        return Err(format!(
            "ICU spellout rules are unavailable for language {language:?}"
        ));
    }
    let tokens = word_tokens(source);
    let vocabulary = number_vocabulary(language)?;
    let mut out = Vec::new();

    for start_index in 0..tokens.len() {
        // ICU RBNF parsing is the expensive part of this scan. Run it only
        // when a spelled-number word is near the window start; ordinary prose
        // windows cannot produce a number and must not pay for the parse.
        if !plausible_number_start(source, &tokens, start_index, &vocabulary) {
            continue;
        }
        // Generate candidates from each token boundary. The overlap pass below
        // keeps the widest complete number phrase; suppressing a start based
        // on ICU parsing the preceding prose token can discard a valid number.
        let max_end = (start_index + 8).min(tokens.len());
        for end_index in start_index..max_end {
            let start = tokens[start_index].start;
            let end = tokens[end_index].end;
            let surface = &source[start..end];
            if surface.len() > 120 {
                break;
            }
            if end_index > start_index
                && crosses_hard_punctuation(
                    &source[tokens[end_index - 1].end..tokens[end_index].start],
                )
            {
                break;
            }

            if surface.chars().any(|character| character.is_ascii_digit()) {
                continue;
            }
            let token_count = end_index - start_index + 1;
            let mut parsed = None;
            let mut entity_type = "NUMBER";
            let mut ordinal_parse = false;
            for (formatter, kind) in [
                (formatters.ordinal.as_ref(), "ORDINAL"),
                (formatters.spellout.as_ref(), "NUMBER"),
            ] {
                let Some(formatter) = formatter else { continue };
                let Ok(value) = formatter.parse_to_formattable(surface, Some(0)) else {
                    continue;
                };
                let Ok(number) = value.get_double() else {
                    continue;
                };
                if !number.is_finite() {
                    continue;
                }
                // ICU RBNF parsing may accept prefixes even with
                // UNUM_PARSE_ALL_INPUT. Only accept a complete canonical
                // round-trip generated by the locale's same rule set.
                let canonical = if kind == "ORDINAL" && number.fract() == 0.0 {
                    format_ordinal(number as i64, &formatters.locale)
                        .or_else(|| formatter.format_i64(number as i64).ok())
                } else if number.fract() == 0.0
                    && number >= i64::MIN as f64
                    && number <= i64::MAX as f64
                {
                    formatter.format_i64(number as i64).ok()
                } else {
                    None
                };
                let Some(canonical) = canonical else { continue };
                let canonical_matches = normalize_spellout(surface)
                    == normalize_spellout(&canonical)
                    || (kind == "ORDINAL" && ordinal_equivalent(surface, &canonical));
                let value = if canonical_matches {
                    Some(number)
                } else if kind == "NUMBER"
                    && number.fract() == 0.0
                    && normalize_spellout(surface).starts_with(&normalize_spellout(&canonical))
                {
                    // Some ICU RBNF parsers stop at a valid prefix instead
                    // of consuming a locale's concatenated compound form
                    // (Italian "venticinque" parses as 20). Recover only by
                    // finding an exact formatter-produced spelling nearby;
                    // never accept a prefix or normalize away source text.
                    let lower = (number as i64).saturating_sub(100).max(0);
                    let upper = (number as i64).saturating_add(100).min(999);
                    (lower..=upper).find_map(|candidate| {
                        let formatted = formatter.format_i64(candidate).ok()?;
                        (normalize_spellout(surface) == normalize_spellout(&formatted))
                            .then_some(candidate as f64)
                    })
                } else {
                    None
                };
                let Some(number) = value else { continue };
                parsed = Some(number);
                entity_type = kind;
                ordinal_parse = kind == "ORDINAL";
                break;
            }
            let value = match parsed {
                Some(value) => value,
                None => {
                    // CLDR's RBNF parser rejects compound thousands in some
                    // locales (Italian "duemilaventicinque" parses only as a
                    // prefix). Rebuild the number from the locale's own
                    // formatted thousands word, then parse the remainder with
                    // the same rules. No language table is involved.
                    let Some(formatter) = formatters.spellout.as_ref() else {
                        continue;
                    };
                    let Some(value) = compose_thousands(surface, formatter) else {
                        continue;
                    };
                    value
                }
            };

            // Common words for 0/1 are ambiguous in prose. Require a unit
            // for those, and do not treat duration phrases such as "one day"
            // as standalone numbers.
            let magnitude = value.abs();
            let suffix = quantity_suffix(source, end);
            if !ordinal_parse
                && magnitude <= 1.0
                && token_count <= 2
                && !suffix.is_some_and(|(_, kind)| kind != "TIME")
            {
                // Short spellings such as "one"/"zero" are ambiguous in
                // prose and durations, but a typed quantity such as "one
                // percent" is meaningful evidence.
                continue;
            }

            let (candidate_end, classified_type) = extend_quantity(source, end, entity_type);
            // A spelled year with no unit is a date, exactly like a digit year
            // ("42" is a count, "2007" is a year). Typing it as a number
            // would let a bar chart plot it.
            let classified_type = if classified_type == "NUMBER"
                && !ordinal_parse
                && suffix.is_none()
                && value.fract() == 0.0
                && (1000.0..=2099.0).contains(&value)
            {
                "DATE"
            } else {
                classified_type
            };
            out.push(WordNumberCandidate {
                start,
                end: candidate_end,
                value,
                entity_type: classified_type,
            });
            // Prefer a suffix-extended quantity; otherwise continue so the
            // longest complete number phrase wins for this token start.
            if candidate_end > end {
                break;
            }
        }
    }

    // Keep only the widest candidate for identical starts, with typed units
    // taking priority on equal spans. This also makes output deterministic.
    out.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.entity_type.cmp(b.entity_type))
    });
    let mut filtered: Vec<WordNumberCandidate> = Vec::with_capacity(out.len());
    for candidate in out {
        if let Some(previous) = filtered.last_mut() {
            if candidate.start < previous.end {
                if candidate.start == previous.start && candidate.end > previous.end {
                    *previous = candidate;
                }
                continue;
            }
        }
        filtered.push(candidate);
    }
    Ok(filtered)
}

/// Rebuilds a thousands compound from the locale's own RBNF output:
/// "duemilaventicinque" = format(2000) + parse("venticinque"),
/// "two thousand seven" = format(2000) + parse("seven"). This is what keeps
/// spelled years working without a per-language word table.
fn compose_thousands(surface: &str, formatter: &UNumberFormat) -> Option<f64> {
    let normalized = normalize_spellout(surface);
    for thousands in 1..=99i64 {
        let amount = thousands * 1000;
        let Ok(formatted) = formatter.format_i64(amount) else {
            continue;
        };
        let prefix = normalize_spellout(&formatted);
        if prefix.is_empty() || !normalized.starts_with(&prefix) {
            continue;
        }
        let remainder = &normalized[prefix.len()..];
        if remainder.is_empty() {
            return Some(amount as f64);
        }
        let Some(rest) = parse_complete(remainder, formatter) else {
            continue;
        };
        if (0.0..1000.0).contains(&rest) {
            return Some(amount as f64 + rest);
        }
    }
    None
}

/// Complete parse with the same canonical round-trip rule used by the main
/// loop, including the compound-prefix recovery for "venticinque" (20 + 5).
fn parse_complete(surface: &str, formatter: &UNumberFormat) -> Option<f64> {
    let value = formatter.parse_to_formattable(surface, Some(0)).ok()?;
    let number = value.get_double().ok()?;
    if !number.is_finite() {
        return None;
    }
    let normalized = normalize_spellout(surface);
    if number.fract() == 0.0
        && number >= i64::MIN as f64
        && number <= i64::MAX as f64
        && formatter
            .format_i64(number as i64)
            .ok()
            .is_some_and(|canonical| normalize_spellout(&canonical) == normalized)
    {
        return Some(number);
    }
    let lower = (number as i64).saturating_sub(100).max(0);
    let upper = (number as i64).saturating_add(100).min(999);
    (lower..=upper).find_map(|candidate| {
        let formatted = formatter.format_i64(candidate).ok()?;
        (normalize_spellout(&formatted) == normalized).then_some(candidate as f64)
    })
}

fn word_tokens(source: &str) -> Vec<WordToken> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut previous_was_word = false;
    for (offset, character) in source.char_indices() {
        let word = character.is_alphanumeric() || is_combining_mark(character);
        let connector = matches!(
            character,
            '-' | '\u{2010}' | '\u{2011}' | '\u{00AD}' | '\'' | '\u{2019}'
        ) && previous_was_word
            && source[offset + character.len_utf8()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if word || connector {
            start.get_or_insert(offset);
            previous_was_word = word;
        } else {
            if let Some(token_start) = start.take() {
                tokens.push(WordToken {
                    start: token_start,
                    end: offset,
                });
            }
            previous_was_word = false;
        }
    }
    if let Some(token_start) = start {
        tokens.push(WordToken {
            start: token_start,
            end: source.len(),
        });
    }
    tokens
}

fn is_combining_mark(character: char) -> bool {
    matches!(character as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF |
        0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

fn ordinal_equivalent(surface: &str, canonical: &str) -> bool {
    let mut left = surface.to_lowercase();
    let mut right = canonical.to_lowercase();
    for ordinal_suffix in ["st", "nd", "rd", "th"] {
        if let Some(prefix) = left.strip_suffix(ordinal_suffix) {
            left = prefix.to_string();
            break;
        }
    }
    for ordinal_suffix in ["st", "nd", "rd", "th"] {
        if let Some(prefix) = right.strip_suffix(ordinal_suffix) {
            right = prefix.to_string();
            break;
        }
    }
    normalize_spellout(&left) == normalize_spellout(&right)
}

fn format_ordinal(number: i64, locale: &ULoc) -> Option<String> {
    let formatter =
        UNumberFormat::try_new_with_style(UNumberFormatStyle::UNUM_ORDINAL, locale).ok()?;
    formatter.format_i64(number).ok()
}

fn normalize_spellout(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            !character.is_whitespace()
                && !matches!(
                    character,
                    '-' | '\u{2010}' | '\u{2011}' | '\u{00AD}' | '\'' | '\u{2019}'
                )
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn crosses_hard_punctuation(separator: &str) -> bool {
    separator.chars().any(|character| {
        matches!(
            character,
            '.' | '!' | '?' | ';' | ':' | '。' | '！' | '？' | '؛'
        )
    })
}

fn quantity_suffix(source: &str, end: usize) -> Option<(&'static str, &'static str)> {
    let tail = source.get(end..)?.trim_start();
    let lower = tail.to_lowercase();
    let suffixes = [
        ("percent", "PERCENT"),
        ("per cent", "PERCENT"),
        ("per cento", "PERCENT"),
        ("pour cent", "PERCENT"),
        ("por cento", "PERCENT"),
        ("percento", "PERCENT"),
        ("prozent", "PERCENT"),
        ("процентов", "PERCENT"),
        ("процента", "PERCENT"),
        ("процент", "PERCENT"),
        ("dollars", "MONEY"),
        ("dollar", "MONEY"),
        ("euros", "MONEY"),
        ("euro", "MONEY"),
        ("pounds", "MONEY"),
        ("pound", "MONEY"),
        ("yen", "MONEY"),
        ("рублей", "MONEY"),
        ("рубля", "MONEY"),
        ("million", "MONEY"),
        ("billion", "MONEY"),
        ("trillion", "MONEY"),
        ("mille", "NUMBER"),
        ("milioni", "NUMBER"),
        ("millions", "NUMBER"),
        ("orders", "NUMBER"),
        ("people", "NUMBER"),
        ("years", "TIME"),
        ("year", "TIME"),
        ("days", "TIME"),
        ("day", "TIME"),
        ("hours", "TIME"),
        ("hour", "TIME"),
        ("minutes", "TIME"),
        ("minute", "TIME"),
        ("votes", "NUMBER"),
        ("ordini", "NUMBER"),
        ("persone", "NUMBER"),
        ("ans", "NUMBER"),
        ("jahre", "NUMBER"),
        ("bestellungen", "NUMBER"),
        ("años", "NUMBER"),
        ("personas", "NUMBER"),
        ("pessoas", "NUMBER"),
    ];
    suffixes.into_iter().find(|(suffix, _)| {
        lower.starts_with(suffix)
            && lower
                .chars()
                .nth(suffix.chars().count())
                .is_none_or(|next| !next.is_alphanumeric())
    })
}

fn extend_quantity(source: &str, end: usize, default_type: &'static str) -> (usize, &'static str) {
    let Some((suffix, kind)) = quantity_suffix(source, end) else {
        return (end, default_type);
    };
    let suffix_start = end + source[end..].len() - source[end..].trim_start().len();
    (
        suffix_start + suffix.len(),
        if kind == "TIME" { "NUMBER" } else { kind },
    )
}
