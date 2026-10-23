//! # semantic-core — shared semantic extraction base
//!
//! The single home for the semantic algorithms that every extractor shares.
//! Rust's mission: analyse a script in full and return every editorial and
//! visual *candidate* with identity, evidence and priority consistent.
//! PipelineGen selects and orchestrates; Chronon renders. This crate never
//! touches FFmpeg, GPU, compositing or RenderingGen — it is pure text
//! analysis with deterministic, source-grounded output.
//!
//! Layering (no cycles):
//!
//! ```text
//! semantic-core   (this crate: numbers, dates, identity, relations, manifest)
//!   ├── visualner         (entity tokenizer)   -> re-exports dates/numbers
//!   ├── pipelinegen-muscles (phrase/scene)     -> re-exports dates/numbers
//!   └── semantic-engine   (the NDJSON worker)  -> builds the manifest
//! ```
//!
//! Unification here means ONE numeric parser, ONE date normalizer, ONE
//! identity registry and ONE relation vocabulary — not one giant file. Each
//! collector keeps its tokenizer-specific span detection; the parse and the
//! typed projection live here so two paths can never disagree about a value.

#![deny(unsafe_code)]

pub mod bullets;
pub mod dates;
mod generated_language_packs;
mod generated_legal_packs;
pub mod grounding;
pub mod identity;
pub mod language_packs;
pub mod legal_packs;
pub mod lexicon;
pub mod manifest;
pub mod numbers;
pub mod phrase_impact;
pub mod relations;
pub mod sentences;

pub use bullets::{BulletCandidate, BulletSignal, SignalKind};
pub use dates::{temporal_facts, TemporalFact};
pub use identity::{CanonicalIdentity, IdentityRegistry, Mention};
pub use manifest::ChartItem;
pub use manifest::{
    AnalyzerVersions, Candidate, EntityFact, MicroSummary, SceneInput, SceneManifest,
    ScriptRequest, SemanticGroup, SemanticGroupMember, SemanticManifest, VisualProposal,
    SCHEMA_VERSION,
};
pub use numbers::{parse_unambiguous_value, values_are_comparable, NumericFact};
pub use relations::{AssertedRelation, CoMention, RelationStatus, RelationView};
pub use sentences::{split_sentences, SentenceSpan};
