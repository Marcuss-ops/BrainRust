//! Wire types for `imagecatalog.v1` / `image_plan.v1`.
//!
//! The catalog types intentionally use the column names of the existing
//! PipelineGen tables, including aliases for the historical `file_hash` and
//! `legacy_file_md5` spellings of the content hash. The snapshot is produced
//! by PipelineGen (or by `tools/export_image_catalog.py`); this crate never
//! opens the database itself.

use semantic_core::SceneInput;
use serde::{Deserialize, Serialize};

/// Envelope version accepted by the worker.
pub const REQUEST_SCHEMA: &str = "imagecatalog.v1";
/// Version of the returned image plan.
pub const PLAN_SCHEMA: &str = "image_plan.v1";

/// One worker request: script text plus the current image-catalog snapshot.
#[derive(Debug, Clone, Deserialize)]
pub struct ImageCatalogRequest {
    /// Envelope version (`imagecatalog.v1`); absent is accepted for
    /// compatibility with simple callers.
    #[serde(default)]
    pub version: Option<String>,
    /// Operation (`plan`); absent defaults to `plan`.
    #[serde(default)]
    pub operation: Option<String>,
    /// BCP-47 language tag of every scene.
    pub language: String,
    /// Script scenes in request order.
    pub scenes: Vec<SceneInput>,
    /// Existing durable image-catalog rows. Empty is valid: every grounded
    /// query then becomes a catalog miss.
    #[serde(default)]
    pub catalog: CatalogSnapshot,
    /// Optional deterministic clock (`YYYY-MM-DDTHH:MM:SSZ`). When supplied,
    /// freshness classification replays identically; when absent the worker
    /// uses the current UTC time.
    #[serde(default)]
    pub now: Option<String>,
}

/// Snapshot of the three existing `entity_image_catalog_*` tables.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct CatalogSnapshot {
    #[serde(default)]
    pub entities: Vec<CatalogEntity>,
    #[serde(default)]
    pub candidates: Vec<CatalogCandidate>,
    #[serde(default)]
    pub materializations: Vec<CatalogMaterialization>,
    /// Semantic sidecar for existing candidates. It does not create a second
    /// catalog: annotations are keyed by `candidate_id` and merged into this
    /// snapshot by the read-only exporter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<CatalogAnnotation>,
}

/// Existing `entity_image_catalog_entities` row.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CatalogEntity {
    pub canonical_entity_id: String,
    /// The current durable catalog is PERSON-only; the field is retained so a
    /// future catalog extension does not change the wire shape.
    #[serde(default = "default_entity_type")]
    pub entity_type: String,
    #[serde(default)]
    pub canonical_name: String,
}

/// Existing `entity_image_catalog_candidates` row.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CatalogCandidate {
    pub candidate_id: i64,
    pub canonical_entity_id: String,
    #[serde(default)]
    pub provider: String,
    /// Provider rank, 1-based. Zero is treated as 1 by the ranking score.
    #[serde(default = "default_rank")]
    pub rank: u32,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub source_page_url: String,
    #[serde(default)]
    pub thumbnail_url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// `fresh`, `active` (legacy), `stale`, `broken` or `retired`.
    #[serde(default = "default_candidate_status")]
    pub status: String,
    /// `unknown`, `accepted` or `rejected`. Missing/legacy values fail closed:
    /// an uncertified row is never selected.
    #[serde(default = "default_semantic_status")]
    pub semantic_status: String,
    #[serde(default)]
    pub semantic_score: f64,
    #[serde(default)]
    pub technical_score: f64,
    #[serde(default)]
    pub quality_reason: String,
    /// Last successful catalog observation. Empty keeps the stored status.
    #[serde(default)]
    pub last_seen_at: String,
}

/// Existing `entity_image_catalog_materializations` row.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CatalogMaterialization {
    pub candidate_id: i64,
    #[serde(default)]
    pub asset_id: String,
    /// Current column name. The aliases keep snapshots from the pre-286
    /// schema readable without inventing a second field.
    #[serde(default, alias = "file_hash", alias = "legacy_file_md5")]
    pub content_sha256: String,
    #[serde(default)]
    pub drive_file_id: String,
    #[serde(default)]
    pub drive_link: String,
    /// `pending`, `materialized` or `failed`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub materialized_at: String,
    #[serde(default)]
    pub last_verified_at: String,
}

