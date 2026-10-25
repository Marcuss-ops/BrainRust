//! The semantic MediaSampler pipeline (Fase 4):
//!
//! ```text
//! candidate
//!  ↓
//! ownership validation
//!  ↓
//! duplicate/reuse validation
//!  ↓
//! subject relevance
//!  ↓
//! action relevance
//!  ↓
//! context relevance
//!  ↓
//! semantic score
//!  ↓
//! diversity
//!  ↓
//! final score
//! ```
//!
//! The sampler does NOT pick the highest generic vector similarity. It
//! applies hard constraints first (ownership, reuse, subject match), then a
//! semantic score, then diversity. The LIVE-test bug where Artlist returned
//! a boxing clip for Greek Salad is rejected at the SUBJECT RELEVANCE stage.
//!
//! Split into focused submodules; every item is a pure move from the former
//! single file, with no behavior change.

mod bound;
mod pipeline;
mod scoring;
mod tokens;

#[cfg(test)]
mod tests;

pub use bound::BoundAssets;
pub use pipeline::{sample_scene, sample_scene_group};
