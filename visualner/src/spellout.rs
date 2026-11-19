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
    let tokens = word_tokens(source);
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
            // CLDR parses "tre milioni" as the prefix `3` in several locales,
            // because the scale noun is a suffix of the window. Apply the
            // shared scale table to the surface that was actually consumed:
            // the value is multiplied only when it is smaller than the scale
            // itself, so an already-complete parse ("3.000.000") is untouched.
            let value = match suffix_scale_factor(source, end, candidate_end) {
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
            let has_unit =
                candidate_end > end || next_token_is_open_class(source, candidate_end, language);
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
            if candidate_end > end && quantity_suffix(source, end).is_some() {
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

/// Original surface plus a conjunction-free variant. CLDR spells Italian
/// 1200 as the fused "milleduecento" while scripts also write "mille e
/// duecento"; removing the language conjunction is the only difference. Both
/// variants keep the original byte span for grounding.
fn parse_variants(surface: &str, rules: Option<&NumberWordRules>) -> Vec<String> {
    let mut variants = vec![surface.to_string()];
    let Some(rules) = rules else {
        return variants;
    };
    if rules.conjunctions.is_empty() {
        return variants;
    }
    let words: Vec<&str> = surface.split_whitespace().collect();
    let stripped: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| {
            !rules
                .conjunctions
                .iter()
                .any(|conjunction| conjunction.eq_ignore_ascii_case(word))
        })
        .collect();
    // Only an internal conjunction may be dropped. A leading "and five" or
    // trailing "mille e" would otherwise move the span onto the conjunction
    // instead of the number.
    if stripped.len() != words.len()
        && !stripped.is_empty()
        && stripped.first() == words.first()
        && stripped.last() == words.last()
    {
        variants.push(stripped.join(" "));
    }
    variants
}

#[derive(Debug, Clone, Copy)]
enum Component {
    Unit(f64),
    Ten(f64),
    Hundred(f64),
    Scale(f64),
    Half(semantic_core::number_words::FractionSpec),
}

fn classify_component(value: f64) -> Option<Component> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return None;
    }
    if (100.0..1000.0).contains(&value) && (value as i64) % 100 == 0 {
        return Some(Component::Hundred(value));
    }
    if (20.0..100.0).contains(&value) && (value as i64) % 10 == 0 {
        return Some(Component::Ten(value));
    }
    if value < 20.0 {
        return Some(Component::Unit(value));
    }
    None
}

