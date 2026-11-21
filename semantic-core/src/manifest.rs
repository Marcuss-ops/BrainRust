//! `semantic_manifest.v3` — the single typed contract returned by the Rust
//! semantic engine.
//!
//! The manifest is the ONE source of truth PipelineGen reads. Per the contract
//! every candidate carries UTF-8 byte offsets, verbatim evidence, a canonical
//! id, a score and provenance (which analyzer produced it). The engine keeps
//! every valid candidate; the Go planner selects what to visualize. Nothing is
//! invented to fill an empty category: an extractor that finds nothing emits
//! zero results.

use serde::{Deserialize, Serialize};

use crate::identity::{CanonicalIdentity, LinkedEntity};
use crate::narrative::DiscourseProjection;
use crate::numbers::NumericFact;
use crate::relations::RelationView;

#[path = "manifest/search_story.rs"]
mod search_story;
pub use search_story::*;

/// Wire version of the manifest.
pub const SCHEMA_VERSION: &str = "semantic_manifest.v3";

/// Versions this decoder accepts. `semantic_manifest.v2` is the migration
/// path: it has no `entities` projection and no `entity_index` analyzer field,
/// both of which default on decode.
pub const SUPPORTED_SCHEMA_VERSIONS: &[&str] = &["semantic_manifest.v2", SCHEMA_VERSION];

/// Version of the analyzer stack recorded on the manifest so a replay can be
/// tied to the exact extractor build that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnalyzerVersions {
    pub visualner: String,
    pub semantic_core: String,
    pub phrase_impact: String,
    /// Version of the lexical knowledge tables (`semantic_core::lexicon`),
    /// so a vocabulary change can never serve a stale cached analysis.
    pub lexicon: String,
    /// Version of the packaged entity index used for v3 linked ids. Defaults
    /// to empty when decoding a v2 manifest that predates the field.
    #[serde(default)]
    pub entity_index: String,
    /// Version of deterministic entity-link scoring and explanation output.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub linker: String,
    /// Version of the word-number grammar/fraction rules.
    #[serde(default)]
    pub numbers: String,
    /// Version of the relative-date grammar and arithmetic.
    #[serde(default)]
    pub dates: String,
    /// Version of unit attachment, period binding and quantity interpretation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quantities: String,
    /// Version of grounded person-role extraction.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub roles: String,
    /// Version of explicit person-age extraction.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub age_facts: String,
    /// Version of explicit person-person relation extraction.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub person_relations: String,
    /// Version of the packaged place index and local place linker.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub places: String,
    /// Version of explicit place-route extraction.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub routes: String,
    /// Version of the grounded event-frame extractor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub events: String,
    /// Version of pronoun/coreference and thread resolution.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub discourse: String,
}

/// Rank of a source-grounded title held by a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleRank {
    Head,
    Deputy,
    Member,
    Former,
    Acting,
}

/// Tense/status asserted by an explicit role marker in the same sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleTense {
    Current,
    Former,
    Appointed,
    Resigned,
    Unknown,
}

/// Whether the source asserts, negates, or leaves the role uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleStatus {
    Asserted,
    Negated,
    Uncertain,
}

/// A title attached to a grounded person mention in one scene.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PersonRole {
    pub id: String,
    pub person_id: String,
    pub role_code: String,
    pub role_surface: String,
    /// `masculine`, `feminine`, or `unknown`; populated only when the matched
    /// lexical surface explicitly carries a gender feature.
    #[serde(default = "unknown_gender")]
    pub gender: String,
    pub rank: RoleRank,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<crate::dates::TemporalFact>,
    pub tense: RoleTense,
    pub status: RoleStatus,
    pub start_byte: usize,
    pub end_byte: usize,
    pub sentence_index: usize,
    pub evidence: String,
}

fn unknown_gender() -> String {
    "unknown".to_string()
}

/// A source-grounded age attached to one unambiguous PERSON mention.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PersonAgeFact {
    pub person_id: String,
    pub age: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_birth_year: Option<i32>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub sentence_index: usize,
    pub evidence: String,
}

