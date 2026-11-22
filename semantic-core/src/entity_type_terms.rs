//! Data-driven vocabulary for the additive visual/documentary entity types.
//!
//! Terms live in `lexicon/entity_type_terms.json` for all 11 languages. The
//! classifier is lexical and conservative: it matches the whole lowercased
//! surface or the head/tail noun phrase, never a substring.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

pub const ENTITY_TYPE_TERMS_SCHEMA: &str = "entity_type_terms.v1";

#[derive(Debug, Deserialize)]
pub struct EntityTypeTermFile {
    pub schema: String,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub languages: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// The closed set this module can emit. Existing OBJECT/PRODUCT fallbacks stay
/// authoritative for terms already covered by the older vocabulary.
pub const ADDITIVE_TYPES: &[&str] = &[
    "ANIMAL",
    "FOOD",
    "VEHICLE",
    "LAW",
    "COURT_CASE",
    "NORP",
    "DISEASE",
    "TECHNOLOGY",
    "ARTWORK",
    "CELESTIAL",
];

fn parse() -> EntityTypeTermFile {
    let file: EntityTypeTermFile =
        serde_json::from_str(include_str!("../../lexicon/entity_type_terms.json"))
            .expect("embedded entity type term lexicon must be valid JSON");
    assert_eq!(
        file.schema, ENTITY_TYPE_TERMS_SCHEMA,
        "unsupported entity type term schema"
    );
    assert_eq!(
        file.languages.len(),
        11,
        "entity type terms must cover all 11 supported languages"
    );
    file
}

pub fn terms() -> &'static EntityTypeTermFile {
    static FILE: OnceLock<EntityTypeTermFile> = OnceLock::new();
    FILE.get_or_init(parse)
}

/// Resolves a grounded surface to an additive type, if the language vocabulary
/// contains it. Multi-word surfaces match the longest term first so
/// "court case" wins over "case".
pub fn classify(text: &str, language: &str) -> Option<&'static str> {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let languages = &terms().languages;
    let inventory = languages
        .get(base)
        .or_else(|| languages.get("en"))?;
    let lower = text
        .trim()
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != ' ')
        .to_lowercase();
    if lower.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &'static str)> = None;
    for (kind, entries) in inventory {
        if !ADDITIVE_TYPES.contains(&kind.as_str()) {
            continue;
        }
        for term in entries {
            let term = term.to_lowercase();
            let matches = lower == term
                || lower
                    .strip_suffix(&term)
                    .is_some_and(|prefix| prefix.ends_with(' '))
                || lower
                    .strip_prefix(&term)
                    .is_some_and(|suffix| suffix.starts_with(' '));
            if matches && best.is_none_or(|(length, _)| term.chars().count() > length) {
                best = Some((term.chars().count(), kind.as_str()));
            }
        }
    }
    best.map(|(_, kind)| kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_additive_type_is_covered_in_every_language() {
        for language in ["de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr"] {
            let inventory = terms()
                .languages
                .get(language)
                .unwrap_or_else(|| panic!("missing {language}"));
            for kind in ADDITIVE_TYPES {
                let entries = inventory
                    .get(*kind)
                    .unwrap_or_else(|| panic!("{language}: missing {kind}"));
                assert!(!entries.is_empty(), "{language}: {kind}");
            }
        }
    }

    #[test]
    fn lexical_classification_is_exact_and_language_scoped() {
        assert_eq!(classify("tiger", "en"), Some("ANIMAL"));
        assert_eq!(classify("Tiger", "de"), Some("ANIMAL"));
        assert_eq!(classify("court case", "en"), Some("COURT_CASE"));
        assert_eq!(classify("software", "it"), Some("TECHNOLOGY"));
        assert_eq!(classify("treaty", "en"), Some("LAW"));
        assert_eq!(classify("pizza", "it"), Some("FOOD"));
        assert_eq!(classify("planet", "ru"), Some("CELESTIAL"));
        assert_eq!(classify("democrats", "en"), Some("NORP"));
        assert_eq!(classify("influenza", "pt"), Some("DISEASE"));
        assert_eq!(classify("painting", "en"), Some("ARTWORK"));
        assert_eq!(classify("helicopter", "id"), Some("VEHICLE"));
        // Unrelated text never classifies.
        assert_eq!(classify("Aurora", "en"), None);
        assert_eq!(classify("ball", "en"), None);
    }
}
