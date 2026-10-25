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