/// Additive/multiplicative composition over the locale's own token values.
/// This is what turns Spanish "mil doscientos", Turkish "bin iki yüz" and
/// Italian "mille e duecento" into 1200: CLDR parses each token, the grammar
/// combines them. It runs only when a direct CLDR parse of the whole window
/// failed, so existing compound spellings keep their exact behaviour.
fn compose_run(
    surface: &str,
    rules: Option<&NumberWordRules>,
    formatter: &UNumberFormat,
) -> Option<f64> {
    let rules = rules?;
    let words: Vec<&str> = surface.split_whitespace().collect();
    let mut components = Vec::new();
    for (index, raw) in words.iter().enumerate() {
        let word = raw
            .trim_matches(|character: char| {
                !character.is_alphanumeric() && !matches!(character, '-' | '\'' | '\u{2019}')
            })
            .to_lowercase();
        if word.is_empty()
            || rules
                .conjunctions
                .iter()
                .any(|conjunction| conjunction.eq_ignore_ascii_case(&word))
            || rules
                .approximators
                .iter()
                .any(|approximator| approximator.eq_ignore_ascii_case(&word))
        {
            continue;
        }
        // "two and a half million": the article belongs to the fraction and
        // is not a number of its own. Drop it only in that position, so
        // "one hundred" keeps its one.
        let next_is_additive = words
            .get(index + 1)
            .map(|next| {
                let next = next
                    .trim_matches(|character: char| {
                        !character.is_alphanumeric()
                            && !matches!(character, '-' | '\'' | '\u{2019}')
                    })
                    .to_lowercase();
                rules.fractions.get(&next).is_some_and(|spec| spec.additive)
            })
            .unwrap_or(false);
        if next_is_additive
            && rules
                .fraction_articles
                .iter()
                .any(|article| article.eq_ignore_ascii_case(&word))
        {
            continue;
        }
        if let Some(spec) = rules.fractions.get(&word) {
            if spec.additive {
                components.push(Component::Half(*spec));
            }
            // Denominator fractions are emitted by denominator_fractions().
            continue;
        }
        if let Some(scale) = semantic_core::scale_word_factor(&word) {
            components.push(Component::Scale(scale));
            continue;
        }
        if let Some(value) = rules.variants.get(&word) {
            components.push(classify_component(*value)?);
            continue;
        }
        let value = parse_complete(&word, formatter)?;
        components.push(classify_component(value)?);
    }
    if components.len() < 2 {
        return None;
    }
    let mut total = 0.0;
    let mut current = 0.0;
    let mut last_scale = None;
    for component in components {
        match component {
            Component::Scale(scale) => {
                total += if current == 0.0 {
                    scale
                } else {
                    current * scale
                };
                current = 0.0;
                last_scale = Some(scale);
            }
            Component::Hundred(hundred) => {
                if current == 0.0 {
                    current = hundred;
                } else if current < 10.0 {
                    current *= hundred;
                } else {
                    return None;
                }
            }
            Component::Ten(ten) => {
                if current == 0.0 {
                    current = ten;
                } else if current < 10.0 && rules.multiplicative_tens {
                    current *= ten;
                } else if current >= 100.0 || (rules.multiplicative_tens && current >= 20.0) {
                    current += ten;
                } else {
                    return None;
                }
            }
            Component::Unit(unit) => {
                if current == 0.0 {
                    current = unit;
                } else if (unit < 10.0 && current >= 20.0) || current >= 100.0 {
                    current += unit;
                } else {
                    return None;
                }
            }
            Component::Half(spec) => {
                let unit = last_scale.unwrap_or(1.0);
                let addition = unit / f64::from(spec.denominator);
                if current > 0.0 {
                    current += addition;
                } else {
                    total += addition;
                }
            }
        }
    }
    let value = total + current;
    (value > 0.0).then_some(value)
}

/// Consumes one scale noun immediately after `candidate_end`, multiplying the
/// value when the parsed number is smaller than the scale. This covers languages
/// whose scale noun is a separate word and absent from `quantity_suffix`.
fn extend_scale_word(source: &str, candidate_end: usize, value: f64) -> (usize, f64) {
    let tail = &source[candidate_end.min(source.len())..];
    let tokens = word_tokens(tail);
    let Some(token) = tokens.first() else {
        return (candidate_end, value);
    };
    if !tail[..token.start].chars().all(char::is_whitespace) {
        return (candidate_end, value);
    }
    let word = &tail[token.start..token.end];
    let Some(factor) = semantic_core::scale_word_factor(word) else {
        return (candidate_end, value);
    };
    if value.abs() < factor {
        (candidate_end + token.end, value * factor)
    } else {
        (candidate_end, value)
    }
}

/// Last scale word inside an already-parsed quantity (`Dois milhões` carries
/// the scale inside the window, not as a suffix extension). The additive half
/// is one half of that scale.
fn last_scale_factor(source: &str, start: usize, end: usize) -> Option<f64> {
    source
        .get(start..end)?
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter_map(semantic_core::scale_word_factor)
        .next_back()
}

