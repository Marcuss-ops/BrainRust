//! Legal-domain knowledge: outcome patterns, entity cues, comparison
//! markers and the micro-summary vocabulary.

use serde::Deserialize;
use std::collections::BTreeMap;

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

/// Legal/procedural extraction knowledge for ONE language. The pack is an
/// optional domain layer (`lexicon/legal/<lang>.json`): a language without a
/// pack produces no legal facts instead of borrowing another language's
/// markers. The core keeps working through the generic fact pipeline.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegalLexicon {
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub patterns: Vec<LegalPattern>,
    /// Markers that make the age pattern skip a number ("16 anni di
    /// reclusione" is a sentence length, not an age).
    #[serde(default)]
    pub age_guard_markers: Vec<String>,
    #[serde(default)]
    pub entity_cues: Vec<EntityCue>,
    #[serde(default)]
    pub entity_alias_cues: Vec<EntityCue>,
    /// Sentences that talk about finality, not a new conviction.
    #[serde(default)]
    pub finality_markers: Vec<String>,
    /// Outcome words that qualify a dated micro summary as one step.
    #[serde(default)]
    pub outcome_markers: Vec<String>,
    #[serde(default)]
    pub comparison: LegalComparison,
    #[serde(default)]
    pub micro: MicroLexicon,
}

/// Cues and titles for the fact-comparison and step-sequence groups.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegalComparison {
    #[serde(default)]
    pub conviction_cues: Vec<String>,
    #[serde(default)]
    pub conviction_exclusions: Vec<String>,
    #[serde(default)]
    pub investigation_cues: Vec<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub sequence_title: String,
}

/// Micro-summary rendering for one language: templates and date suffixes.
/// A language without templates emits no micro summaries rather than Italian
/// text under a foreign language tag.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MicroLexicon {
    /// `"predicate|normalized"` → template with `{name}`, `{normalized}`,
    /// `{target}` and `{date}` placeholders.
    #[serde(default)]
    pub templates: BTreeMap<String, String>,
    #[serde(default)]
    pub date_year: String,
    #[serde(default)]
    pub date_surface: String,
    #[serde(default)]
    pub year_surface: String,
}

impl LegalLexicon {
    /// Template for one grounded fact, if this language renders it.
    pub fn micro_template(&self, predicate: &str, normalized: &str) -> Option<&str> {
        self.micro
            .templates
            .get(&format!("{predicate}|{normalized}"))
            .map(String::as_str)
    }
}
