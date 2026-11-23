use super::*;

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

pub(super) fn unknown_gender() -> String {
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
    /// Additive `ChartsPack.hpp` qualifier (`exact`, `approximate`,
    /// `greater_than`, `forecast`, `hypothetical`, `range`, `missing`) when
    /// the grounded quantity carries one. Absent for legacy items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    /// Grounded period (`2023`, `2023-Q2`, ...) when the quantity has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    /// Grounded inclusive lower bound for an explicit range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower: Option<f64>,
    /// Grounded inclusive upper bound for an explicit range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper: Option<f64>,
    /// Quantity range id when the value is one endpoint of an explicit range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_id: Option<String>,
}

/// Chart-level metadata for downstream renderers. Every field is derived from
/// grounded facts; absent values stay `None` instead of being guessed.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ChartMetadata {
    /// Repeated metric predicate (`revenue`, `sales`, ...) when grounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    /// Unit code (`EUR`, `km`, `PCT`, ...) when grounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Quantity dimension (`currency`, `percent`, `count`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimension: Option<String>,
    /// Shared period (`2023`) when every point carries the same grounded year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
    /// `time` for ordered date labels, `category` for named categories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_type: Option<String>,
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
    /// Structured chart metadata when the values are unit/metric grounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_meta: Option<ChartMetadata>,
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
