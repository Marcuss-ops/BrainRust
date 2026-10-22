//! Versioned linguistic knowledge, kept OUT of Rust code.
//!
//! Every curated vocabulary this workspace used to hard-code in Rust
//! (stopwords, boilerplate phrases, object vocabularies, location cues, scale
//! words, legal markers, discourse cues, process phases) lives in JSON tables
//! under `rust/lexicon/`. Rust owns the
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

use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Version of the lexicon tables. Bump whenever a table changes semantics.
pub const LEXICON_VERSION: &str = "lexicon.v2";

/// Environment variable naming a directory of overriding table files.
pub const LEXICON_DIR_ENV: &str = "SEMANTIC_LEXICON_DIR";

// ── Table shapes ────────────────────────────────────────────────────

/// Candidate-exclusion vocabularies (visualner stopword fallback).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Stopwords {
    pub stop_words: Vec<String>,
    pub non_ascii_stop_words: Vec<String>,
    pub stop_phrases: Vec<String>,
    pub title_words: Vec<String>,
    /// Narration boilerplate that must never become an entity
    /// ("Imagine the", "Get ready", ...).
    pub generic_phrases: Vec<String>,
    /// Articulated prepositions that UDPipe splits into preposition +
    /// article ("del" -> "di" + "il"). The tagger alignment uses this closed
    /// morphosyntactic class to keep the contraction's tag.
    #[serde(default)]
    pub articulated_prepositions: Vec<String>,
}

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
    pub object_words: Vec<String>,
    /// Nouns that stay VISUAL_CONCEPT explicitly ("company").
    pub generic_visual_nouns: Vec<String>,
    pub company_suffixes: Vec<String>,
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
    pub currency_words: Vec<String>,
}

/// Contextual geographic cues (no gazetteer: syntax only).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocationCues {
    pub context_prepositions: Vec<String>,
    pub genitive_prepositions: Vec<String>,
    pub container_nouns: Vec<String>,
    pub month_names: Vec<String>,
    pub generic_location_nouns: Vec<String>,
    /// Locative prepositions that ground a bare prepositional phrase
    /// ("stadio di Torino" → located_in).
    pub sentence_locative_prepositions: Vec<String>,
}

/// Which grounded entity kinds may own a fact of one legal pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    Person,
    Org,
    PersonOrOrg,
    /// Person, court or procedural role (GUP, prosecutor, defence).
    LegalActor,
}

/// The value side of one legal fact pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Verbatim date surface from the same sentence (required).
    Date,
    /// Verbatim numeric surface from the same sentence (required).
    Number,
    /// A second grounded entity, e.g. the partner or the owning organization.
    Entity,
    /// The marker itself: the fact is an event, not a quantity.
    Marker,
}

/// One legal/procedural fact pattern. Markers are matched case-insensitively
/// inside one canonical sentence; longer, more specific phrases must come
/// before their fragments so the first match carries the sharper meaning.
#[derive(Debug, Clone, Deserialize)]
pub struct LegalPattern {
    pub predicate: String,
    /// Canonical conclusion stored in `normalized_value`.
    pub normalized: String,
    pub markers: Vec<String>,
    pub subject: String,
    pub value: String,
}

impl LegalPattern {
    pub fn subject_kind(&self) -> SubjectKind {
        match self.subject.as_str() {
            "person" => SubjectKind::Person,
            "org" => SubjectKind::Org,
            "person_or_org" => SubjectKind::PersonOrOrg,
            _ => SubjectKind::LegalActor,
        }
    }

    pub fn value_kind(&self) -> ValueKind {
        match self.value.as_str() {
            "date" => ValueKind::Date,
            "number" => ValueKind::Number,
            "entity" => ValueKind::Entity,
            _ => ValueKind::Marker,
        }
    }
}

/// A curated legal actor surface (a court or procedural role a general
/// tokenizer cannot type reliably); only the semantic type is added.
#[derive(Debug, Clone, Deserialize)]
pub struct EntityCue {
    pub cue: String,
    pub kind: String,
}

/// Legal/procedural extraction knowledge.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegalLexicon {
    pub patterns: Vec<LegalPattern>,
    /// Markers that make the age pattern skip a number ("16 anni di
    /// reclusione" is a sentence length, not an age).
    pub age_guard_markers: Vec<String>,
    pub entity_cues: Vec<EntityCue>,
    pub entity_alias_cues: Vec<EntityCue>,
}

/// One ordered process phase (facility lifecycle).
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessPhase {
    pub order: usize,
    pub name: String,
    pub markers: Vec<String>,
}

/// Process/event knowledge: ordered lifecycle phases plus the event
/// predicates that qualify a sentence for a visual timeline.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProcessPhases {
    /// Ordered by `order`.
    pub phases: Vec<ProcessPhase>,
    pub event_predicate_markers: Vec<String>,
}

