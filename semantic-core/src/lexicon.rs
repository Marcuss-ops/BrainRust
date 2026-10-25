//! Versioned linguistic knowledge, kept OUT of Rust code.
//!
//! Every curated vocabulary this workspace used to hard-code in Rust
//! (stopwords, boilerplate phrases, object vocabularies, location cues, scale
//! words, legal markers, discourse cues, process phases) lives in JSON tables
//! under `lexicon/`. Rust owns the
//! mechanism (tokenization, spans, scoring, grounding); the lexicon owns the
//! knowledge. Adding or correcting a word is a data change, never a code
//! change, so a new language or editorial domain scales without recompiling.
//!
//! Loading is deterministic: the embedded JSON is the built-in default and
//! the same input always produces the same analysis. Setting
//! `SEMANTIC_LEXICON_DIR` to a directory containing any of the table files
//! replaces those tables wholesale (never merged) so a deployment can extend
//! or trim the vocabulary without a rebuild; a malformed override is ignored
//! with a warning and the embedded default keeps running.
//!
//! The tables are versioned (`LEXICON_VERSION`) and recorded in
//! [`crate::AnalyzerVersions`], so a vocabulary change can never serve a
//! stale cached analysis.
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Version of the lexicon tables. Bump whenever a table changes semantics.
pub const LEXICON_VERSION: &str = "lexicon.v4";

/// Environment variable naming a directory of overriding table files.
pub const LEXICON_DIR_ENV: &str = "SEMANTIC_LEXICON_DIR";

// ── Table shapes ────────────────────────────────────────────────────

mod cues;
mod legal;
mod orthography;
mod profiles;
mod vocabulary;

pub use cues::{
    DimensionCue, DiscourseCues, EntityRelationMarker, FacilityLexicon, NumberWord, OperatorCue,
    ProcessPhase, ProcessPhases, RelationCues, RelationMarkerGroup,
};
pub use legal::{
    EntityCue, LegalComparison, LegalLexicon, LegalPattern, MicroLexicon, SubjectKind, ValueKind,
};
pub use orthography::{Abbreviations, EraMarker, Orthography};
pub use profiles::{DateOrder, LanguageProfile, LanguageProfiles, ScriptProfile, Stopwords};
pub use vocabulary::{
    EntityGazetteer, LocationCues, MonthAlias, MonthEntry, Months, ObjectVocabulary, Place,
    ScaleWords,
};

/// The complete loaded lexicon.
#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    pub stopwords: Stopwords,
    pub objects: ObjectVocabulary,
    pub orthography: Orthography,
    pub language_profiles: LanguageProfiles,
    pub gazetteer: EntityGazetteer,
    /// Lowercased `gazetteer.locations` and place names/aliases, built once.
    pub place_index: std::collections::HashSet<String>,
    /// Lowercased alias-only names of places below the minor threshold. They
    /// stay out of LOCATION typing: a village alias is not stronger evidence
    /// than the capitalized-token name rule ("João Ribeiro" is a person).
    pub minor_place_aliases: std::collections::HashSet<String>,
    /// Lowercased ambiguous aliases per language, built once.
    pub place_ambiguities: BTreeMap<String, std::collections::HashSet<String>>,
    pub scale_words: ScaleWords,
    pub location_cues: LocationCues,
    pub process_phases: ProcessPhases,
    pub facility: FacilityLexicon,
    pub discourse: DiscourseCues,
    pub relation: RelationCues,
    /// Generated CLDR month table (`lexicon/months.json`).
    pub months: Months,
    /// `months` as (name, calendar month) in tie-break order.
    pub month_date_scan_table: Vec<(String, u8)>,
    /// `months` as names, for whole-token entity typing.
    pub month_typing_names: Vec<String>,
}

/// Loads one table: `$SEMANTIC_LEXICON_DIR/<file>` when present and valid,
/// otherwise the embedded default. A malformed override warns and falls back
/// so a broken deployment can never crash or silently change an analysis.
fn load_table<T: serde::de::DeserializeOwned>(file: &str, embedded: &'static str) -> T {
    if let Ok(dir) = std::env::var(LEXICON_DIR_ENV) {
        let path = std::path::Path::new(&dir).join(file);
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str(&text) {
                Ok(table) => return table,
                Err(error) => {
                    eprintln!("semantic-core lexicon: ignoring {path:?}: {error}");
                }
            }
        }
    }
    serde_json::from_str(embedded)
        .unwrap_or_else(|error| panic!("embedded lexicon table {file} is invalid: {error}"))
}

/// Builds the per-language ambiguity lookup. A word listed for a language is
/// never typed as a place in that language, even though the index contains it.
fn build_place_ambiguities(
    gazetteer: &EntityGazetteer,
) -> BTreeMap<String, std::collections::HashSet<String>> {
    gazetteer
        .place_ambiguities
        .iter()
        .map(|(language, words)| {
            (
                language.clone(),
                words.iter().map(|word| word.to_lowercase()).collect(),
            )
        })
        .collect()
}

