//! `semantic_manifest.v2` — the single typed contract returned by the Rust
//! semantic engine.
//!
//! The manifest is the ONE source of truth PipelineGen reads. Per the contract
//! every candidate carries UTF-8 byte offsets, verbatim evidence, a canonical
//! id, a score and provenance (which analyzer produced it). The engine keeps
//! every valid candidate; the Go planner selects what to visualize. Nothing is
//! invented to fill an empty category: an extractor that finds nothing emits
//! zero results.

use serde::{Deserialize, Serialize};

use crate::identity::CanonicalIdentity;
use crate::numbers::NumericFact;
use crate::relations::RelationView;

/// Wire version of the manifest.
pub const SCHEMA_VERSION: &str = "semantic_manifest.v2";

/// Version of the analyzer stack recorded on the manifest so a replay can be
/// tied to the exact extractor build that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalyzerVersions {
    pub visualner: String,
    pub semantic_core: String,
    pub phrase_impact: String,
}

/// One grounded candidate with the fields shared by every projection.
///
/// `evidence` must equal the verbatim source slice at
/// `[start_byte, end_byte)`; the grounding checker enforces this (NO
/// EVIDENCE -> NO CANDIDATE). `score` is a deterministic relevance score in
/// `[0, 1]`, never a provider confidence. `provenance` names the analyzer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    /// Canonical id resolved by the identity registry.
    pub id: String,
    /// Exact source surface.
    pub text: String,
    /// Closed semantic type.
    #[serde(rename = "type")]
    pub kind: String,
    pub start_byte: usize,
    pub end_byte: usize,
    /// Stable scene-local index assigned by the canonical sentence splitter.
    pub sentence_index: usize,
    pub evidence: String,
    pub score: f32,
    pub provenance: String,
}

/// One comparable numeric value inside a chart proposal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartItem {
    /// The sentence the value appears in (verbatim).
    pub label: String,
    /// Exact numeric surface (e.g. `"42%"`), a verbatim source slice.
    pub text: String,
    /// `NUMBER`, `PERCENT` or `MONEY`, as extracted.
    pub kind: String,
    pub value: f64,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A proposed visual composition for a scene, produced by the engine as a
/// *proposal* (with its evidence) and left for the planner to validate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualProposal {
    /// `multiple_entities`, `chart`, `timeline`, `entity_card`, ...
    pub proposal_type: String,
    /// Canonical ids of the identities the proposal is about.
    pub entity_ids: Vec<String>,
    /// Reusable query suggestions; retrieval remains owned by PipelineGen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual_query_suggestions: Vec<String>,
    /// Suggested layout (`side_by_side`, `grid`, `bar_comparison`, ...).
    pub suggested_visual: String,
    /// `requires_semantic_validation` until the planner resolves assets and
    /// budget; `validated` only after the planner confirms comparability and
    /// available media.
    pub status: String,
    pub score: f32,
    /// Comparable values backing a chart proposal (empty otherwise).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chart_items: Vec<ChartItem>,
    /// Date surfaces backing a timeline proposal (empty otherwise).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temporal_surfaces: Vec<String>,
    /// Verbatim source evidence backing the proposal.
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A grounded multi-candidate structure that a planner can map to a visual
/// recipe. The engine describes source order and evidence; it never selects
/// an animation or invents missing list members.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticGroup {
    pub id: String,
    /// `chronological`, `process`, `comparison`, `enumeration` or `ranking`.
    pub group_type: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranking_basis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_count: Option<usize>,
    /// `None` when the source does not state a target size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
    pub members: Vec<SemanticGroupMember>,
}

/// One exact, source-grounded member of a semantic group.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticGroupMember {
    pub candidate_id: String,
    /// Source scene for this member. Local groups carry it too so a planner
    /// can consume local and cross-scene groups through the same contract.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scene_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<usize>,
    pub sentence_index: usize,
    pub text: String,
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_surface: Option<String>,
}

/// A source-grounded fact explicitly attached to a canonical entity.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityFact {
    pub id: String,
    pub subject_entity_id: String,
    pub predicate: String,
    pub value: String,
    #[serde(rename = "type")]
    pub value_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    /// `asserted`, `negated` or `uncertain` as stated by the source.
    pub status: String,
    pub sentence_index: usize,
    pub evidence: String,
    /// Exact value surface span, such as the date or numeric quantity.
    pub start_byte: usize,
    pub end_byte: usize,
    /// Sentence span that proves the predicate-to-value association.
    pub evidence_start_byte: usize,
    pub evidence_end_byte: usize,
}