/// Extends a parsed quantity with an additive fraction ("due milioni e
/// mezzo", "two million and a half"). The added part is one `denominator` of
/// the scale just consumed, or one unit when no scale is present.
fn extend_additive_fraction(
    source: &str,
    number_start: usize,
    number_end: usize,
    candidate_end: usize,
    value: f64,
    rules: Option<&NumberWordRules>,
) -> (usize, f64) {
    let Some(rules) = rules else {
        return (candidate_end, value);
    };
    let tail = &source[candidate_end.min(source.len())..];
    let tail_tokens = word_tokens(tail);
    let mut index = 0;
    if let Some(first) = tail_tokens.first() {
        let word = &tail[first.start..first.end].to_lowercase();
        if rules
            .conjunctions
            .iter()
            .any(|conjunction| conjunction.eq_ignore_ascii_case(word))
        {
            index += 1;
        }
    }
    if let Some(article) = tail_tokens.get(index) {
        let word = &tail[article.start..article.end].to_lowercase();
        if rules
            .fraction_articles
            .iter()
            .any(|known| known.eq_ignore_ascii_case(word))
        {
            index += 1;
        }
    }
    let Some(fraction) = tail_tokens.get(index) else {
        return (candidate_end, value);
    };
    let word = &tail[fraction.start..fraction.end].to_lowercase();
    let Some(spec) = rules.fractions.get(word) else {
        return (candidate_end, value);
    };
    if !spec.additive {
        return (candidate_end, value);
    }
    let scale_before = suffix_scale_factor(source, number_end, candidate_end)
        .or_else(|| last_scale_factor(source, number_start, number_end));
    let unit = scale_before.unwrap_or(1.0);
    let mut end = candidate_end + fraction.end;
    let mut value = value + unit / f64::from(spec.denominator);
    // English puts the scale after the half ("two and a half million"). The
    // scale was not consumed before the fraction, so apply it now and extend
    // the span over it; a scale already consumed before the fraction ("due
    // milioni e mezzo") must not be applied twice.
    if scale_before.is_none() {
        let tail = &source[end.min(source.len())..];
        if let Some(scale_token) = word_tokens(tail).first() {
            let word = &tail[scale_token.start..scale_token.end];
            if let Some(factor) = semantic_core::scale_word_factor(word) {
                value *= factor;
                end += scale_token.end;
            }
        }
    }
    (end, value)
}

/// Fused half compounds (`zweieinhalb`, `tweeënhalf`). The suffix list is
/// data; the prefix is parsed by the locale's own cardinal rules.
fn parse_half_compound(
    surface: &str,
    rules: Option<&NumberWordRules>,
    formatter: &UNumberFormat,
) -> Option<f64> {
    let rules = rules?;
    let word = surface.trim().to_lowercase();
    rules.half_suffixes.iter().find_map(|suffix| {
        let prefix = word.strip_suffix(&suffix.to_lowercase())?.trim();
        if prefix.is_empty() {
            return None;
        }
        parse_complete(prefix, formatter).map(|base| base + 0.5)
    })
}

/// Denominator fractions are their own fact ("un terzo" = 1/3). The numerator
/// is the article or the preceding number word; without either it is 1.
fn denominator_fractions(
    source: &str,
    tokens: &[WordToken],
    rules: Option<&NumberWordRules>,
    formatter: Option<&UNumberFormat>,
) -> Vec<WordNumberCandidate> {
    let (Some(rules), Some(formatter)) = (rules, formatter) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let word = source[token.start..token.end].to_lowercase();
        let Some(spec) = rules.fractions.get(&word) else {
            continue;
        };
        if spec.additive {
            continue;
        }
        let mut numerator = 1.0;
        let mut start = token.start;
        if let Some(previous) = index.checked_sub(1).and_then(|index| tokens.get(index)) {
            let previous_word = source[previous.start..previous.end].to_lowercase();
            if rules
                .fraction_articles
                .iter()
                .any(|article| article.eq_ignore_ascii_case(&previous_word))
            {
                start = previous.start;
            } else if let Some(value) = parse_complete(&previous_word, formatter) {
                if value.fract() == 0.0 && (1.0..100.0).contains(&value) {
                    numerator = value;
                    start = previous.start;
                }
            }
        }
        out.push(WordNumberCandidate {
            start,
            end: token.end,
            value: numerator / f64::from(spec.denominator),
            entity_type: "FRACTION",
            has_unit: true,
        });
    }
    let words = semantic_core::discourse::word_spans(source);
    for (index, _) in words.iter().enumerate() {
        for (phrase, spec) in &rules.fraction_phrases {
            let parts: Vec<&str> = phrase.split_whitespace().collect();
            if parts.is_empty() || index + parts.len() > words.len() {
                continue;
            }
            if parts
                .iter()
                .enumerate()
                .all(|(offset, part)| words[index + offset].lowered == *part)
            {
                out.push(WordNumberCandidate {
                    start: words[index].start,
                    end: words[index + parts.len() - 1].end,
                    value: 1.0 / f64::from(spec.denominator),
                    entity_type: "FRACTION",
                    has_unit: true,
                });
            }
        }
    }
    out
}