/// Builds the O(1) place lookup from the curated seeds and the GeoNames
/// index. Aliases are already lowercased by the generator; the extra
/// conversion keeps a hand-written override safe.
/// Places below this population are only recognized through their primary
/// name or the curated seed, never through a bare alias: small-town aliases
/// collide with person names ("João Ribeiro").
const MINOR_PLACE_ALIAS_POPULATION: u64 = 50_000;

fn build_place_index(
    gazetteer: &EntityGazetteer,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let mut index = std::collections::HashSet::with_capacity(
        gazetteer.locations.len() + gazetteer.places.len() * 6,
    );
    let mut primary = std::collections::HashSet::new();
    let mut major_aliases = std::collections::HashSet::new();
    let mut minor_aliases = std::collections::HashSet::new();
    for location in &gazetteer.locations {
        let lower = location.to_lowercase();
        index.insert(lower.clone());
        primary.insert(lower);
    }
    for place in &gazetteer.places {
        let name = place.name.to_lowercase();
        index.insert(name.clone());
        primary.insert(name);
        for alias in &place.aliases {
            let alias = alias.to_lowercase();
            index.insert(alias.clone());
            if place.population < MINOR_PLACE_ALIAS_POPULATION {
                minor_aliases.insert(alias);
            } else {
                major_aliases.insert(alias);
            }
        }
    }
    // A name that any place uses as its primary name, or an alias shared with
    // a major place, is not a minor-only alias.
    minor_aliases.retain(|alias| !primary.contains(alias) && !major_aliases.contains(alias));
    (index, minor_aliases)
}

impl Lexicon {
    /// Builds the lexicon once: embedded defaults with any runtime override.
    pub fn global() -> &'static Lexicon {
        static LEXICON: OnceLock<Lexicon> = OnceLock::new();
        LEXICON.get_or_init(|| {
            let gazetteer: EntityGazetteer = load_table(
                "entity_gazetteer.json",
                include_str!("../../lexicon/entity_gazetteer.json"),
            );
            let (place_index, minor_place_aliases) = build_place_index(&gazetteer);
            let place_ambiguities = build_place_ambiguities(&gazetteer);
            let months: Months =
                load_table("months.json", include_str!("../../lexicon/months.json"));
            let month_date_scan_table = months.date_scan_table();
            let month_typing_names = months.typing_names();
            Lexicon {
                stopwords: load_table(
                    "stopwords.json",
                    include_str!("../../lexicon/stopwords.json"),
                ),
                objects: load_table(
                    "object_vocabulary.json",
                    include_str!("../../lexicon/object_vocabulary.json"),
                ),
                orthography: load_table(
                    "orthography.json",
                    include_str!("../../lexicon/orthography.json"),
                ),
                language_profiles: load_table(
                    "language_profiles.json",
                    include_str!("../../lexicon/language_profiles.json"),
                ),
                gazetteer,
                place_index,
                minor_place_aliases,
                place_ambiguities,
                scale_words: load_table(
                    "scale_words.json",
                    include_str!("../../lexicon/scale_words.json"),
                ),
                location_cues: load_table(
                    "location_cues.json",
                    include_str!("../../lexicon/location_cues.json"),
                ),
                process_phases: load_table(
                    "process_phases.json",
                    include_str!("../../lexicon/process_phases.json"),
                ),
                facility: load_table(
                    "facility_facts.json",
                    include_str!("../../lexicon/facility_facts.json"),
                ),
                discourse: load_table(
                    "discourse_cues.json",
                    include_str!("../../lexicon/discourse_cues.json"),
                ),
                relation: load_table(
                    "relation_cues.json",
                    include_str!("../../lexicon/relation_cues.json"),
                ),
                months,
                month_date_scan_table,
                month_typing_names,
            }
        })
    }

    /// True when a capitalized surface is a known place. The capitalization
    /// guard keeps lowercase common nouns that collide with a GeoNames alias
    /// ("casa" -> Chas, "mare" -> Male) out of the LOCATION typing.
    pub fn is_place(&self, surface: &str, language: &str) -> bool {
        // The capitalization guard only means something in a cased script.
        // Han/Japanese/Arabic/Thai have no uppercase initial, so requiring one
        // would make every proper name in those scripts invisible to the
        // index. German keeps the guard: it is cased, even though every noun
        // is capitalized.
        if self.language_profiles.is_cased(language)
            && !surface.chars().next().is_some_and(char::is_uppercase)
        {
            return false;
        }
        let lower = surface.to_lowercase();
        let key = language.split(['-', '_']).next().unwrap_or("");
        if self
            .place_ambiguities
            .get(key)
            .is_some_and(|ambiguous| ambiguous.contains(&lower))
        {
            return false;
        }
        if self.minor_place_aliases.contains(&lower) {
            return false;
        }
        self.place_index.contains(&lower)
    }

    /// Object terms of one BCP-47 language; empty for unsupported languages.
    pub fn object_terms(&self, language: &str) -> &[String] {
        let key = language.split(['-', '_']).next().unwrap_or("");
        self.objects
            .terms
            .get(key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// Convenience accessor for [`Lexicon::global`].
pub fn lexicon() -> &'static Lexicon {
    Lexicon::global()
}

#[cfg(test)]
mod tests;
