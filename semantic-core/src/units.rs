//! Data-driven unit vocabulary for typed quantities.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

pub const UNITS_SCHEMA: &str = "units.v1";

#[derive(Debug, Deserialize)]
pub struct UnitsLexicon {
    pub schema: String,
    pub dimensions: Vec<String>,
    pub languages: BTreeMap<String, LanguageUnits>,
}

#[derive(Debug, Deserialize)]
pub struct LanguageUnits {
    pub units: Vec<UnitDefinition>,
    #[serde(default)]
    pub qualifiers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub predicates: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnitDefinition {
    pub surfaces: Vec<String>,
    pub dimension: String,
    pub code: String,
    pub factor_to_base: f64,
}

pub fn units_lexicon() -> &'static UnitsLexicon {
    static LEXICON: OnceLock<UnitsLexicon> = OnceLock::new();
    LEXICON.get_or_init(|| {
        let lexicon: UnitsLexicon = serde_json::from_str(include_str!("../../lexicon/units.json"))
            .expect("embedded units lexicon must be valid JSON");
        assert_eq!(
            lexicon.schema, UNITS_SCHEMA,
            "unsupported units lexicon schema"
        );
        lexicon
    })
}

pub fn language_units(language: &str) -> Option<&'static LanguageUnits> {
    static LANGUAGES: OnceLock<BTreeMap<String, LanguageUnits>> = OnceLock::new();
    LANGUAGES
        .get_or_init(|| {
            crate::generated_units::UNITS
                .iter()
                .map(|(language, body)| {
                    let row: LanguageUnits =
                        serde_json::from_str(body).expect("generated unit pack must be valid JSON");
                    ((*language).to_string(), row)
                })
                .collect()
        })
        .get(language)
}

pub fn find_unit(language: &str, surface: &str) -> Option<&'static UnitDefinition> {
    let surface = surface.trim().to_lowercase();
    language_units(language)?.units.iter().find(|unit| {
        unit.surfaces
            .iter()
            .any(|candidate| candidate.to_lowercase() == surface)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_table_is_well_formed_and_covers_italian_and_english() {
        let lexicon = units_lexicon();
        assert_eq!(lexicon.schema, UNITS_SCHEMA);
        assert!(!lexicon.dimensions.is_empty());
        assert_eq!(lexicon.languages.len(), 11);
        for language in lexicon.languages.keys() {
            let units = language_units(language).expect("seed language");
            assert!(!units.units.is_empty());
            for unit in &units.units {
                assert!(lexicon.dimensions.contains(&unit.dimension));
                assert!(!unit.code.is_empty());
                assert!(unit.factor_to_base.is_finite() && unit.factor_to_base > 0.0);
                assert!(!unit.surfaces.is_empty());
            }
        }
    }

    #[test]
    fn exact_unit_surfaces_resolve_without_cross_language_fallback() {
        assert_eq!(
            find_unit("it", "EURO").map(|unit| unit.code.as_str()),
            Some("EUR")
        );
        assert_eq!(
            find_unit("en", "km").map(|unit| unit.dimension.as_str()),
            Some("length")
        );
        assert_eq!(
            find_unit("fr", "kilomètres").map(|unit| unit.code.as_str()),
            Some("km")
        );
        assert_eq!(
            find_unit("ru", "километров").map(|unit| unit.code.as_str()),
            Some("km")
        );
        assert!(find_unit("ru", "euro").is_none());
    }
}