/// Start of a preceding approximator ("circa cinquemila"). The span includes
/// the cue, the value stays exact; the cue is not copied into any query.
fn approximator_start(
    source: &str,
    tokens: &[WordToken],
    start_index: usize,
    rules: Option<&NumberWordRules>,
) -> Option<usize> {
    let rules = rules?;
    for approximator in &rules.approximators {
        let parts: Vec<&str> = approximator.split_whitespace().collect();
        if parts.is_empty() || start_index < parts.len() {
            continue;
        }
        let first = start_index - parts.len();
        let matches = parts.iter().enumerate().all(|(offset, part)| {
            let token = &tokens[first + offset];
            source[token.start..token.end].eq_ignore_ascii_case(part)
        });
        if !matches {
            continue;
        }
        let separator = source.get(tokens[first].end..tokens[first + 1].start);
        let separator_ok = parts.len() == 1
            || separator.is_some_and(|value| value.chars().all(char::is_whitespace));
        if separator_ok {
            return Some(tokens[first].start);
        }
    }
    None
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
    let lower = 0;
    let upper = 999;
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
        ("personnes", "NUMBER"),
        ("menschen", "NUMBER"),
        ("mensen", "NUMBER"),
        ("osób", "NUMBER"),
        ("человек", "NUMBER"),
        ("kişi", "NUMBER"),
        ("insan", "NUMBER"),
        ("orang", "NUMBER"),
    ];
    suffixes.into_iter().find(|(suffix, _)| {
        lower.starts_with(suffix)
            && lower
                .chars()
                .nth(suffix.chars().count())
                .is_none_or(|next| !next.is_alphanumeric())
    })
}

/// True when the token immediately after `end` is an open-class word, which
/// makes it a unit for the preceding spelled quantity. A preposition or
/// article from the language pack means the number is a bare count ("quaranta
/// dei tecnici"), not a measurement. Without a pack the check fails closed:
/// an unprovable unit never enters the numeric projection.
fn next_token_is_open_class(source: &str, end: usize, language: &str) -> bool {
    let Some(tail) = source.get(end..) else {
        return false;
    };
    let tail = tail.trim_start();
    let Some(first) = tail.chars().next() else {
        return false;
    };
    if !first.is_alphanumeric() {
        return false;
    }
    let word_end = tail
        .char_indices()
        .find(|(_, character)| !character.is_alphanumeric())
        .map(|(index, _)| index)
        .unwrap_or(tail.len());
    let word = tail[..word_end].to_lowercase();
    semantic_core::language_packs::get(language)
        .map(|pack| !pack.is_stopword(&word))
        .unwrap_or(false)
}

