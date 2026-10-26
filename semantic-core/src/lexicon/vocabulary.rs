//! Typed surface vocabularies: visual objects, gazetteer places, scale
//! words, location cues and the generated CLDR month table.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Per-language visual-object vocabulary (typed OBJECT candidates).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ObjectVocabulary {
    pub terms: BTreeMap<String, Vec<String>>,
}

/// One place from the GeoNames index. `aliases` are the lowercased primary
/// name, ASCII name and alternate names (Latin, Greek or Cyrillic script) that
/// the index can type as a LOCATION.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Place {
    pub name: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub population: u64,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// The gazetteer that types single surfaces without context (brands, cities,
/// works, known people). `locations` is the hand-curated seed; `places` is the
/// real GeoNames index (cities500: populated places with population >= 5000,
/// plus capitals, CC BY 4.0). The typing mechanism is the same for both.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EntityGazetteer {
    /// Surname particles per language ("de Vries", "van Dijk"): a lowercase
    /// connector that stays inside a person name when a capitalized token
    /// follows it.
    #[serde(default)]
    pub name_particles: BTreeMap<String, Vec<String>>,
    pub object_words: Vec<String>,
    /// Nouns that stay VISUAL_CONCEPT explicitly ("company").
    pub generic_visual_nouns: Vec<String>,
    pub company_suffixes: Vec<String>,
    /// Language-specific institution heads/tails that type a capitalized
    /// multiword name as an organization (e.g. "Prism Laboratory").
    #[serde(default)]
    pub organization_markers: BTreeMap<String, Vec<String>>,
    pub brands: Vec<String>,
    pub locations: Vec<String>,
    /// GeoNames places; empty when the index is disabled by an override.
    #[serde(default)]
    pub places: Vec<Place>,
    /// Common words per language that are also place aliases ("corte",
    /// "come", "casa"). They are excluded from gazetteer typing in that
    /// language so a common noun never becomes a LOCATION.
    #[serde(default)]
    pub place_ambiguities: BTreeMap<String, Vec<String>>,
    pub works: Vec<String>,
    pub persons: Vec<String>,
    pub product_names: Vec<String>,
    pub product_substrings: Vec<String>,
}

/// Scaled-quantity vocabulary: scale words ("2 Millionen"), the English
/// singular chain ("2 million"), measurement units and currencies.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScaleWords {
    pub words: Vec<String>,
    pub english_scale_words: Vec<String>,
    pub unit_words: Vec<String>,
    #[serde(default)]
    pub percentage_words: Vec<String>,
    pub currency_words: Vec<String>,
}

/// Contextual geographic cues (no gazetteer: syntax only).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocationCues {
    pub context_prepositions: Vec<String>,
    /// Prepositions that also mark human indirect objects (for example
    /// Italian/Spanish/Portuguese "a"). A place-name cue needs gazetteer
    /// evidence when one of these precedes a capitalized span.
    #[serde(default)]
    pub ambiguous_context_prepositions: Vec<String>,
    pub genitive_prepositions: Vec<String>,
    pub container_nouns: Vec<String>,
    pub generic_location_nouns: Vec<String>,
    /// Locative prepositions that ground a bare prepositional phrase
    /// ("stadio di Torino" → located_in).
    pub sentence_locative_prepositions: Vec<String>,
}

/// Month names generated from CLDR (`lexicon/months.json`, synced by
/// `lexicon/tools/sync_cldr_months.py`): the ten supported languages' wide
/// month names in both CLDR contexts, plus documented legacy aliases. The
/// array order is the matching tie-break order.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Months {
    pub months: Vec<MonthEntry>,
    pub legacy_aliases: Vec<MonthAlias>,
}

/// One generated month name with its calendar number (1-12).
#[derive(Debug, Clone, Deserialize)]
pub struct MonthEntry {
    pub name: String,
    pub month: u8,
    #[serde(default)]
    pub locales: Vec<String>,
    #[serde(default)]
    pub contexts: Vec<String>,
}

/// A historical spelling kept for compatibility (see the sync tool).
#[derive(Debug, Clone, Deserialize)]
pub struct MonthAlias {
    pub name: String,
    pub month: u8,
    #[serde(default)]
    pub reason: String,
}

impl Months {
    /// The (name, calendar month) table in generated tie-break order, for
    /// date-span scanning.
    pub fn date_scan_table(&self) -> Vec<(String, u8)> {
        self.months
            .iter()
            .map(|entry| (entry.name.clone(), entry.month))
            .collect()
    }

    /// The names in generated tie-break order, for whole-token entity typing.
    pub fn typing_names(&self) -> Vec<String> {
        self.months.iter().map(|entry| entry.name.clone()).collect()
    }
}
