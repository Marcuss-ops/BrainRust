//! Canonical numeric-value parsing shared by every semantic extractor.
//!
//! Before this module, three code paths read numbers independently:
//! `visualner::analysis` (`numeric_facts`, `chart_points`) and
//! `pipelinegen_muscles::phrase_impact` (`extract_chart_values`). This module
//! is the single source of truth for turning a numeric *surface* into a typed
//! value. Every collector keeps its own span detection (which is
//! tokenizer-specific), but the parse and the typed projection live here so a
//! chart can never plot a value one parser accepted and another refused.

use serde::Serialize;

/// One grounded numeric-value fact. `value` is parsed only when the surface
/// is unambiguous for the scene language (`25%`, `$1,200`, `12,5% euro`); a
/// scaled quantity such as `2 million` carries no raw value instead of a
/// guessed one. The killer rule is inherited: surface == evidence == source
/// slice at `[start_byte, end_byte)`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NumericFact {
    /// Exact source form (e.g. "25%").
    pub surface: String,
    /// One of `NUMBER`, `PERCENT`, `MONEY`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Parsed numeric value when unambiguous; `None` otherwise.
    pub value: Option<f64>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub evidence: String,
}

/// Parses a numeric surface such as `25%`, `$1,200`, `1.200 euro`, `12,5%`,
/// `3.5` or `25 percent`, using the separator convention in
/// `language_profiles.json`: `thousands_separator` and `decimal_separator` of
/// the language, falling back to the `default` entry. Returns `None` for
/// scale words (`million`) and for any grouping that does not match the
/// convention, so a chart never plots a guessed value.
pub fn parse_unambiguous_value(text: &str, language: &str) -> Option<f64> {
    let mut rest = text.trim().to_lowercase();
    let mut suffixes: Vec<&str> = crate::lexicon::lexicon()
        .scale_words
        .percentage_words
        .iter()
        .map(String::as_str)
        .chain(["dollars", "euros", "euro", "pounds", "yen"])
        .collect();
    suffixes.sort_by_key(|suffix| std::cmp::Reverse(suffix.len()));
    for suffix in suffixes {
        if let Some(stripped) = rest.strip_suffix(suffix) {
            rest = stripped.trim_end().to_string();
            break;
        }
    }
    let rest =
        rest.trim_matches(|c: char| matches!(c, '%' | '$' | '£' | '€' | '¥') || c.is_whitespace());
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return None;
    }
    let profiles = &crate::lexicon::lexicon().language_profiles;
    let thousands = profiles.thousands_separator(language);
    let decimal = profiles.decimal_separator(language);
    let (int_part, frac_part) = match rest.matches(decimal).count() {
        0 => (rest, None),
        1 => {
            let (int_part, frac) = rest.split_once(decimal)?;
            (int_part, Some(frac))
        }
        _ => return None,
    };
    if matches!(frac_part, Some(frac) if frac.is_empty() || frac.contains(thousands)) {
        return None;
    }
    // A lone decimal-comma followed by exactly three digits ("1,200" in an
    // Italian sentence) reads as 1.2 by convention, but is the usual way an
    // English thousands group is written, so it is refused rather than guessed.
    if decimal == ',' && !rest.contains(thousands) && frac_part.is_some_and(|frac| frac.len() == 3)
    {
        return None;
    }
    let int_digits = if int_part.contains(thousands) {
        let groups: Vec<&str> = int_part.split(thousands).collect();
        let (first, rest_groups) = groups.split_first()?;
        let valid =
            (1..=3).contains(&first.len()) && rest_groups.iter().all(|group| group.len() == 3);
        if !valid {
            return None;
        }
        groups.concat()
    } else {
        int_part.to_string()
    };
    if int_digits.is_empty() {
        return None;
    }
    let number = match frac_part {
        Some(frac) => format!("{int_digits}.{frac}"),
        None => int_digits,
    };
    number.parse::<f64>().ok()
}

/// Two numeric facts are *comparable* (safe to place in one chart) only when
/// both carry an unambiguous value and the same kind (PERCENT with PERCENT,
/// MONEY with MONEY). A date is never comparable to a percentage, and a
/// scaled quantity without a parse is never comparable at all. This is the
/// gate a chart resolver must pass before proposing `bar_comparison`.
pub fn values_are_comparable(a: &NumericFact, b: &NumericFact) -> bool {
    a.kind == b.kind && a.value.is_some() && b.value.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_come_from_the_language_profile() {
        // Grouping and decimals per language, read from data.
        assert_eq!(parse_unambiguous_value("1,200.50", "en"), Some(1200.5));
        assert_eq!(parse_unambiguous_value("1.200,50", "it"), Some(1200.5));
        assert_eq!(parse_unambiguous_value("1.200,50", "de"), Some(1200.5));
        assert_eq!(parse_unambiguous_value("25%", "it"), Some(25.0));
        // A language without its own entry falls back to the default entry.
        assert_eq!(parse_unambiguous_value("1.200,50", "xx"), Some(1200.5));
        // The refused ambiguous form stays refused: "1,200" in Italian would
        // be 1.2 by convention and 1,200 by English grouping, so it is None.
        assert_eq!(parse_unambiguous_value("1,200", "it"), None);
    }
}