/// An explicit relation between two grounded people in one sentence.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PersonRelation {
    pub from_id: String,
    pub to_id: String,
    pub kind: String,
    pub status: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub sentence_index: usize,
    pub evidence: String,
}

/// A source-grounded place mention, linked only when its exact alias is
/// unambiguous (or a same-sentence parent resolves the collision).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Place {
    pub id: String,
    pub entity_local_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_id: Option<String>,
    pub link_status: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parent_ids: Vec<String>,
    pub role: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub sentence_index: usize,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RouteStop {
    pub place_id: String,
    pub role: String,
}

/// A route is emitted only for a grounded origin/destination pair with an
/// explicit movement cue in the same sentence.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Route {
    pub id: String,
    pub mode: String,
    /// Grounded subject identity used when consecutive sentence hops merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    pub stops: Vec<RouteStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub straight_line_km: Option<f64>,
    pub status: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub evidence: String,
}

/// One hop's source evidence in a script-level route assembled across scenes.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RouteEvidence {
    pub scene_id: String,
    pub route_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub evidence: String,
}

/// Scene-qualified stop reference for a multi-scene route. Place ids remain
/// scene-local; the scene id makes the reference unambiguous.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CrossSceneRouteStop {
    pub scene_id: String,
    pub place_id: String,
    pub role: String,
}

/// A grounded journey stitched from consecutive scene-local route hops.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CrossSceneRoute {
    pub id: String,
    pub subject_id: String,
    pub mode: String,
    pub stops: Vec<CrossSceneRouteStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub straight_line_km: Option<f64>,
    pub status: String,
    pub evidence: Vec<RouteEvidence>,
}

/// One grounded candidate with the fields shared by every projection.
///
/// `evidence` must equal the verbatim source slice at
/// `[start_byte, end_byte)`; the grounding checker enforces this (NO
/// EVIDENCE -> NO CANDIDATE). `score` is a deterministic relevance score in
/// `[0, 1]`, never a provider confidence. `provenance` names the analyzer.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct VisualProposal {
    /// Stable scene-local id the planner uses to report a search outcome in
    /// [`SearchFeedback`]. Deterministic for a given scene and proposal order.
    pub id: String,
    /// `multiple_entities`, `chart`, `timeline`, `entity_card`, ...
    pub proposal_type: String,
    /// Canonical ids of the identities the proposal is about.
    pub entity_ids: Vec<String>,
    /// Reusable query suggestions; retrieval remains owned by PipelineGen.
    #[serde(default)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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

/// A grounded multi-sentence passage kept as one editorial unit.
///
/// A segment is 2–5 consecutive sentences cut at the canonical segmenter's
/// boundaries (never mid-sentence), anchored by a grounded signal in its
/// first and last sentence, and scored with the shared sentence salience
/// (`semantic_core::bullets`) averaged over the window. Segments never
/// overlap: the best windows win, in stable order.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ImportantSegment {
    pub id: String,
    /// Always `important_segment`.
    pub kind: String,
    /// Inclusive scene-local sentence range, from the canonical segmenter.
    pub sentence_index_start: usize,
    pub sentence_index_end: usize,
    /// Verbatim source slice from the first to the last sentence boundary.
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
    /// Deterministic salience in `[0, 1]`: the shared sentence salience
    /// averaged over the window's sentences.
    pub score: f32,
    /// Grounded signals counted inside the window (for auditability).
    pub visual_mentions: usize,
    pub measurement_mentions: usize,
}

/// Wire version of the semantic consolidation projection carried by the same
/// manifest. It is additive: raw scene collections remain the
/// complete inventory, and this section is the compiler's selective production
/// view over them.
pub const VISUAL_CONSOLIDATION_SCHEMA: &str = "visual_consolidation.v1";

