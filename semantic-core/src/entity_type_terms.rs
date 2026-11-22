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
    /// Web domains owned by a curated outlet, keyed by language. A domain is a
    /// grounded source surface even though it is not a proper name, so it is a
    /// separate table from the display names above.
    #[serde(default)]
    pub media_outlet_domains: BTreeMap<String, Vec<String>>,
}

/// A publication or news outlet. It is a named subject and an organization for
/// every downstream identity table, but it is deliberately NOT part of
/// [`ADDITIVE_TYPES`]: those match head nouns ("tiger", "tank") by suffix, and
/// outlet names are full proper names ("la Repubblica", "il Fatto
/// Quotidiano") that must match exactly or not at all.
pub const MEDIA_OUTLET: &str = "MEDIA_OUTLET";

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
    assert!(
        file.types.iter().any(|kind| kind == MEDIA_OUTLET),
        "the declared type list must include {MEDIA_OUTLET}"
    );
    assert_eq!(
        file.media_outlet_domains.len(),
        11,
        "media outlet domains must cover all 11 supported languages"
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
    let inventory = languages.get(base).or_else(|| languages.get("en"))?;
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

/// Resolves a citation source by its full outlet surface.
///
/// Matching is exact on the lowercased, punctuation-trimmed surface. Outlet
/// names overlap ordinary words ("repubblica", "stampa", "tempo"), so the
/// head-noun matcher above would turn prose into sources; requiring the whole
/// name keeps the precision the recipe needs to be trustworthy.
/// Resolves a web domain to its curated outlet. Unlike [`is_media_outlet`],
/// this does not require a proper noun: `ansa.it` and `fanpage.it` are domains
/// by shape, and the curated table decides whether they are sources. Prose
/// never matches because the whole label (dot included) must be in the table.
pub fn is_media_outlet_domain(text: &str, language: &str) -> bool {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let Some(domains) = terms()
        .media_outlet_domains
        .get(base)
        .or_else(|| terms().media_outlet_domains.get("en"))
    else {
        return false;
    };
    let lower = text
        .trim()
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '.')
        .to_lowercase();
    !lower.is_empty() && domains.iter().any(|domain| domain.to_lowercase() == lower)
}

pub fn is_media_outlet(text: &str, language: &str) -> bool {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let languages = &terms().languages;
    let Some(inventory) = languages.get(base).or_else(|| languages.get("en")) else {
        return false;
    };
    let lower = text
        .trim()
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != ' ' && ch != '\'')
        .to_lowercase();
    if lower.is_empty() {
        return false;
    }
    inventory
        .get(MEDIA_OUTLET)
        .is_some_and(|entries| entries.iter().any(|term| term.to_lowercase() == lower))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_additive_type_is_covered_in_every_language() {
        for language in [
            "de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr",
        ] {
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
            let outlets = inventory
                .get(MEDIA_OUTLET)
                .unwrap_or_else(|| panic!("{language}: missing {MEDIA_OUTLET}"));
            assert!(!outlets.is_empty(), "{language}: {MEDIA_OUTLET}");
            let domains = terms()
                .media_outlet_domains
                .get(language)
                .unwrap_or_else(|| panic!("{language}: missing outlet domains"));
            assert!(!domains.is_empty(), "{language}: outlet domains");
            assert!(
                domains.iter().all(|domain| domain.contains('.')),
                "{language}: a domain needs a dot"
            );
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
        assert_eq!(classify("планета", "ru"), Some("CELESTIAL"));
        assert_eq!(classify("democrats", "en"), Some("NORP"));
        assert_eq!(classify("gripe", "pt"), Some("DISEASE"));
        assert_eq!(classify("painting", "en"), Some("ARTWORK"));
        assert_eq!(classify("helikopter", "id"), Some("VEHICLE"));
        // Unrelated text never classifies.
        assert_eq!(classify("Aurora", "en"), None);
        assert_eq!(classify("ball", "en"), None);
    }

    #[test]
    fn media_outlets_match_only_the_full_name() {
        assert!(is_media_outlet("Il Fatto Quotidiano", "it"));
        assert!(is_media_outlet("la Repubblica", "it"));
        assert!(is_media_outlet("ANSA", "it"));
        assert!(is_media_outlet("The Guardian", "en"));
        assert!(is_media_outlet("Le Monde", "fr"));
        assert!(is_media_outlet("Hürriyet", "tr"));
        // The head-noun vocabulary must not leak into the source vocabulary:
        // a bare "Repubblica" is the state (ORG), not the paper.
        assert!(!is_media_outlet("Repubblica", "it"));
        assert!(!is_media_outlet("stampa", "it"));
        assert!(!is_media_outlet("il fatto", "it"));
        assert!(!is_media_outlet("the", "en"));
        // A term from another language is not a source in this one.
        assert!(!is_media_outlet("Le Monde", "it"));
    }

    #[test]
    fn outlet_domains_are_curated_not_inferred_from_prose() {
        for (language, domain) in [
            ("de", "spiegel.de"),
            ("en", "theguardian.com"),
            ("es", "elpais.com"),
            ("fr", "lemonde.fr"),
            ("id", "kompas.com"),
            ("it", "ansa.it"),
            ("nl", "nos.nl"),
            ("pl", "onet.pl"),
            ("pt", "publico.pt"),
            ("ru", "tass.ru"),
            ("tr", "aa.com.tr"),
        ] {
            assert!(
                is_media_outlet_domain(domain, language),
                "{language}: {domain}"
            );
            assert!(
                is_media_outlet_domain(&domain.to_uppercase(), language),
                "{language}: domains are case-insensitive"
            );
            // The prose head is not a domain: "ansa" is an outlet name, not a
            // web source, and an unlisted domain never qualifies by shape.
            let head = domain.split('.').next().unwrap();
            assert!(
                !is_media_outlet_domain(head, language),
                "{language}: {head}"
            );
            assert!(
                !is_media_outlet_domain("example.com", language),
                "{language}: arbitrary domains are not sources"
            );
        }
    }

    #[test]
    fn dropped_article_aliases_stay_unambiguous() {
        // Article-dropped forms whose head is distinctive enough to stay a
        // source ("Fatto Quotidiano", "New York Times"). Ambiguous heads keep
        // requiring the article ("Repubblica", "Stampa", "Zeit").
        assert!(is_media_outlet("Fatto Quotidiano", "it"));
        assert!(is_media_outlet("Sole 24 Ore", "it"));
        assert!(is_media_outlet("New York Times", "en"));
        assert!(is_media_outlet("Guardian", "en"));
        assert!(is_media_outlet("Figaro", "fr"));
        assert!(is_media_outlet("Telegraaf", "nl"));
        assert!(!is_media_outlet("Repubblica", "it"));
        assert!(!is_media_outlet("Stampa", "it"));
        assert!(!is_media_outlet("Zeit", "de"));
        assert!(!is_media_outlet("Monde", "fr"));
    }

    #[test]
    fn the_head_noun_classifier_never_returns_the_media_outlet_type() {
        assert_eq!(classify("ansa", "it"), None);
        assert_eq!(classify("la repubblica", "it"), None);
    }
}
