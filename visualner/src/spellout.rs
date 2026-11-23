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
use semantic_core::number_words::NumberWordRules;

#[path = "spellout/quantity.rs"]
mod quantity;
#[path = "spellout/tokens.rs"]
mod tokens;
use tokens::WordToken;

#[derive(Debug, Clone)]
pub(crate) struct WordNumberCandidate {
    pub start: usize,
    pub end: usize,
    pub value: f64,
    pub entity_type: &'static str,
    /// True when the candidate already includes a unit suffix or is
    /// immediately followed by an open-class noun ("sedici anni", "cinque
    /// consulenze", "five reports"). A bare count ("quarantadue dei
    /// tecnici") stays false: it is not a measurement.
    pub has_unit: bool,
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
    rules: Option<&NumberWordRules>,
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
        let half_compound = rules.is_some_and(|rules| {
            rules
                .half_suffixes
                .iter()
                .any(|suffix| normalized.ends_with(&suffix.to_lowercase()))
                || rules.variants.contains_key(&normalized)
        });
        half_compound
            || vocabulary.words.contains(&normalized)
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
    let tokens = tokens::word_tokens(source);
    let vocabulary = number_vocabulary(language)?;
    let rules = semantic_core::number_words::rules(language);
    let mut out = denominator_fractions(source, &tokens, rules, formatters.spellout.as_ref());

    for start_index in 0..tokens.len() {
        // ICU RBNF parsing is the expensive part of this scan. Run it only
        // when a spelled-number word is near the window start; ordinary prose
        // windows cannot produce a number and must not pay for the parse.
        if !plausible_number_start(source, &tokens, start_index, &vocabulary, rules) {
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
            // CLDR's parsers are case-sensitive: a sentence-initial "Tre" is
            // not accepted while "tre" is. Number words are case-insensitive
            // in every supported language, so parse the folded copy and keep
            // the original span for grounding.
            let parse_surface = surface.to_lowercase();
            let variants = parse_variants(&parse_surface, rules);
            let mut parsed = None;
            let mut entity_type = "NUMBER";
            let mut ordinal_parse = false;
            'formatters: for (formatter, kind) in [
                (formatters.ordinal.as_ref(), "ORDINAL"),
                (formatters.spellout.as_ref(), "NUMBER"),
            ] {
                let Some(formatter) = formatter else { continue };
                for variant in &variants {
                    let Ok(value) = formatter.parse_to_formattable(variant, Some(0)) else {
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
                    let canonical_matches = normalize_spellout(variant)
                        == normalize_spellout(&canonical)
                        || (kind == "ORDINAL" && ordinal_equivalent(variant, &canonical));
                    let value = if canonical_matches {
                        Some(number)
                    } else if kind == "NUMBER"
                        && number.fract() == 0.0
                        && normalize_spellout(variant).starts_with(&normalize_spellout(&canonical))
                    {
                        // Some ICU RBNF parsers stop at a valid prefix instead
                        // of consuming a locale's concatenated compound form
                        // (Italian "venticinque" parses as 20). Recover only by
                        // finding an exact formatter-produced spelling nearby;
                        // never accept a prefix or normalize away source text.
                        // CLDR may parse a compound as its first element only:
                        // Italian "duecento" parses as 2, "venticinque" as 20.
                        // Search the full sub-thousand range so the formatted
                        // compound (200, 25) is found; the round-trip still
                        // decides, so a wrong candidate cannot win.
                        let lower = 0;
                        let upper = 999;
                        (lower..=upper).find_map(|candidate| {
                            let formatted = formatter.format_i64(candidate).ok()?;
                            (normalize_spellout(variant) == normalize_spellout(&formatted))
                                .then_some(candidate as f64)
                        })
                    } else {
                        None
                    };
                    let Some(number) = value else { continue };
                    parsed = Some(number);
                    entity_type = kind;
                    ordinal_parse = kind == "ORDINAL";
                    break 'formatters;
                }
            }
            let value = match parsed {
                Some(value) => value,
                None => {
                    // CLDR's RBNF parser rejects compound thousands in some
                    // locales (Italian "duemilaventicinque" parses only as a
                    // prefix, Spanish "mil doscientos" not at all). Rebuild the
                    // number from the locale's own tokens, then fall back to
                    // the formatted-thousands recovery. No language table is
                    // involved beyond the grammar flags in number_words.json.
                    let Some(formatter) = formatters.spellout.as_ref() else {
                        continue;
                    };
                    let Some(value) = compose_run(surface, rules, formatter)
                        .or_else(|| parse_half_compound(surface, rules, formatter))
                        .or_else(|| compose_thousands(surface, formatter))
                    else {
                        continue;
                    };
                    value
                }
            };

            // Common words for 0/1 are ambiguous in prose. Require a unit
            // for those, and do not treat duration phrases such as "one day"
            // as standalone numbers.
            let magnitude = value.abs();
            let suffix = quantity::quantity_suffix(source, end);
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

            let (candidate_end, classified_type) =
                quantity::extend_quantity(source, end, entity_type);
            // CLDR parses "tre milioni" as the prefix `3` in several locales,
            // because the scale noun is a suffix of the window. Apply the
            // shared scale table to the surface that was actually consumed:
            // the value is multiplied only when it is smaller than the scale
            // itself, so an already-complete parse ("3.000.000") is untouched.
            let value = match quantity::suffix_scale_factor(source, end, candidate_end) {
                Some(factor) if value.abs() < factor => value * factor,
                _ => value,
            };
            // A scale noun after the window ("Zweieinhalb Millionen") is a
            // suffix CLDR does not fold into the number. Consume it with the
            // same shared table; the currency word, if any, stays outside.
            let (candidate_end, value) = extend_scale_word(source, candidate_end, value);
            // "due milioni e mezzo" / "two million and a half": the additive
            // half belongs to the same quantity, so it extends the span and
            // adds one half of the scale that was just consumed.
            let (candidate_end, value) =
                extend_additive_fraction(source, start, end, candidate_end, value, rules);
            let start = approximator_start(source, &tokens, start_index, rules).unwrap_or(start);
            // A list of idioms that must never become a number. The window
            // covers two words before and three after the candidate so
            // "grazie mille" and "mille grazie" are both caught.
            if rules.is_some_and(|_| {
                let from = tokens[start_index.saturating_sub(2)].start;
                let to_index = (end_index + 3).min(tokens.len()) - 1;
                let to = tokens[to_index].end;
                semantic_core::number_words::contains_negative_phrase(&source[from..to], language)
            }) {
                continue;
            }
            let has_unit = candidate_end > end
                || quantity::next_token_is_open_class(source, candidate_end, language);
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
                has_unit,
            });
            // Prefer a unit/currency-extended quantity; a bare scale word must
            // not stop the scan, because the phrase may continue
            // ("due mila sette" = 2007, not 2000 + 7).
            if candidate_end > end && quantity::quantity_suffix(source, end).is_some() {
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

#[path = "spellout/compose.rs"]
mod compose;

use compose::{
    approximator_start, compose_run, compose_thousands, crosses_hard_punctuation,
    denominator_fractions, extend_additive_fraction, extend_scale_word, format_ordinal,
    normalize_spellout, ordinal_equivalent, parse_half_compound, parse_variants,
};

#[cfg(test)]
#[path = "spellout_tests.rs"]
mod number_words_gold;
