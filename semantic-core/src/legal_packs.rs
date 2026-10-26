//! Generated optional legal/domain packs.
//!
//! `lexicon/legal/<lang>.json` is a domain layer, not part of the core: it
//! carries the markers, entity cues and micro-summary templates for one
//! language. A language without a pack produces no legal facts and no micro
//! summaries — the core keeps emitting generic facts — instead of importing
//! another language's vocabulary. Adding a language is a data file plus a
//! regenerated table, never a Rust match arm.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::lexicon::LegalLexicon;

const INDEX: &str = include_str!("../../lexicon/legal/index.json");

#[derive(Debug, Deserialize)]
struct RawIndex {
    languages: Vec<String>,
}

fn packs() -> &'static HashMap<String, LegalLexicon> {
    static PACKS: OnceLock<HashMap<String, LegalLexicon>> = OnceLock::new();
    PACKS.get_or_init(|| {
        let index: RawIndex = serde_json::from_str(INDEX).unwrap_or(RawIndex {
            languages: Vec::new(),
        });
        let available: HashMap<&str, &str> = crate::generated_legal_packs::LEGAL_PACKS
            .iter()
            .copied()
            .collect();
        let mut out = HashMap::new();
        for language in index.languages {
            let Some(json) = available.get(language.as_str()) else {
                continue;
            };
            let Ok(pack) = serde_json::from_str::<LegalLexicon>(json) else {
                continue;
            };
            out.insert(language, pack);
        }
        out
    })
}

/// Resolves a BCP-47 tag (`it-IT`) to its base pack (`it`).
pub fn get(language: &str) -> Option<&'static LegalLexicon> {
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

/// Languages with a committed legal pack, for diagnostics and coverage tests.
pub fn supported_languages() -> Vec<&'static str> {
    let mut languages: Vec<&'static str> = packs().keys().map(String::as_str).collect();
    languages.sort_unstable();
    languages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italian_and_english_packs_load_with_their_own_markers() {
        let it = get("it-IT").expect("Italian legal pack");
        assert!(it
            .patterns
            .iter()
            .any(|pattern| pattern.markers.iter().any(|marker| marker == "condannato")));
        let en = get("en").expect("English legal pack");
        assert!(!en
            .patterns
            .iter()
            .any(|pattern| pattern.markers.iter().any(|marker| marker == "condannato")));
    }

    #[test]
    fn a_language_without_a_pack_has_no_legal_layer() {
        // The inventory grows by data entries (de now ships entity cues), but
        // a language with no committed pack still gets no legal layer at all.
        assert!(get("ko").is_none());
        assert!(get("zh").is_none());
    }
}
