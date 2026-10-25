//! Orthographic knowledge used by sentence splitting and date recognition:
//! abbreviation sets and era markers, per language.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Sentence-boundary abbreviations: a token ending in one of these terminates
/// a clause, not the sentence. `common` applies to every language; a language
/// adds its own forms through `by_language`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Abbreviations {
    #[serde(default)]
    pub common: Vec<String>,
    /// Company suffixes (`Inc.`) that only suppress the boundary before a
    /// lowercase continuation ("Acme Inc. and ..." stays one sentence).
    #[serde(default)]
    pub corporate: Vec<String>,
    #[serde(default)]
    pub by_language: BTreeMap<String, Vec<String>>,
}

impl Abbreviations {
    /// Every abbreviation that suppresses a sentence boundary in `language`.
    pub fn for_language(&self, language: &str) -> impl Iterator<Item = &str> {
        let key = language.split(['-', '_']).next().unwrap_or("");
        let localized = self.by_language.get(key).map(Vec::as_slice).unwrap_or(&[]);
        self.common
            .iter()
            .chain(localized.iter())
            .map(String::as_str)
    }
}

/// One era marker and the era it denotes (`bce`/`ce`).
#[derive(Debug, Clone, Deserialize)]
pub struct EraMarker {
    pub marker: String,
    pub era: String,
}

/// Orthographic knowledge used by sentence splitting and date recognition:
/// abbreviation sets and era markers. Kept in data so a new language's forms
/// never require a code change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Orthography {
    #[serde(default)]
    pub abbreviations: Abbreviations,
    #[serde(default)]
    pub era_markers: Vec<EraMarker>,
}