/// Raw inventory of one script before any production selection. These counts
/// are measured, never estimated: they come from the scene manifests the same
/// worker returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConsolidationInventory {
    pub entity_mentions: usize,
    pub unique_identities: usize,
    pub named_identities: usize,
    pub facts: usize,
    pub canonical_facts: usize,
    pub proposals: usize,
    pub search_entries: usize,
    /// Query rungs inside `search_feedback` entries. Suggestions carried by
    /// `visual_proposals` are counted under `proposals`; `raw_queries` in the
    /// metrics uses this same definition, so production and raw compare
    /// like-for-like.
    pub queries: usize,
    pub temporal_facts: usize,
    pub canonical_dates: usize,
    pub micro_summaries: usize,
    pub canonical_micro_summaries: usize,
    pub groups: usize,
    pub timelines: usize,
}

/// One canonical fact after duplicate mentions are merged. Every raw fact id is
/// retained so the inventory stays auditable, while the production view sees a
/// single claim with one representation decision.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ConsolidatedFact {
    /// Deterministic canonical id (`cf_0001`, ...).
    pub id: String,
    /// Stable deduplication key (`subject|predicate|value|date|status`).
    pub fact_key: String,
    /// Every raw fact id merged into this canonical fact.
    pub fact_ids: Vec<String>,
    pub subject_entity_id: String,
    pub predicate: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_date: Option<String>,
    pub status: String,
    pub importance: f32,
    pub scene_ids: Vec<String>,
    /// Sentence index of the first raw mention, used to bind a timeline member
    /// to the fact without re-parsing the source.
    pub sentence_index: usize,
    /// Verbatim evidence of the first raw mention.
    pub evidence: String,
    /// Exact value span of the first raw mention.
    pub start_byte: usize,
    pub end_byte: usize,
    /// Sentence span that grounds the first raw mention; this is the span a
    /// fact card uses as its verbatim evidence.
    pub evidence_start_byte: usize,
    pub evidence_end_byte: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<FactRepresentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncovered_reason: Option<String>,
}

/// Which production candidate represents a canonical fact.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct FactRepresentation {
    pub candidate_id: String,
    /// `timeline`, `chart`, `fact_card`, ...
    pub recipe: String,
}

/// One canonical date: the normalized value plus every source surface and span
/// that produced it. Duplicate partial dates are not discarded; they are
/// collapsed into one reference while preserving the original evidence.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ConsolidatedDate {
    pub normalized_date: String,
    pub count: usize,
    pub surfaces: Vec<String>,
    pub spans: Vec<ConsolidatedDateSpan>,
}

/// One source occurrence of a canonical date.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConsolidatedDateSpan {
    pub scene_id: String,
    pub surface: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A numeric surface the source explicitly frames as a reference, not a
/// quantity ("l'impronta numero 33"). It is kept in the inventory and excluded
/// from quantitative chart candidates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReferenceNumber {
    pub scene_id: String,
    pub surface: String,
    pub reason: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A grounded event-to-event ordering link derived from one timeline group.