/// One comparison operator and the markers that express it.
#[derive(Debug, Clone, Deserialize)]
pub struct OperatorCue {
    pub symbol: String,
    pub markers: Vec<String>,
}

/// Facility-fact knowledge (inauguration, capacity, operators, status).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FacilityLexicon {
    pub inauguration_markers: Vec<String>,
    pub capacity_markers: Vec<String>,
    pub capacity_unit_markers: Vec<String>,
    /// Ordered: the first matching operator wins.
    pub operators: Vec<OperatorCue>,
    pub status_uncertainty_cues: Vec<String>,
    pub status_negation_substrings: Vec<String>,
}

/// One explicit relation predicate and its multilingual markers.
#[derive(Debug, Clone, Deserialize)]
pub struct RelationMarkerGroup {
    pub predicate: String,
    pub markers: Vec<String>,
}

/// One entity-graph relation predicate (token-level markers).
#[derive(Debug, Clone, Deserialize)]
pub struct EntityRelationMarker {
    pub relation: String,
    pub markers: Vec<String>,
}

/// One comparison dimension and its multilingual keys.
#[derive(Debug, Clone, Deserialize)]
pub struct DimensionCue {
    pub name: String,
    pub markers: Vec<String>,
}

/// One number word and its numeric value ("tre" -> 3).
#[derive(Debug, Clone, Deserialize)]
pub struct NumberWord {
    pub value: i32,
    pub words: Vec<String>,
}

/// Every discourse/salience cue shared by phrase impact and scene
/// intelligence: one table instead of one list per module.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscourseCues {
    /// News-event salience cues per language; `default` covers every
    /// language without its own entry.
    pub content_cues: BTreeMap<String, Vec<String>>,
    pub filler_words: Vec<String>,
    pub filler_phrases: Vec<Vec<String>>,
    pub causal_after: Vec<Vec<String>>,
    pub causal_before: Vec<Vec<String>>,
    pub hypothetical: Vec<String>,
    pub attribution: Vec<String>,
    pub contribute: Vec<Vec<String>>,
    pub sequence: Vec<String>,
    pub comparison_signals: Vec<Vec<String>>,
    pub comparative_adj: Vec<String>,
    pub negation_tokens: Vec<String>,
    pub contrast_verbs: Vec<String>,
    /// Entity-graph relation predicates (token-level matching).
    pub entity_relation_markers: Vec<EntityRelationMarker>,
    /// Comparison dimensions ("investment", "cost", ...).
    pub dimensions: Vec<DimensionCue>,
    /// Separators that split a sentence around a causal/sequence marker.
    pub split_separators: Vec<String>,
    /// Relative time expressions ("anni dopo", "three years later", ...).
    pub relative_plural: Vec<Vec<String>>,
    pub relative_single: Vec<String>,
    pub relative_fixed: Vec<Vec<String>>,
    pub relative_expressions: Vec<String>,
    /// Year-unit words ("anno", "year", "jahr", "an").
    pub year_units: Vec<String>,
    /// Connectives between the two years of an interval ("dal 2020 al 2023").
    pub interval_connectives: Vec<String>,
    pub number_words: Vec<NumberWord>,
}

/// Explicit-relation and assertion-status vocabularies (one shared table).
/// The clause scoping and the negation/uncertainty readings are shared by
/// relation facts and legal facts so the two can never disagree again.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RelationCues {
    /// Separators that bound the clause a status is read from.
    pub clause_separators: Vec<String>,
    pub uncertainty_cues: Vec<String>,
    pub relation_markers: Vec<RelationMarkerGroup>,
    /// Negation cue words per language (`default` fallback), used to mark
    /// an assertion `negated` instead of dropping it.
    pub negation_cues: BTreeMap<String, Vec<String>>,
}

fn cues_for<'a>(table: &'a BTreeMap<String, Vec<String>>, language: &str) -> &'a [String] {
    let key = language.split(['-', '_']).next().unwrap_or("en");
    table
        .get(key)
        .or_else(|| table.get("default"))
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

impl DiscourseCues {
    /// Salience cues for a BCP-47 tag (falls back to the `default` table).
    pub fn content_cues_for(&self, language: &str) -> &[String] {
        cues_for(&self.content_cues, language)
    }
}

impl RelationCues {
    pub fn negation_cues_for(&self, language: &str) -> &[String] {
        cues_for(&self.negation_cues, language)
    }
}

