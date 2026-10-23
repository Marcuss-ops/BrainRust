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
    /// Version of the lexical knowledge tables (`semantic_core::lexicon`),
    /// so a vocabulary change can never serve a stale cached analysis.
    pub lexicon: String,
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
    /// Stable scene-local id the planner uses to report a search outcome in
    /// [`SearchFeedback`]. Deterministic for a given scene and proposal order.
    pub id: String,
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
    /// ISO date backing a dated member, so a planner can order and animate the
    /// group without re-parsing the surface. `None` when the member is not a
    /// dated event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_date: Option<String>,
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
    /// ISO date backing a fact whose `normalized_value` is a non-date
    /// conclusion (e.g. `acquitted`). Ordering uses this field so the
    /// canonical outcome stays readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_date: Option<String>,
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
    /// Deterministic rule-based weight in `[0, 1]`: dated and asserted facts
    /// about a grounded subject rank higher. Used for ordering only; it is not
    /// a measured relevance score.
    pub importance: f32,
    /// Canonical identifiers a downstream visual planner may bind to this
    /// fact: the subject, and the target when the fact links two entities.
    pub visual_concept_ids: Vec<String>,
}

impl EntityFact {
    /// Fills `importance` and `visual_concept_ids` from fields the extractor
    /// already grounded. Every producer calls this once, so the rule lives in
    /// one place.
    pub fn finalize(&mut self) {
        let outcome = self.predicate == "investigation_status"
            || matches!(
                self.normalized_value.as_deref(),
                Some("acquitted" | "convicted" | "annulled_with_remand" | "final")
            );
        let mut weight: f32 = if outcome {
            0.9
        } else if self.normalized_date.is_some() {
            0.7
        } else if self.target_entity_id.is_some() {
            0.6
        } else {
            0.4
        };
        if self.status != "asserted" {
            weight *= 0.6;
        }
        self.importance = weight;

        let mut ids = vec![self.subject_entity_id.clone()];
        if let Some(target) = &self.target_entity_id {
            if !ids.contains(target) {
                ids.push(target.clone());
            }
        }
        self.visual_concept_ids = ids;
    }
}

/// A short, source-grounded phrase derived from one or more atomic facts.
///
/// The text may change the wording but never the subject, state, date or
/// attribution: every claim must be justified by `evidence_fact_ids`, and the
/// span points at the sentence that carries the evidence. A summary without a
/// fact backing is not emitted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MicroSummary {
    pub id: String,
    /// `micro_summary`, `fact_comparison`, `step_sequence` or
    /// `timeline_summary`.
    pub kind: String,
    pub text: String,
    /// Facts that justify every claim in `text` (never empty).
    pub evidence_fact_ids: Vec<String>,
    /// Canonical subject of the summary, when one fact owns it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_entity_id: Option<String>,
    /// ISO-normalized date backing the summary, used for ordering only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_date: Option<String>,
    pub sentence_index: usize,
    /// Verbatim evidence sentence.
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub score: f32,
}

/// One rung of the asset-search ladder proposed by the engine. The text is
/// always an exact source slice; retrieval stays in PipelineGen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchQuery {
    /// `exact_mention`, `context_phrase` or `source_sentence`, in widening
    /// order. The exact identity is tried first; the sentence is the last
    /// resort for an illustrative stock search.
    pub kind: String,
    pub text: String,
}

/// What the planner should do when a search ladder is exhausted. Only the
/// planner can set this: the engine never sees a provider response.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchRetry {
    /// `broaden` (widen the query), `drop_entity` (the mention is not
    /// searchable) or `skip_visual` (leave the fact uncovered).
    pub action: String,
    pub reason: String,
}

/// One asset-search handshake entry for a proposal that needs media.
///
/// The engine emits the query ladder and `status = "pending"`. PipelineGen owns
/// retrieval and rewrites `status` (`satisfied` or `failed`) and `retry`, so a
/// failed search is reported in the same contract instead of being lost in the
/// planner's logs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchFeedbackEntry {
    /// Proposal id this entry reports on.
    pub proposal_id: String,
    pub scene_id: String,
    /// Widening query ladder, exact source slices only.
    pub queries: Vec<SearchQuery>,
    /// `pending` (engine), `satisfied` or `failed` (planner).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<SearchRetry>,
}

/// Asset-search feedback for the whole script. The engine emits one `pending`
/// entry per image-backed proposal; every later field belongs to the planner.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SearchFeedback {
    pub entries: Vec<SearchFeedbackEntry>,
}

/// One node of the whole-script story graph.
///
/// A node is either a canonical identity (`kind = "identity"`) or a grounded
/// fact (`kind = "fact"`). Identity nodes carry every scene they are mentioned
/// in, which is what makes a cross-scene link explicit instead of implicit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryNode {
    pub id: String,
    /// `identity` or `fact`.
    pub kind: String,
    /// Display form: the canonical surface for an identity, the value for a
    /// fact.
    pub label: String,
    /// Scene where the node is grounded first (its anchor).
    pub scene_id: String,
    /// Every scene the node is grounded in, in request order.
    pub scene_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_date: Option<String>,
    /// Sentence of the fact evidence; absent on an identity node, whose mention
    /// may sit in another scene.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentence_index: Option<usize>,
    /// Verbatim source slice at `[start_byte, end_byte)`.
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A grounded link between two story nodes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryEdge {
    pub id: String,
    /// `about` (identity -> fact) or `chronological` (fact -> fact).
    pub kind: String,
    pub from_id: String,
    pub to_id: String,
    /// True when the endpoints are grounded in different scenes. This is the
    /// flag the planner uses to find story material that spans distant scenes.
    pub cross_scene: bool,
    /// Verbatim source evidence for the link at `[start_byte, end_byte)`,
    /// always taken from the target node's scene.
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Whole-script graph over grounded identities, facts and their dates.
///
/// Only links the source supports are emitted: an identity is joined to its own
/// facts, and dated facts of one identity are ordered chronologically when the
/// identity is mentioned in at least two scenes. Relationships the text does not
/// assert are never invented.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StoryGraph {
    pub nodes: Vec<StoryNode>,
    pub edges: Vec<StoryEdge>,
}

/// One fact evaluated for visual representation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverageItem {
    pub fact_id: String,
    pub predicate: String,
    pub subject_entity_id: String,
    pub importance: f32,
    /// `covered` when a grounded proposal in the same scene represents it.
    pub status: String,
    /// Proposal types that represent the fact (empty when uncovered).
    pub proposal_types: Vec<String>,
    pub sentence_index: usize,
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Coverage of one scene: every grounded fact and whether something visual
/// already represents it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneCoverage {
    pub scene_id: String,
    /// Facts ordered by importance, then source position.
    pub facts: Vec<CoverageItem>,
}

/// Whole-script visual coverage report. PipelineGen reads this to know which
/// facts still need an image, a card or a chart instead of guessing.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct VisualCoverage {
    pub total_facts: usize,
    pub covered_facts: usize,
    pub scenes: Vec<SceneCoverage>,
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
    /// Fact-backed micro phrases and their comparison/sequence projections.
    pub micro_summaries: Vec<MicroSummary>,
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
    /// Grounded identities, facts and dates joined across scene boundaries.
    pub story_graph: StoryGraph,
    /// Which grounded facts a scene's proposals already represent.
    pub visual_coverage: VisualCoverage,
    /// Asset-search handshake: query ladders plus the planner's outcomes.
    pub search_feedback: SearchFeedback,
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
