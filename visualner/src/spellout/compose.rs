//! Number-word composition and parsing helpers.

use rust_icu_sys::UNumberFormatStyle;
use rust_icu_uloc::ULoc;
use rust_icu_unum::UNumberFormat;
use semantic_core::number_words::NumberWordRules;

use super::quantity;
use super::tokens::{word_tokens, WordToken};
use super::WordNumberCandidate;

/// Original surface plus a conjunction-free variant. CLDR spells Italian
/// 1200 as the fused "milleduecento" while scripts also write "mille e
/// duecento"; removing the language conjunction is the only difference. Both
/// variants keep the original byte span for grounding.
pub(super) fn parse_variants(surface: &str, rules: Option<&NumberWordRules>) -> Vec<String> {
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
pub(super) fn compose_run(
    surface: &str,
    rules: Option<&NumberWordRules>,
    formatter: &UNumberFormat,
    vocabulary: &super::NumberVocabulary,
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
        // Ordinary prose words are not number words. Asking ICU to parse
        // each of them makes the recovery pass pay one failed RBNF parse per
        // word for every window it sees; the vocabulary already knows every
        // spelling the locale's formatter can produce, so it can reject them
        // for free. A word outside the vocabulary cannot compose a number
        // that the direct parse would have missed.
        if !vocabulary.words.contains(&normalize_spellout(&word)) {
            return None;
        }
        let value = parse_complete(&word, formatter, vocabulary)?;
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
pub(super) fn extend_scale_word(source: &str, candidate_end: usize, value: f64) -> (usize, f64) {
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
pub(super) fn extend_additive_fraction(
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
    let scale_before = quantity::suffix_scale_factor(source, number_end, candidate_end)
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
pub(super) fn parse_half_compound(
    surface: &str,
    rules: Option<&NumberWordRules>,
    formatter: &UNumberFormat,
    vocabulary: &super::NumberVocabulary,
) -> Option<f64> {
    let rules = rules?;
    let word = surface.trim().to_lowercase();
    rules.half_suffixes.iter().find_map(|suffix| {
        let prefix = word.strip_suffix(&suffix.to_lowercase())?.trim();
        if prefix.is_empty() {
            return None;
        }
        parse_complete(prefix, formatter, vocabulary).map(|base| base + 0.5)
    })
}

/// Denominator fractions are their own fact ("un terzo" = 1/3). The numerator
/// is the article or the preceding number word; without either it is 1.
pub(super) fn denominator_fractions(
    source: &str,
    tokens: &[WordToken],
    rules: Option<&NumberWordRules>,
    formatter: Option<&UNumberFormat>,
    vocabulary: &super::NumberVocabulary,
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
        // "il terzo 90" is an ordinal list label, not one third. A numeric
        // token immediately after the fraction word is the value it labels.
        if let Some(next) = tokens.get(index + 1) {
            let next_word = source[next.start..next.end].to_lowercase();
            if next_word
                .chars()
                .any(|character| character.is_ascii_digit())
                || parse_complete(&next_word, formatter, vocabulary).is_some()
            {
                continue;
            }
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
            } else if let Some(value) = parse_complete(&previous_word, formatter, vocabulary) {
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
pub(super) fn approximator_start(
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
pub(super) fn compose_thousands(
    surface: &str,
    formatter: &UNumberFormat,
    vocabulary: &super::NumberVocabulary,
) -> Option<f64> {
    let normalized = normalize_spellout(surface);
    for (prefix, amount) in &vocabulary.thousands_spellings {
        if prefix.is_empty() || !normalized.starts_with(prefix.as_str()) {
            continue;
        }
        let remainder = &normalized[prefix.len()..];
        if remainder.is_empty() {
            return Some(*amount as f64);
        }
        let Some(rest) = parse_complete(remainder, formatter, vocabulary) else {
            continue;
        };
        if (0.0..1000.0).contains(&rest) {
            return Some(*amount as f64 + rest);
        }
    }
    None
}

/// Complete parse with the same canonical round-trip rule used by the main
/// loop, including the compound-prefix recovery for "venticinque" (20 + 5).
fn parse_complete(
    surface: &str,
    formatter: &UNumberFormat,
    vocabulary: &super::NumberVocabulary,
) -> Option<f64> {
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
    vocabulary
        .spellings
        .get(&normalized)
        .filter(|value| **value <= 999)
        .map(|value| *value as f64)
}

pub(super) fn ordinal_equivalent(surface: &str, canonical: &str) -> bool {
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

pub(super) fn format_ordinal(number: i64, locale: &ULoc) -> Option<String> {
    let formatter =
        UNumberFormat::try_new_with_style(UNumberFormatStyle::UNUM_ORDINAL, locale).ok()?;
    formatter.format_i64(number).ok()
}

pub(super) fn normalize_spellout(value: &str) -> String {
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

pub(super) fn crosses_hard_punctuation(separator: &str) -> bool {
    separator.chars().any(|character| {
        matches!(
            character,
            '.' | '!' | '?' | ';' | ':' | '。' | '！' | '？' | '؛'
        )
    })
}
