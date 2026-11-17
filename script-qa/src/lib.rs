//! Editorial quality gate for PipelineGen scripts.
//!
//! This crate is a pure evaluator: it reads a resolved plan and a generation
//! result, computes the canonical `GenerationQuality` metrics, and returns the
//! ordered failure reasons. It owns no database, provider, filesystem or
//! clock state.
//!
//! The wire entry point is `protocol::handle_line`; the persistent
//! `semantic-engine` worker dispatches `quality_gate` to it.

#![deny(unsafe_code)]

pub mod dates;
pub mod gate;
pub mod model;
pub mod protocol;
pub mod tokenize;

pub use gate::evaluate;
pub use model::{
    ArtifactsInput, ClipBindingInput, ClipEvidenceInput, EntitiesInput, EntityInput, Evaluation,
    GateRequest, ItemInput, OutputInput, PlanInput, Quality, ResearchCandidateInput,
    ResearchEvidenceInput, ResultInput, SceneBindingsInput, SceneInput, SegmentInput,
    SpecSceneInput,
};
