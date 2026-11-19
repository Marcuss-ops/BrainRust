//! Language data for the ICU number-word scanner.
//!
//! CLDR RBNF already knows how to spell and parse cardinals in every language,
//! so cardinal vocabulary is not duplicated here. This table carries the
//! grammar and precision data CLDR does not expose in a parseable form:
//! conjunction words, additive fractions, denominator words, approximators and
//! the idioms that must never become a number ("mille grazie").
//!
//! Every entry is data. Adding a language is a JSON edit plus the pack gate in
//! `lexicon/tools/build_language_packs.py`; no language branch lives in Rust.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// One fraction surface. `denominator` is the divisor; an absent numerator
/// means the surface already carries one ("sepertiga" = 1/3). `additive`
/// marks half-like forms, which add a fraction of the preceding quantity
/// instead of dividing it ("due milioni e mezzo" = 2.5 million).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct FractionSpec {
    pub denominator: u32,
    #[serde(default)]
    pub additive: bool,
}

/// Grammar data for one language.
#[derive(Debug, Clone, Deserialize)]
pub struct NumberWordRules {
    #[serde(default)]
    pub conjunctions: Vec<String>,
    #[serde(default)]
    pub approximators: Vec<String>,
    #[serde(default)]
    pub fraction_articles: Vec<String>,
    #[serde(default)]
    pub fractions: BTreeMap<String, FractionSpec>,
    /// French numbers multiply the unit by the ten (`quatre-vingt` = 4 × 20).
    #[serde(default)]
    pub multiplicative_tens: bool,
    /// Fused half suffixes: German `zweieinhalb` = 2.5, Dutch `tweeënhalf`.
    #[serde(default)]
    pub half_suffixes: Vec<String>,
    /// Explicit inflected forms CLDR does not parse (`две` = 2, feminine).
    #[serde(default)]
    pub variants: BTreeMap<String, f64>,
    /// Multi-word fraction phrases (Turkish `üçte biri` = 1/3).
    #[serde(default)]
    pub fraction_phrases: BTreeMap<String, FractionSpec>,
    #[serde(default)]
    pub negatives: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NumberWordsFile {
    languages: BTreeMap<String, NumberWordRules>,
}

fn file() -> &'static BTreeMap<String, NumberWordRules> {
    static FILE: OnceLock<BTreeMap<String, NumberWordRules>> = OnceLock::new();
    FILE.get_or_init(|| {
        let parsed: NumberWordsFile =
            serde_json::from_str(include_str!("../../lexicon/number_words.json"))
                .expect("embedded number_words.json must be valid JSON");
        parsed.languages
    })
}

/// Rules for `language` (`it-IT` falls back to `it`), or `None` when the
/// language has no data. The caller fails closed: no rules, no word numbers.
pub fn rules(language: &str) -> Option<&'static NumberWordRules> {
    let lower = language.trim().to_ascii_lowercase();
    if let Some(rules) = file().get(&lower) {
        return Some(rules);
    }
    let base = lower.split(['-', '_']).next().unwrap_or(&lower);
    file().get(base)
}

/// True when the normalized phrase is exactly a listed idiom. Matching is on
/// word boundaries, so "mille grazie" does not block "mille persone".
pub fn is_negative_phrase(text: &str, language: &str) -> bool {
    let Some(rules) = rules(language) else {
        return false;
    };
    let normalized = normalize(text);
    rules
        .negatives
        .iter()
        .any(|negative| normalized == normalize(negative))
}

/// True when a listed idiom occurs as a whole-word subsequence anywhere in
/// `text`. The scanner uses it on the sentence window around a candidate, so
/// "Mille grazie" and "grazie mille" are both rejected without guessing
/// which side of the number the idiom starts on.
pub fn contains_negative_phrase(text: &str, language: &str) -> bool {
    let Some(rules) = rules(language) else {
        return false;
    };
    let words: Vec<String> = normalize(text)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    rules.negatives.iter().any(|negative| {
        let needle: Vec<String> = normalize(negative)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        !needle.is_empty()
            && words
                .windows(needle.len())
                .any(|window| window == needle.as_slice())
    })
}

/// Normalization shared with the scanner: lowercase, punctuation folded to a
/// single space, so data entries need no byte-level spelling.
fn normalize(text: &str) -> String {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pack_language_has_rules() {
        for language in crate::language_packs::supported_languages() {
            assert!(
                rules(language).is_some(),
                "missing number_words rules for {language}"
            );
        }
    }

    #[test]
    fn region_tags_fall_back_to_the_base_language() {
        assert!(rules("it-IT").is_some());
        assert!(rules("pt_BR").is_some());
        assert!(rules("xx").is_none());
    }

    #[test]
    fn fractions_and_conjunctions_are_data() {
        let it = rules("it").expect("it rules");
        assert!(it.conjunctions.iter().any(|word| word == "e"));
        let terzo = it.fractions.get("terzo").expect("terzo");
        assert_eq!(terzo.denominator, 3);
        assert!(!terzo.additive);
        assert!(it.fractions["mezzo"].additive);
        assert!(!it.multiplicative_tens);
        assert!(rules("fr").expect("fr rules").multiplicative_tens);
    }

    #[test]
    fn idioms_do_not_become_numbers() {
        assert!(is_negative_phrase("Mille grazie.", "it"));
        assert!(!is_negative_phrase("Mille persone.", "it"));
        assert!(contains_negative_phrase("Beh, mille grazie davvero.", "it"));
        assert!(contains_negative_phrase("Grazie mille!", "it"));
        assert!(!contains_negative_phrase("Mille persone in piazza.", "it"));
    }
}
