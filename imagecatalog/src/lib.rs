//! `imagecatalog` — the image-only association planner for PipelineGen.
//!
//! The module takes script text and a snapshot of the **existing**
//! `entity_image_catalog_entities` / `_candidates` / `_materializations`
//! tables plus an optional semantic annotation sidecar keyed by
//! `candidate_id`, runs the canonical semantic engine to obtain grounded
//! entity queries, and returns one deterministic image decision per query.
//!
//! It deliberately does not:
//!
//! - open PipelineGen's SQLite database or load credentials (the Go
//!   composition root owns the database; the snapshot is passed in);
//! - create a second image catalog or a new vector database;
//! - download, materialize, render or search the internet;
//! - handle video, stock clips or sound effects. This crate is images only.
//!
//! The selection policy mirrors the existing catalog rules: semantic status
//! `accepted`, the technical score and dimension gates, the seven-day
//! fresh/stale boundary, and verified materializations. A durable Drive asset
//! remains reusable even when its original remote URL is `broken`.
//!
//! The returned `score` is a deterministic ranking score, not a calibrated
//! probability that the image is editorially correct.

#![deny(unsafe_code)]

mod canonical;
mod catalog;
mod clock;
mod plan;
pub mod types;

pub use catalog::{AssessedCandidate, CatalogIndex};
pub use plan::{plan, plan_from_manifest, plan_text};
pub use types::{
    CatalogAnnotation, CatalogCandidate, CatalogEntity, CatalogMaterialization, CatalogSnapshot,
    ImageCandidateAssessment, ImageCatalogRequest, ImageDecision, ImagePlan, ImagePlanCounts,
    ImageQueryPlan, ImageSelection, SceneImagePlan, PLAN_SCHEMA, REQUEST_SCHEMA,
};
