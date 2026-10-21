//! Temporal facts are centralized in `semantic-core::dates`.
//!
//! This module is a thin re-export so `visualner` keeps its public API while a
//! single, deterministic date normalizer serves every extractor. The previous
//! in-crate copy lived here; it moved to `semantic-core` so `scene_intelligence`
//! and the semantic worker consume the SAME facts instead of re-parsing dates
//! on an independent path. No behavior change: same signatures, same tests
//! (they now run in `semantic-core`).
pub use semantic_core::dates::*;
