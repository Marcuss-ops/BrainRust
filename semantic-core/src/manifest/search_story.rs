//! Search ladders, grounded query components and whole-script graph DTOs.

use serde::{Deserialize, Serialize};

/// One grounded component inside a composed image query: the exact span of a
/// subject, object, measurement or date the query is composed of.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct QueryComponent {
    /// `entity`, `related_entity`, `location`, `co_mentioned_entity`, `measurement` or `date`.
    pub kind: String,
    /// The closed source type the component carries (`PERSON`, `LOCATION`,
    /// `NUMBER`, `PERCENT`, `MONEY`, `DATE`, ...).
    pub component_type: String,
    /// Verbatim source slice at `[start_byte, end_byte)`.
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A ready-to-use image query composed of several grounded components that
/// the source states together, with every component's own span as
/// provenance. The literal-substring rule is kept: the query text is a
/// verbatim source slice spanning the components, never a synthetic join of
/// surfaces the source never wrote.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ComposedImageQuery {
    pub id: String,
    /// Always `composed_image_query`.
    pub kind: String,
    /// Verbatim source slice spanning the components.
    pub query: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub sentence_index: usize,
    /// Grounded components the query is composed of (never fewer than two).
    pub components: Vec<QueryComponent>,
}

/// A contextual image query composed from visual entities joined by an
/// asserted relation or a same-sentence co-mention. A co-mention is only a
/// retrieval cue; it does not assert a real-world relationship. Unlike
/// `ComposedImageQuery`, `query` is not a source substring: each word comes
/// from provenance components and the exact sentence remains attached.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ContextualImageQuery {
    pub id: String,
    /// Always `contextual_image_query`.
    pub kind: String,
    /// A compact provider query composed from the grounded related entities.
    pub query: String,
    /// Exact sentence evidence for the relation or co-mention.
    pub evidence: String,
    pub evidence_start_byte: usize,
    pub evidence_end_byte: usize,
    pub sentence_index: usize,
    /// The grounded entity components; `kind` distinguishes relation targets
    /// from same-sentence co-mentions.
    pub components: Vec<QueryComponent>,
}

/// One rung of the asset-search ladder proposed by the engine. Literal queries
/// preserve a source mention; contextual queries combine grounded components
/// and carry the sentence that supports the combination.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SearchQuery {
    /// `exact_mention` or `context_phrase`.
    pub kind: String,
    pub text: String,
    /// `literal`, `derived_from_source_context` or `derived_from_semantic_template`.
    pub origin: String,
    /// True when the query searches for a generic illustrative asset rather
    /// than an image of a specific real-world entity.
    pub illustrative: bool,
    /// Exact sentence or mention that grounds the query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_start_byte: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_end_byte: Option<usize>,
    /// Exact source spans combined into `text`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<QueryComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_terms: Vec<String>,
}

impl SearchQuery {
    /// A literal query that is itself a source surface.
    pub fn literal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: "exact_mention".to_string(),
            origin: "literal".to_string(),
            illustrative: false,
            evidence: None,
            evidence_start_byte: None,
            evidence_end_byte: None,
            components: Vec::new(),
            template_id: None,
            derived_terms: Vec::new(),
        }
    }

    /// A query assembled from exact source components in one evidence span.
    pub fn contextual(
        text: impl Into<String>,
        evidence: impl Into<String>,
        start_byte: usize,
        end_byte: usize,
        components: Vec<QueryComponent>,
        illustrative: bool,
    ) -> Self {
        Self {
            text: text.into(),
            kind: "context_phrase".to_string(),
            origin: "derived_from_source_context".to_string(),
            illustrative,
            evidence: Some(evidence.into()),
            evidence_start_byte: Some(start_byte),
            evidence_end_byte: Some(end_byte),
            components,
            template_id: None,
            derived_terms: Vec::new(),
        }
    }

    /// A clearly illustrative query expanded with a vocabulary template.
    pub fn semantic_context(
        text: impl Into<String>,
        evidence: impl Into<String>,
        start_byte: usize,
        end_byte: usize,
        components: Vec<QueryComponent>,
        template_id: impl Into<String>,
        derived_terms: Vec<String>,
    ) -> Self {
        Self {
            text: text.into(),
            kind: "context_phrase".to_string(),
            origin: "derived_from_semantic_template".to_string(),
            illustrative: true,
            evidence: Some(evidence.into()),
            evidence_start_byte: Some(start_byte),
            evidence_end_byte: Some(end_byte),
            components,
            template_id: Some(template_id.into()),
            derived_terms,
        }
    }
}

/// What the planner should do when a search ladder is exhausted. Only the
/// planner can set this: the engine never sees a provider response.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct SearchFeedback {
    pub entries: Vec<SearchFeedbackEntry>,
}

/// One node of the whole-script story graph.
///
/// A node is either a canonical identity (`kind = "identity"`) or a grounded
/// fact (`kind = "fact"`). Identity nodes carry every scene they are mentioned
/// in, which is what makes a cross-scene link explicit instead of implicit.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct StoryGraph {
    pub nodes: Vec<StoryNode>,
    pub edges: Vec<StoryEdge>,
}

/// One fact evaluated for visual representation.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SceneCoverage {
    pub scene_id: String,
    /// Facts ordered by importance, then source position.
    pub facts: Vec<CoverageItem>,
}

/// Whole-script visual coverage report. PipelineGen reads this to know which
/// facts still need an image, a card or a chart instead of guessing.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct VisualCoverage {
    pub total_facts: usize,
    pub covered_facts: usize,
    pub scenes: Vec<SceneCoverage>,
}
