//! The deterministic VisualNER extraction pipeline (V1):
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
//! V1 is deterministic: NO ONNX, NO LLM, NO GPU. The killer rule is
//! `NO EVIDENCE → NO ENTITY`: every returned entity must be a verbatim
//! substring of the source text, with byte offsets proving the evidence.
//!
//! The pipeline is split into focused submodules; every item is a pure move
//! from the former single file, with no behavior change.

mod location;
mod model;
mod names;
mod numeric;
mod phrases;
mod pipeline;
mod pos;
mod ranking;
mod stopwords;
mod tokenizer;
mod validation;
mod values;
mod verb;

pub use model::model_path;
pub use names::is_heading_surface;
pub use numeric::{numeric_value_spans, NumericSpan};
pub(crate) use pipeline::extract_with_context_and_spellout;
pub use pipeline::{extract, extract_with_context, script_person_tokens, ScriptContext};

#[cfg(test)]
pub(crate) use tokenizer::tokenize;

// ── Stopword + blocklist tables ──────────────────────────────────────
//
// V1 uses curated, dependency-free tables. The tables are intentionally
// small and explicit so the extractor stays deterministic and auditable.
// Adding a word here is a one-line change; no model retraining.

#[cfg(test)]
#[path = "extractor_tests/mod.rs"]
mod tests;
