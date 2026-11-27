//! Discourse, relation and process cue vocabularies: phase markers,
//! operators, facilities, relation markers and the shared cue lookup.

use serde::Deserialize;
use std::collections::BTreeMap;

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
    /// Explicit facility nouns, across supported languages, used to recognize
    /// a named structure only when followed by a source-grounded name.
    #[serde(default)]
    pub facility_nouns: Vec<String>,
    /// Explicit asserted predicates for facts such as "is located in".
    #[serde(default)]
    pub location_relation_markers: Vec<String>,
    /// Genitive forms used only for a facility noun phrase such as
    /// "Stadio Aurora di Torino"; general prepositions are too ambiguous.
    #[serde(default)]
    pub bare_location_prepositions: Vec<String>,
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

/// Minimal multilingual table of conclusion markers used by the intro agenda.
/// A topic whose sentence states one of these outside the opening is a spoiler
/// and is not offered as an introduction.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IntroAgendaLexicon {
    #[serde(default)]
    pub spoiler_cues: BTreeMap<String, Vec<String>>,
}

impl IntroAgendaLexicon {
    pub fn is_spoiler(&self, language: &str, text: &str) -> bool {
        let key = language.split(['-', '_']).next().unwrap_or(language);
        let cues = self
            .spoiler_cues
            .get(key)
            .or_else(|| self.spoiler_cues.get("en"));
        let Some(cues) = cues else {
            return false;
        };
        let lowered = text.to_lowercase();
        let words: std::collections::HashSet<&str> = lowered
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        cues.iter().any(|cue| {
            let cue = cue.trim().to_lowercase();
            if cue.is_empty() {
                return false;
            }
            if cue.contains(' ') {
                lowered.contains(&cue)
            } else {
                words.contains(cue.as_str())
            }
        })
    }
}

/// One entity-graph relation predicate (token-level markers).
#[derive(Debug, Clone, Deserialize)]
pub struct EntityRelationMarker {
    pub relation: String,
    pub markers: Vec<String>,
}

/// One canonical predicate resolvable from a typed `Quantity` through the
/// Universal Dependencies parse. The ontology is language-independent: only
/// the lemma list is localized, and the same resolver reads every language.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AttributePredicateDef {
    pub canonical: String,
    /// Quantity dimension this predicate measures (`length`, `mass`, ...).
    pub dimension: String,
    /// Entity families the predicate may bind, such as `PERSON`.
    #[serde(default)]
    pub subject_kinds: Vec<String>,
    /// Localized display label for the card line.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Language -> lemma list. Lemmas are matched against the UDPipe lemma,
    /// so inflected surfaces need no entry.
    #[serde(default)]
    pub lemmas: BTreeMap<String, Vec<String>>,
}

/// A property already produced by a dedicated extractor (age, role). It
/// contributes a canonical name, a dimension and display labels only; it is
/// never re-extracted here.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AttributeExtractorDef {
    pub canonical: String,
    pub dimension: String,
    /// Localized singular/plural unit surfaces, when the extractor does not
    /// keep the source unit itself.
    #[serde(default)]
    pub units: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

/// Canonical attribute ontology (`lexicon/attribute_predicates.json`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AttributePredicates {
    #[serde(default)]
    pub predicates: Vec<AttributePredicateDef>,
    #[serde(default)]
    pub extractors: Vec<AttributeExtractorDef>,
}

impl AttributePredicates {
    /// Resolves a UDPipe lemma to the canonical predicate it expresses. A
    /// lemma outside the ontology resolves to nothing: the resolver never
    /// guesses from a unit alone.
    pub fn resolve(
        &self,
        language: &str,
        lemma: &str,
        dimension: &str,
    ) -> Option<&AttributePredicateDef> {
        let key = language.split(['-', '_']).next().unwrap_or(language);
        let lemma = lemma.to_lowercase();
        self.predicates.iter().find(|predicate| {
            predicate.dimension == dimension
                && predicate.lemmas.get(key).is_some_and(|lemmas| {
                    lemmas
                        .iter()
                        .any(|candidate| candidate.to_lowercase() == lemma)
                })
        })
    }

    /// Declared predicates for one dimension, in registry order.
    pub fn for_dimension<'a>(
        &'a self,
        dimension: &str,
    ) -> impl Iterator<Item = &'a AttributePredicateDef> + 'a {
        let dimension = dimension.to_string();
        self.predicates
            .iter()
            .filter(move |predicate| predicate.dimension == dimension)
    }

    /// Localized display label for a canonical attribute name.
    pub fn label<'a>(&'a self, canonical: &str, language: &str) -> Option<&'a str> {
        let key = language.split(['-', '_']).next().unwrap_or(language);
        self.predicates
            .iter()
            .find(|entry| entry.canonical == canonical)
            .and_then(|entry| entry.labels.get(key))
            .or_else(|| {
                self.extractors
                    .iter()
                    .find(|entry| entry.canonical == canonical)
                    .and_then(|entry| entry.labels.get(key))
            })
            .map(String::as_str)
    }

    /// Localized singular/plural unit surfaces for an extractor-owned
    /// property (`age`).
    pub fn units<'a>(&'a self, canonical: &str, language: &str) -> &'a [String] {
        let key = language.split(['-', '_']).next().unwrap_or(language);
        self.extractors
            .iter()
            .find(|entry| entry.canonical == canonical)
            .and_then(|entry| entry.units.get(key))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
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
    /// Ratio/multiplier terms scoped to one supported language.
    #[serde(default)]
    pub ratio_signals: BTreeMap<String, Vec<String>>,
    /// Verbs that explicitly describe a transition from the first value to the
    /// second one in a quantitative comparison.
    #[serde(default)]
    pub change_signals: BTreeMap<String, Vec<String>>,
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