/// Every fact and proposal extracted for one scene.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneManifest {
    pub scene_id: String,
    /// `valid`, or `rejected` when the extractor refused the scene.
    pub status: &'static str,
    pub reason: Option<String>,
    pub entities: Vec<Candidate>,
    /// Canonical identities resolved across the whole script.
    pub identities: Vec<CanonicalIdentity>,
    /// Relations split into co_mentions (grouping signal) and asserted ones.
    pub relations: RelationView,
    pub numeric_facts: Vec<NumericFact>,
    /// Date facts (ISO-normalized only when fully resolved).
    pub temporal_facts: Vec<crate::dates::TemporalFact>,
    /// Facts linked to an entity only when an explicit predicate supports it.
    pub entity_facts: Vec<EntityFact>,
    /// Editorial sentence bullets (source-derived, never paraphrased).
    pub bullets: Vec<Candidate>,
    pub short_phrases: Vec<Candidate>,
    pub long_phrases: Vec<Candidate>,
    pub quotes: Vec<Candidate>,
    pub important_words: Vec<Candidate>,
    pub visual_proposals: Vec<VisualProposal>,
    /// Grounded multi-item structures suitable for planner selection.
    pub semantic_groups: Vec<SemanticGroup>,
    /// Candidates dropped by the grounding check (never silently lost).
    pub rejected_items: usize,
}

/// The whole-script manifest: one request, one response, one source of truth.
#[derive(Debug, Clone, Serialize)]
pub struct SemanticManifest {
    pub schema_version: &'static str,
    pub language: String,
    /// FNV-1a 64 over the canonical request (language + scenes + analyzer
    /// versions). Equal inputs give equal fingerprints (idempotent replay).
    pub fingerprint: String,
    pub analyzers: AnalyzerVersions,
    pub scenes: Vec<SceneManifest>,
    /// Optional whole-script editorial products computed in this same batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editorial: Option<EditorialManifest>,
}

/// Bounded editorial projection carried by the semantic manifest. Internal
/// ranking graphs are intentionally omitted; only products consumed by Go
/// planning and their stage timings cross the worker boundary.
#[derive(Debug, Clone, Serialize)]
pub struct EditorialManifest {
    pub summary: String,
    pub bullet_points: Vec<crate::phrase_impact::BulletPoint>,
    pub heavy_sentences: Vec<crate::phrase_impact::RankedSentence>,
    pub chapter_manifest: crate::phrase_impact::ChapterManifest,
    pub scene_highlights: Vec<crate::phrase_impact::SceneHighlight>,
    pub timings: crate::phrase_impact::StageTimings,
}

impl From<crate::phrase_impact::ResultDocument> for EditorialManifest {
    fn from(result: crate::phrase_impact::ResultDocument) -> Self {
        Self {
            summary: result.summary,
            bullet_points: result.bullet_points,
            heavy_sentences: result.heavy_sentences,
            chapter_manifest: result.chapter_manifest,
            scene_highlights: result.scene_highlights,
            timings: result.timings,
        }
    }
}

/// Editorial enrichment inputs embedded in `analyze_script`, so semantic and
/// Phrase Impact products can share one worker analysis request. The caller
/// supplies vectors (or explicitly opts into lexical mode); Rust performs no
/// model inference.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EditorialRequest {
    pub request: crate::phrase_impact::Request,
    #[serde(default)]
    pub scenes: Vec<crate::phrase_impact::SceneInput>,
    #[serde(default)]
    pub chapter_options: crate::phrase_impact::ChapterOptions,
}

/// Request shape: one batch carrying every scene of a script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneInput {
    pub scene_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptRequest {
    /// BCP-47 language tag of every scene (required).
    pub language: String,
    pub scenes: Vec<SceneInput>,
    /// When present, summary, chapter, heavy-sentence and highlight products
    /// are computed and returned as part of this semantic manifest.
    #[serde(default)]
    pub editorial: Option<EditorialRequest>,
    /// Optional safety cap on entities per scene. Zero means no editorial
    /// budget: the engine keeps every grounded entity and the planner picks
    /// the top-N. The cap exists only to bound pathological inputs.
    #[serde(default)]
    pub entity_safety_cap: usize,
}
