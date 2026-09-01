//! # mediasampler — semantic asset sampler (Fase 4)
//!
//! Ranks candidate assets for one scene through a hard-constraint +
//! soft-scoring pipeline and is the deterministic replacement for the
//! LIVE-test bug where Artlist returned a boxing clip for the Greek Salad
//! segment.
//!
//! The sampler does NOT pick the highest generic vector similarity. It
//! applies hard constraints first (ownership, reuse, subject match), then
//! a semantic score, then diversity. The pipeline is:
//!
//! ```text
//! candidate
//!  ↓ ownership validation
//!  ↓ duplicate/reuse validation
//!  ↓ subject relevance
//!  ↓ action relevance
//!  ↓ context relevance
//!  ↓ semantic score
//!  ↓ diversity
//!  ↓ final score
//! ```
//!
//! Deterministic: same input → same winner 100/100 runs. Cross-scene: when
//! `allow_reuse = false`, a candidate already bound to another scene is
//! rejected with `AlreadyBound`.

#![deny(unsafe_code)]

pub mod sampler;
pub mod types;

pub use sampler::{sample_scene, BoundAssets};
pub use types::{Candidate, RejectionReason, SampleOptions, SampleResult, Scene};