/// Semantic description and identity evidence for one existing candidate.
///
/// `subjects` are canonical entity ids. When non-empty they must include the
/// candidate's `canonical_entity_id`; `identity_status = "mismatch"` is the
/// explicit exception for a catalog row whose image shows another subject or
/// no person at all. The policy in `catalog.rs` rejects both mismatches and
/// synthetic/illustration depictions for identity queries.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct CatalogAnnotation {
    pub candidate_id: i64,
    /// Human-readable description of what is visible in the image.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Canonical identities visible in the image (including the primary one
    /// when it is actually depicted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    /// Free-form editorial tags: actions, places, composition, provenance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `photograph`, `composite`, `illustration`, `synthetic`, `unknown`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub depiction: String,
    /// `verified_visual`, `catalog_claimed`, `unverified`, `mismatch`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub identity_status: String,
    /// `reviewed`, `unverified` or `pending`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub review_status: String,
    /// Who produced the annotation (`visual_review_2026-11-10`,
    /// `media_assets.metadata_json.description`, ...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Source evidence (media asset id, URL, review note).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evidence: String,
}

/// The complete deterministic image plan for one script.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImagePlan {
    pub schema_version: &'static str,
    pub language: String,
    /// Semantic-manifest fingerprint. Equal text and equal analyzer versions
    /// produce the same value.
    pub fingerprint: String,
    /// FNV-1a over the normalized catalog snapshot. Freezes the catalog state
    /// the decisions were made against.
    pub catalog_fingerprint: String,
    pub scenes: Vec<SceneImagePlan>,
    pub counts: ImagePlanCounts,
}

/// Image decisions for one scene, in manifest order.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SceneImagePlan {
    pub scene_id: String,
    pub queries: Vec<ImageQueryPlan>,
}

/// One grounded image opportunity and its catalog decision.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageQueryPlan {
    /// The grounded query exactly as emitted by the semantic engine. Entity
    /// queries are source slices; contextual/composed queries retain their
    /// own evidence field.
    pub query: String,
    /// `entity_search`, `contextual_image_query` or `composed_image_query`.
    pub query_kind: String,
    /// Semantic proposal id for an entity query, absent for composed and
    /// contextual queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    /// Canonical entity ids carried by the proposal (empty for a contextual
    /// or composed query).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_ids: Vec<String>,
    /// Canonical catalog identity resolved for a PERSON query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Verbatim source evidence for the query.
    pub evidence: String,
    pub decision: ImageDecision,
    /// Human-readable explanation of the decision, including the first
    /// rejection reason when no candidate is eligible.
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<ImageSelection>,
    /// The assessed catalog candidates, best first, capped for auditability.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ImageCandidateAssessment>,
}

/// Bounded decision vocabulary. There is no "force a match" outcome.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageDecision {
    /// A usable catalog candidate was selected for this grounded query.
    Selected,
    /// The query is catalog-eligible, but the catalog has no usable candidate.
    /// The caller keeps its existing external retrieval path.
    CatalogMiss,
    /// The current catalog cannot represent this query (non-PERSON identity,
    /// contextual or composed query). The caller keeps its existing path.
    NotCatalogEligible,
}

/// A selected catalog candidate, frozen in the plan.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageSelection {
    pub candidate_id: i64,
    /// Present only when a verified materialization backs the selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// SHA-256 of the materialized bytes, when verified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drive_link: Option<String>,
    /// True when the selection can be rendered from a verified durable asset
    /// without re-downloading the remote URL.
    pub materialized: bool,
    pub source_url: String,
    pub provider: String,
    pub provider_rank: u32,
    pub width: u32,
    pub height: u32,
    /// Effective state: `fresh` or `stale` (broken/retired rows are only
    /// selectable through a verified materialization).
    pub state: String,
    pub semantic_score: f64,
    pub technical_score: f64,
    /// Deterministic ranking score, not a probability.
    pub score: f64,
    /// True when another scene in this script already selected the candidate.
    pub reused: bool,
    /// Visible description supplied by the semantic sidecar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Canonical identities visible in the selected image.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depiction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>,
}

/// Assessment of one catalog candidate for one query.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageCandidateAssessment {
    pub candidate_id: i64,
    pub state: String,
    pub eligible: bool,
    pub selected: bool,
    /// Deterministic ranking score; 0.0 when not eligible.
    pub score: f64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_status: Option<String>,
}

/// Plan counters for callers and dashboards.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ImagePlanCounts {
    pub scenes: usize,
    pub queries: usize,
    pub selected: usize,
    pub catalog_misses: usize,
    pub not_catalog_eligible: usize,
    pub reused_assets: usize,
}

fn default_entity_type() -> String {
    "PERSON".to_string()
}

fn default_rank() -> u32 {
    1
}

fn default_candidate_status() -> String {
    "fresh".to_string()
}

fn default_semantic_status() -> String {
    "unknown".to_string()
}