/// The engine only states precedence the source already orders.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConsolidatedEventLink {
    pub id: String,
    pub group_id: String,
    pub scene_id: String,
    pub from_candidate_id: String,
    pub to_candidate_id: String,
    /// `precedes` only: the next member is later in the source order.
    pub kind: String,
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// One selective production candidate: the unit a planner may turn into a
/// composition. The raw candidate pool remains in the scene manifests; this
/// list is deduplicated, ranked and budget-aware.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ProductionCandidate {
    /// Deterministic id (`vc_0001`, ...).
    pub id: String,
    /// `entity_search`, `entity_card`, `multiple_entities`, `timeline`,
    /// `fact_card` or `chart`.
    pub recipe: String,
    /// 1-based priority after deterministic ranking (1 is most important).
    pub rank: usize,
    /// Deterministic relevance in `[0, 1]`, used for ordering only.
    pub priority: f32,
    /// Scene that anchors the verbatim evidence span.
    pub scene_id: String,
    /// Every scene the candidate is grounded in.
    pub scene_ids: Vec<String>,
    pub entity_ids: Vec<String>,
    pub fact_ids: Vec<String>,
    pub group_ids: Vec<String>,
    pub title: String,
    /// Verbatim source evidence at `[start_byte, end_byte)` inside `scene_id`.
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
    /// Layout hint for the planner (`search_asset`, `entity_card`, `timeline`,
    /// `bar_comparison`, ...). The engine describes, it does not select.
    pub suggested_visual: String,
    /// Numeric values backing a chart candidate (empty otherwise).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chart_items: Vec<ChartItem>,
    /// Date surfaces backing a timeline candidate (empty otherwise).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temporal_surfaces: Vec<String>,
    /// Assets with the same key can be retrieved once and reused. Cards and
    /// multi-entity compositions deliberately share their entity's asset.
    pub asset_key: String,
    /// Deterministic query ladder. Empty when the recipe renders text/data
    /// and needs no external media search.
    pub queries: Vec<SearchQuery>,
    pub requires_media: bool,
    /// Stable key proving the candidate is unique (`entity:person_01:search`).
    pub dedup_key: String,
    /// Canonical facts represented by this candidate.
    pub covered_fact_ids: Vec<String>,
    /// Why this candidate exists and why it is ranked here.
    pub rationale: Vec<String>,
}

/// Measured before/after volume for the same manifest. Every field is an
/// arithmetic count, not a provider measurement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConsolidationMetrics {
    pub raw_proposals: usize,
    pub production_candidates: usize,
    pub raw_search_entries: usize,
    pub production_search_candidates: usize,
    pub raw_queries: usize,
    pub production_queries: usize,
    pub duplicate_entity_mentions_collapsed: usize,
    pub duplicate_facts_merged: usize,
    pub duplicate_queries_removed: usize,
    pub duplicate_proposals_removed: usize,
    pub estimated_search_jobs: usize,
    pub shared_asset_reuses: usize,
    pub canonical_facts: usize,
    pub covered_facts: usize,
    pub uncovered_facts: usize,
    pub reference_numbers: usize,
}

/// Per-scene volume and coverage, so a planner can see where consolidation
/// removed work instead of only reading a script total.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConsolidationSceneStats {
    pub scene_id: String,
    pub raw_proposals: usize,
    pub production_candidates: usize,
    pub raw_queries: usize,
    pub canonical_facts: usize,
    pub covered_facts: usize,
    pub timeline_candidates: usize,
}

/// Selective visual production view over the complete semantic inventory.
///
/// This is the Semantic Consolidation v1 output: the raw manifest answers
/// "what does the text contain?"; this section answers "which compositions are
/// worth producing, and what is still uncovered?".
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct VisualConsolidation {
    pub schema: String,
    pub inventory: ConsolidationInventory,
    pub production_candidates: Vec<ProductionCandidate>,
    pub facts: Vec<ConsolidatedFact>,
    pub dates: Vec<ConsolidatedDate>,
    pub reference_numbers: Vec<ReferenceNumber>,
    pub event_links: Vec<ConsolidatedEventLink>,
    pub scenes: Vec<ConsolidationSceneStats>,
    pub metrics: ConsolidationMetrics,
}

impl Default for VisualConsolidation {
    fn default() -> Self {
        Self {
            schema: VISUAL_CONSOLIDATION_SCHEMA.to_string(),
            inventory: ConsolidationInventory::default(),
            production_candidates: Vec::new(),
            facts: Vec::new(),
            dates: Vec::new(),
            reference_numbers: Vec::new(),
            event_links: Vec::new(),
            scenes: Vec::new(),
            metrics: ConsolidationMetrics::default(),
        }
    }
}