/// Scale word among the tokens consumed by [`extend_quantity`]. The table is
/// `semantic_core::scale_word_factor`, so a new scale spelling is a data edit
/// in `lexicon/scale_words.json` (and its factor table), never a second copy.
fn suffix_scale_factor(source: &str, end: usize, candidate_end: usize) -> Option<f64> {
    source
        .get(end..candidate_end)?
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .find_map(semantic_core::scale_word_factor)
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

#[cfg(test)]
mod number_words_gold {
    use super::*;

    // Assistant draft pending human review, as required by the semantic
    // expansion roadmap. Expected values are independent of the scanner.
    const VALUES: [i64; 21] = [
        2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 20, 21, 30, 42, 100, 1200, 2000, 1_000_000,
    ];

    const FRACTIONS: &[(&str, &str)] = &[
        ("it", "un terzo"),
        ("en", "one third"),
        ("es", "un tercio"),
        ("pt", "um terço"),
        ("fr", "un tiers"),
        ("de", "ein Drittel"),
        ("nl", "een derde"),
        ("pl", "jedna trzecia"),
        ("ru", "одна треть"),
        ("tr", "üçte biri"),
        ("id", "sepertiga"),
    ];

    const APPROXIMATORS: &[(&str, &str)] = &[
        ("it", "circa"),
        ("en", "about"),
        ("es", "alrededor de"),
        ("pt", "cerca de"),
        ("fr", "environ"),
        ("de", "rund"),
        ("nl", "ongeveer"),
        ("pl", "około"),
        ("ru", "около"),
        ("tr", "yaklaşık"),
        ("id", "sekitar"),
    ];

    const UNITS: &[(&str, &str)] = &[
        ("it", "persone"),
        ("en", "people"),
        ("es", "personas"),
        ("pt", "pessoas"),
        ("fr", "personnes"),
        ("de", "Menschen"),
        ("nl", "mensen"),
        ("pl", "osób"),
        ("ru", "человек"),
        ("tr", "kişi"),
        ("id", "orang"),
    ];

    const NEGATIVES: &[(&str, &[&str; 20])] = &[
        (
            "it",
            &[
                "il",
                "lo",
                "la",
                "i",
                "gli",
                "le",
                "un",
                "una",
                "di",
                "da",
                "in",
                "con",
                "su",
                "per",
                "tra",
                "il gatto",
                "la casa",
                "mille grazie",
                "grazie mille",
                "mille scuse",
            ],
        ),
        (
            "en",
            &[
                "the",
                "a",
                "an",
                "of",
                "to",
                "in",
                "with",
                "on",
                "for",
                "by",
                "from",
                "at",
                "it",
                "they",
                "we",
                "the cat",
                "a house",
                "thanks a million",
                "a million thanks",
                "one in a million",
            ],
        ),
        (
            "es",
            &[
                "el",
                "la",
                "los",
                "las",
                "un",
                "una",
                "de",
                "a",
                "en",
                "con",
                "por",
                "para",
                "sin",
                "sobre",
                "entre",
                "el gato",
                "la casa",
                "mil gracias",
                "gracias mil",
                "por favor",
            ],
        ),
        (
            "pt",
            &[
                "o",
                "a",
                "os",
                "as",
                "um",
                "uma",
                "de",
                "do",
                "da",
                "em",
                "com",
                "por",
                "para",
                "sem",
                "entre",
                "o gato",
                "a casa",
                "mil obrigados",
                "obrigado mil vezes",
                "por favor",
            ],
        ),
        (
            "fr",
            &[
                "le",
                "la",
                "les",
                "un",
                "une",
                "de",
                "du",
                "des",
                "en",
                "avec",
                "par",
                "pour",
                "sans",
                "sur",
                "entre",
                "le chat",
                "la maison",
                "mille mercis",
                "merci mille fois",
                "s'il vous plaît",
            ],
        ),
        (
            "de",
            &[
                "der",
                "die",
                "das",
                "den",
                "dem",
                "des",
                "ein",
                "eine",
                "von",
                "zu",
                "in",
                "mit",
                "auf",
                "für",
                "ohne",
                "die Katze",
                "das Haus",
                "tausend dank",
                "vielen dank",
                "bitte sehr",
            ],
        ),
        (
            "nl",
            &[
                "de",
                "het",
                "een",
                "van",
                "naar",
                "in",
                "met",
                "op",
                "voor",
                "zonder",
                "tussen",
                "over",
                "bij",
                "aan",
                "door",
                "de kat",
                "het huis",
                "duizendmaal dank",
                "hartelijk dank",
                "alsjeblieft",
            ],
        ),
        (
            "pl",
            &[
                "ten",
                "ta",
                "to",
                "te",
                "ci",
                "te",
                "jeden",
                "jedna",
                "z",
                "do",
                "w",
                "na",
                "pod",
                "przez",
                "bez",
                "kot",
                "dom",
                "tysiąc podziękowań",
                "stokrotne dzięki",
                "proszę",
            ],
        ),
        (
            "ru",
            &[
                "этот",
                "эта",
                "это",
                "эти",
                "один",
                "одна",
                "из",
                "к",
                "в",
                "с",
                "на",
                "для",
                "без",
                "между",
                "через",
                "кот",
                "дом",
                "тысяча благодарностей",
                "спасибо тысячу раз",
                "пожалуйста",
            ],
        ),
        (
            "tr",
            &[
                "bu",
                "şu",
                "o",
                "bir",
                "ve",
                "ile",
                "için",
                "den",
                "dan",
                "de",
                "da",
                "üzerinde",
                "altında",
                "arasında",
                "gibi",
                "kedi",
                "ev",
                "bin teşekkür",
                "binlerce teşekkür",
                "lütfen",
            ],
        ),
        (
            "id",
            &[
                "ini",
                "itu",
                "sebuah",
                "satu",
                "dan",
                "dengan",
                "untuk",
                "dari",
                "di",
                "pada",
                "atas",
                "bawah",
                "antara",
                "seperti",
                "tanpa",
                "kucing",
                "rumah",
                "seribu terima kasih",
                "terima kasih banyak",
                "tolong",
            ],
        ),
    ];

    #[test]
    fn twenty_positive_cases_per_language() {
        assert_eq!(UNITS.len(), 11);
        for (language, unit) in UNITS {
            let formatters = formatters(language).expect("locale");
            let spellout = formatters.spellout.as_ref().expect("spellout data");
            for value in VALUES {
                let spelled = spellout.format_i64(value).expect("format");
                let text = format!("{spelled} {unit}");
                let found = candidates(&text, language).expect("scan");
                assert!(
                    found
                        .iter()
                        .any(|c| (c.value - value as f64).abs() < 1e-9 && c.start == 0),
                    "{language} {value} {text:?}: {found:?}"
                );
            }
        }
    }

    #[test]
    fn fractions_in_every_language() {
        for (language, phrase) in FRACTIONS {
            let found = candidates(phrase, language).expect("scan");
            assert!(
                found
                    .iter()
                    .any(|c| { c.entity_type == "FRACTION" && (c.value - 1.0 / 3.0).abs() < 1e-9 }),
                "{language} {phrase:?}: {found:?}"
            );
        }
    }

    #[test]
    fn approximators_in_every_language() {
        for ((language, approximator), (_, unit)) in APPROXIMATORS.iter().zip(UNITS) {
            let formatters = formatters(language).expect("locale");
            let spellout = formatters.spellout.as_ref().expect("spellout data");
            let spelled = spellout.format_i64(5000).expect("format");
            let text = format!("{approximator} {spelled} {unit}");
            let found = candidates(&text, language).expect("scan");
            assert!(
                found.iter().any(|c| {
                    (c.value - 5000.0).abs() < 1e-9 && c.start == 0 && c.end > approximator.len()
                }),
                "{language} {text:?}: {found:?}"
            );
        }
    }

    #[test]
    fn twenty_negative_cases_per_language() {
        assert_eq!(NEGATIVES.len(), 11);
        for (language, phrases) in NEGATIVES {
            for phrase in phrases.iter() {
                let found = candidates(phrase, language).expect("scan");
                assert!(found.is_empty(), "{language} {phrase:?}: {found:?}");
            }
        }
    }
}
