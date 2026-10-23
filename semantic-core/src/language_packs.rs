//! Generated language packs.
//!
//! The Rust extractor never carries a per-language word list. The build script
//! `lexicon/tools/build_language_packs.py` turns `lexicon/languages/<lang>`
//! into one JSON pack per language, and this module exposes the fused
//! closed-class index. A new language is a data directory plus a regenerated
//! table: no match arm, no domain code.
//!
//! Two inventories stay separate on purpose:
//!   * `closed_class` breaks a candidate phrase (articles, adpositions,
//!     pronouns, conjunctions, particles, auxiliaries);
//!   * `function_words` is the wider Go sentence-start list, which also
//!     contains open-class words ("great") that belong inside proper names.
//!
//! Using the second as a Rust exclusion list would cut "Great Smoky Mountains"
//! in half, so the extractor only ever consults the first.
//!
//! The pack is the fallback only. When a UDPipe model is installed, POS tags
//! decide function words first; the pack keeps the extractor deterministic on
//! hosts and languages without a model.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;

const INDEX: &str = include_str!("../../lexicon/packs/index.json");

#[derive(Debug, Deserialize)]
struct RawPack {
    language: String,
    #[serde(default)]
    closed_class: Vec<String>,
    #[serde(default)]
    function_words: Vec<String>,
    #[serde(default)]
    stopwords: Vec<String>,
}

/// One language's generated inventories.
#[derive(Debug)]
pub struct LanguagePack {
    pub language: String,
    pub closed_class: Vec<String>,
    pub function_words: Vec<String>,
    pub stopwords: Vec<String>,
    function: HashSet<String>,
    closed: HashSet<String>,
}

impl LanguagePack {
    /// True when the lowercase surface must break a candidate phrase.
    pub fn is_stopword(&self, lower: &str) -> bool {
        self.closed.contains(lower)
    }

    /// True when the surface belongs to either generated inventory. Kept for
    /// diagnostics and tests; candidate filtering uses [`Self::is_stopword`].
    pub fn is_function_word(&self, lower: &str) -> bool {
        self.function.contains(lower)
    }
}

#[derive(Debug, Deserialize)]
struct RawIndex {
    languages: Vec<String>,
}

fn packs() -> &'static HashMap<String, LanguagePack> {
    static PACKS: OnceLock<HashMap<String, LanguagePack>> = OnceLock::new();
    PACKS.get_or_init(|| {
        let index: RawIndex = serde_json::from_str(INDEX).unwrap_or(RawIndex {
            languages: Vec::new(),
        });
        let available: HashMap<&str, &str> = crate::generated_language_packs::PACKS
            .iter()
            .copied()
            .collect();
        let mut out = HashMap::new();
        for language in index.languages {
            let Some(json) = available.get(language.as_str()) else {
                continue;
            };
            let Ok(raw) = serde_json::from_str::<RawPack>(json) else {
                continue;
            };
            let mut closed: HashSet<String> = raw.closed_class.iter().cloned().collect();
            // A pack generated before `closed_class` existed must still filter:
            // fall back to the union of the two older inventories.
            if closed.is_empty() {
                closed.extend(raw.stopwords.iter().cloned());
                closed.extend(raw.function_words.iter().cloned());
            }
            let mut function: HashSet<String> = raw.function_words.iter().cloned().collect();
            function.extend(raw.closed_class.iter().cloned());
            out.insert(
                language.clone(),
                LanguagePack {
                    language: raw.language,
                    closed_class: raw.closed_class,
                    function_words: raw.function_words,
                    stopwords: raw.stopwords,
                    function,
                    closed,
                },
            );
        }
        out
    })
}

/// Resolves a BCP-47 tag (`it-IT`) to its base pack (`it`).
pub fn get(language: &str) -> Option<&'static LanguagePack> {
    let key = language.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    let base = key
        .split(['-', '_'])
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(&key);
    packs().get(&key).or_else(|| packs().get(base))
}

/// Languages with a committed pack, for diagnostics and coverage tests.
pub fn supported_languages() -> Vec<&'static str> {
    let mut languages: Vec<&'static str> = packs().keys().map(String::as_str).collect();
    languages.sort_unstable();
    languages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italian_pack_contains_function_words_and_resolves_regional_tags() {
        let pack = get("it-IT").expect("Italian pack");
        assert!(pack.is_stopword("questo"));
        assert!(pack.is_stopword("cui"));
        assert!(pack.is_stopword("non"));
        assert!(!pack.is_stopword("tifosi"));
    }

    #[test]
    fn open_class_words_from_the_go_filter_are_not_stopwords() {
        // "great" belongs to "Great Smoky Mountains", not to the closed class.
        let pack = get("en").expect("English pack");
        assert!(pack.is_function_word("great"));
        assert!(!pack.is_stopword("great"));
        assert!(pack.is_stopword("the"));
    }

    #[test]
    fn every_committed_language_has_a_pack() {
        for language in ["de", "en", "es", "fr", "id", "it", "pl", "pt", "ru", "tr"] {
            assert!(get(language).is_some(), "missing pack for {language}");
        }
    }
}