/// Every fact and proposal extracted for one scene.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SceneManifest {
    pub scene_id: String,
    /// `valid`, or `rejected` when the extractor refused the scene.
    pub status: String,
    pub reason: Option<String>,
    pub entities: Vec<Candidate>,
    /// Canonical identities resolved across the whole script.
    pub identities: Vec<CanonicalIdentity>,
    /// Grounded titles attached only to a nearby, same-sentence PERSON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub person_roles: Vec<PersonRole>,
    /// Explicit ages grounded to an unambiguous person in the same sentence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub age_facts: Vec<PersonAgeFact>,
    /// Explicitly stated relations between grounded people.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub person_relations: Vec<PersonRelation>,
    /// Exact source-grounded place mentions with optional curated links.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<Place>,
    /// Explicit within-scene routes grounded in linked place mentions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<Route>,
    /// Relations split into co_mentions (grouping signal) and asserted ones.
    pub relations: RelationView,
    pub numeric_facts: Vec<NumericFact>,
    /// Quantities with a recognized unit and exact source evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quantities: Vec<crate::quantity::Quantity>,
    /// Comparisons between exact quantities with compatible dimensions,
    /// units, and (for currency) explicit matching periods.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quantity_comparisons: Vec<crate::quantity::QuantityComparison>,
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
    /// Grounded multi-sentence passages (2–5 sentences) at linguistic
    /// boundaries, for planners that treat a passage as one editorial unit.
    pub important_segments: Vec<ImportantSegment>,
    /// Multi-component image queries with per-component provenance. Every
    /// query stays a literal source substring.
    pub composed_image_queries: Vec<ComposedImageQuery>,
    /// Compact image queries derived only from explicit source-grounded
    /// subject-to-location facts; their evidence and components remain literal.
    pub contextual_image_queries: Vec<ContextualImageQuery>,
    pub visual_proposals: Vec<VisualProposal>,
    /// Grounded multi-item structures suitable for planner selection.
    pub semantic_groups: Vec<SemanticGroup>,
    /// Fact-backed micro phrases and their comparison/sequence projections.
    pub micro_summaries: Vec<MicroSummary>,
    /// Candidates dropped by the grounding check (never silently lost).
    pub rejected_items: usize,
}

/// The whole-script manifest: one request, one response, one source of truth.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SemanticManifest {
    pub schema_version: String,
    pub language: String,
    /// FNV-1a 64 over the canonical request (language + scenes + analyzer
    /// versions). Equal inputs give equal fingerprints (idempotent replay).
    pub fingerprint: String,
    pub analyzers: AnalyzerVersions,
    pub scenes: Vec<SceneManifest>,
    /// Conservative multi-hop journeys grounded across consecutive scenes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cross_scene_routes: Vec<CrossSceneRoute>,
    /// Grounded event mentions; cross-scene matching is kept separate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<crate::event::EventFrame>,
    /// Additive v3 entity projection: linked ids, roles, glosses, exclusions
    /// and typed retrieval intents. Empty when decoding a v2 manifest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<LinkedEntity>,
    /// Grounded identities, facts and dates joined across scene boundaries.
    pub story_graph: StoryGraph,
    /// Which grounded facts a scene's proposals already represent.
    pub visual_coverage: VisualCoverage,
    /// Additive entity-continuity projection. Older manifests decode with an
    /// empty projection; older consumers may ignore it.
    #[serde(default)]
    pub discourse: DiscourseProjection,
    /// Selective production view: canonical facts, deduplicated candidates,
    /// query budgets and explicit coverage reasons.
    pub visual_consolidation: VisualConsolidation,
    /// Asset-search handshake: query ladders plus the planner's outcomes.
    pub search_feedback: SearchFeedback,
    /// Optional whole-script editorial products computed in this same batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editorial: Option<EditorialManifest>,
}

/// Bounded editorial projection carried by the semantic manifest. Internal
/// ranking graphs are intentionally omitted; only products consumed by Go
/// planning and their stage timings cross the worker boundary.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// Optional ISO `YYYY-MM-DD` reference used only by deictic relatives
    /// (`ieri`, `3 years ago`). Absent means those facts stay unresolved; the
    /// engine never infers the current date from a clock.
    #[serde(default)]
    pub reference_date: Option<String>,
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
