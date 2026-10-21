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
//
// In addition to the top-N visual entities, `analyze` projects the
//! `temporal_facts` view: every source-grounded date occurrence, with
//! calendar normalization and precision, regardless of the visual top-N
//! budget (see `temporal.rs`).

#![deny(unsafe_code)]

pub mod analysis;
pub mod extractor;
pub mod spellout;
pub mod temporal;
pub mod types;

pub use analysis::{analyze, Analysis};
pub use extractor::extract;
pub use spellout::supported_spellout_language_count;
pub use temporal::temporal_facts;
pub use temporal::TemporalFact;
pub use temporal::{PRECISION_DAY, PRECISION_MONTH, PRECISION_UNRESOLVED, PRECISION_YEAR};
pub use types::{ExtractOptions, VisualEntity};
