//! # visualner — deterministic VisualEntity extractor (V1)
//!
//! Fase 3 of the VidRush semantic-correctness plan. V1 is deterministic:
//! NO ONNX, NO LLM, NO GPU. The killer rule is
//! `NO EVIDENCE → NO ENTITY`: every returned [`VisualEntity`] is a
//! verbatim substring of the source text, with byte offsets proving the
//! evidence.
//!
//! The pipeline is:
//!
//! ```text
//! text
//!  ↓
//! tokenize
//!  ↓
//! noun/noun-phrase candidates
//!  ↓
//! stop phrases
//!  ↓
//! visualness scoring
//!  ↓
//! source evidence validation
//!  ↓
//! rank
//!  ↓
//! top N
//! ```
//!
//! The extractor is the deterministic replacement for the LIVE-test
//! hallucinations ("Imagine the", "ready", "Mediterranean" as entities).
//! It feeds the SceneIR `Entities` field and the MediaSampler candidate
//! ranking.

#![deny(unsafe_code)]

pub mod extractor;
pub mod types;

pub use extractor::extract;
pub use types::{ExtractOptions, VisualEntity};
