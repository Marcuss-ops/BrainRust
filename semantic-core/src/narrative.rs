//! `discourse.v1` — additive narrative-continuity projection.
//!
//! This is not a second entity extractor and it is not a calibrated model. It
//! records what the deterministic resolver could prove from the source:
//! explicit mentions, pronouns and role nominals (resolved, ambiguous or
//! unresolved), entity threads, and scene-level entity continuity. Every
//! reference keeps the exact mention span and the sentence that grounds it.
//!
//! The projection is additive to `semantic_manifest.v2`; older consumers can
//! ignore it, and older manifests decode with an empty projection.

use serde::{Deserialize, Serialize};

/// Wire version of the narrative projection.
pub const DISCOURSE_SCHEMA: &str = "discourse.v1";

/// One source occurrence that refers to a subject.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DiscourseReference {
    /// Stable id in source order (`dref_0001`, ...).
    pub id: String,
    pub scene_id: String,
    pub sentence_index: usize,
    /// Exact source slice at `[start_byte, end_byte)`.
    pub mention: String,
    /// `explicit_name`, `surname`, `pronoun` or `nominal_reference`.
    #[serde(rename = "type")]
    pub reference_type: String,
    /// `resolved`, `ambiguous` or `unresolved`.
    pub status: String,
    /// Canonical identity ids the reference resolves to. Empty unless
    /// `status = resolved`; a plural reference may carry more than one.
    #[serde(default)]
    pub target_entity_ids: Vec<String>,
    /// Plausible antecedents when the evidence is insufficient. Empty for a
    /// resolved or clearly unresolved reference.
    #[serde(default)]
    pub candidate_entity_ids: Vec<String>,
    /// Deterministic rule that resolved the reference. This is a rule name,
    /// never a probability or a provider confidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_basis: Option<String>,
    /// Why the reference stayed ambiguous or unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Verbatim sentence containing the reference.
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub sentence_start_byte: usize,
    pub sentence_end_byte: usize,
}

/// One point where an entity continues across the narrative.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ThreadBeat {
    pub reference_id: String,
    pub scene_id: String,
    pub sentence_index: usize,
    /// `starts`, `continues` or `returns`.
    pub state: String,
}

/// A single entity's continuity chain. Only entities with at least two
/// resolved references are emitted: a one-off mention is already an entity,
/// not a thread.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EntityThread {
    pub entity_id: String,
    pub canonical_surface: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    /// Scenes in which the thread has a resolved beat, in request order.
    pub scene_ids: Vec<String>,
    pub first_scene_id: String,
    pub last_scene_id: String,
    pub mention_count: usize,
    /// Number of times the entity reappears after at least one full sentence
    /// without a resolved beat.
    pub return_count: usize,
    pub beats: Vec<ThreadBeat>,
}

/// Entity-based continuity between a scene and the previous one.
///
/// `score` is the Jaccard share of resolved entity ids shared by the two
/// scenes. It is a deterministic overlap measure, not a probability that the
/// topic is the same.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SceneContinuity {
    pub scene_id: String,
    /// `start`, `continues`, `shift` or `empty`.
    pub status: String,
    pub score: f32,
    pub active_entity_ids: Vec<String>,
    pub shared_entity_ids: Vec<String>,
    pub entered_entity_ids: Vec<String>,
    pub exited_entity_ids: Vec<String>,
}

/// Aggregate counts, useful for planner-side budgeting without rescanning.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct DiscourseCounts {
    pub total: usize,
    pub resolved: usize,
    pub ambiguous: usize,
    pub unresolved: usize,
    pub threads: usize,
    pub cross_scene_threads: usize,
}

/// The whole-script narrative projection.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DiscourseProjection {
    pub schema: String,
    #[serde(default)]
    pub references: Vec<DiscourseReference>,
    #[serde(default)]
    pub threads: Vec<EntityThread>,
    #[serde(default)]
    pub scene_continuity: Vec<SceneContinuity>,
    #[serde(default)]
    pub counts: DiscourseCounts,
}

impl Default for DiscourseProjection {
    fn default() -> Self {
        Self {
            schema: DISCOURSE_SCHEMA.to_string(),
            references: Vec::new(),
            threads: Vec::new(),
            scene_continuity: Vec::new(),
            counts: DiscourseCounts::default(),
        }
    }
}