/// The complete loaded lexicon.
#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    pub stopwords: Stopwords,
    pub objects: ObjectVocabulary,
    pub gazetteer: EntityGazetteer,
    /// Lowercased `gazetteer.locations` and place names/aliases, built once.
    pub place_index: std::collections::HashSet<String>,
    /// Lowercased ambiguous aliases per language, built once.
    pub place_ambiguities: BTreeMap<String, std::collections::HashSet<String>>,
    pub scale_words: ScaleWords,
    pub location_cues: LocationCues,
    pub legal: LegalLexicon,
    pub process_phases: ProcessPhases,
    pub facility: FacilityLexicon,
    pub discourse: DiscourseCues,
    pub relation: RelationCues,
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
fn build_place_index(gazetteer: &EntityGazetteer) -> std::collections::HashSet<String> {
    let mut index = std::collections::HashSet::with_capacity(
        gazetteer.locations.len() + gazetteer.places.len() * 6,
    );
    index.extend(
        gazetteer
            .locations
            .iter()
            .map(|location| location.to_lowercase()),
    );
    for place in &gazetteer.places {
        index.insert(place.name.to_lowercase());
        index.extend(place.aliases.iter().map(|alias| alias.to_lowercase()));
    }
    index
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
            let place_index = build_place_index(&gazetteer);
            let place_ambiguities = build_place_ambiguities(&gazetteer);
            Lexicon {
                stopwords: load_table(
                    "stopwords.json",
                    include_str!("../../lexicon/stopwords.json"),
                ),
                objects: load_table(
                    "object_vocabulary.json",
                    include_str!("../../lexicon/object_vocabulary.json"),
                ),
                gazetteer,
                place_index,
                place_ambiguities,
                scale_words: load_table(
                    "scale_words.json",
                    include_str!("../../lexicon/scale_words.json"),
                ),
                location_cues: load_table(
                    "location_cues.json",
                    include_str!("../../lexicon/location_cues.json"),
                ),
                legal: load_table(
                    "legal_patterns.json",
                    include_str!("../../lexicon/legal_patterns.json"),
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
            }
        })
    }

    /// True when a capitalized surface is a known place. The capitalization
    /// guard keeps lowercase common nouns that collide with a GeoNames alias
    /// ("casa" -> Chas, "mare" -> Male) out of the LOCATION typing.
    pub fn is_place(&self, surface: &str, language: &str) -> bool {
        if !surface.chars().next().is_some_and(char::is_uppercase) {
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
mod tests {
    use super::*;

    #[test]
    fn embedded_tables_load_with_the_documented_shape() {
        let lexicon = Lexicon::global();
        assert!(!lexicon.stopwords.stop_words.is_empty());
        assert!(lexicon
            .stopwords
            .generic_phrases
            .iter()
            .any(|p| p == "imagine"));
        assert_eq!(lexicon.object_terms("en").len(), 24);
        assert_eq!(lexicon.object_terms("it").len(), 20);
        // BCP-47 subtags resolve to the base language.
        assert_eq!(
            lexicon.object_terms("pt-BR").len(),
            lexicon.object_terms("pt").len()
        );
        assert!(lexicon.object_terms("xx").is_empty());
        assert_eq!(lexicon.legal.patterns.len(), 15);
        assert_eq!(lexicon.process_phases.phases.len(), 4);
        assert_eq!(lexicon.relation.relation_markers.len(), 7);
    }

    #[test]
    fn geonames_places_are_indexed_with_a_capitalization_guard() {
        let lexicon = Lexicon::global();
        assert!(lexicon.is_place("Pavia", "it"));
        assert!(lexicon.is_place("Garlasco", "it"));
        assert!(lexicon.is_place("Vigevano", "it"));
        // Lowercase common nouns that collide with a GeoNames alias stay out.
        assert!(!lexicon.is_place("casa", "it"));
        assert!(!lexicon.is_place("mare", "it"));
        assert!(!lexicon.is_place("pavia", "it"));
        // "Corte" is both a French town and the Italian common noun "court".
        assert!(!lexicon.is_place("Corte", "it"));
        assert!(lexicon.is_place("Corte", "en"));
    }

    #[test]
    fn discourse_cues_fall_back_to_the_default_language() {
        let lexicon = Lexicon::global();
        assert_eq!(
            lexicon.discourse.content_cues_for("it"),
            lexicon.discourse.content_cues.get("it").unwrap().as_slice()
        );
        let fallback = lexicon.discourse.content_cues_for("xx");
        let default = lexicon.discourse.content_cues.get("default").unwrap();
        assert_eq!(fallback, default.as_slice());
    }

    #[test]
    fn order_sensitve_tables_keep_their_data_order() {
        // Longer markers must precede their fragments ("a causa di" before
        // "a causa"), so the first match carries the sharper meaning.
        let lexicon = Lexicon::global();
        assert_eq!(lexicon.discourse.causal_after[0], ["a", "causa", "di"]);
        assert_eq!(lexicon.legal.patterns[0].normalized, "not_convicted");
    }
}
