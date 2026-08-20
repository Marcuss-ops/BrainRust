//! Native execution components for PipelineGen.
//!
//! This crate owns capability-scoped media execution for the Rust migration.
//! Components here may perform CPU-heavy or process-level work, but must not
//! become an alternate source of canonical state.
//!
//! Initial boundary rules:
//! - SQLite remains owned by the Go application.
//! - Transactional outbox decisions remain owned by the Go application.
//! - Google Drive and Qdrant access must arrive through explicit contracts.
//! - FFmpeg invocation belongs behind a typed execution interface.
//! - No direct credentials or environment-file loading from this crate.

#![deny(unsafe_code)]

/// Stable placeholder proving that the crate is wired and testable.
pub const COMPONENT: &str = "pipelinegen-muscles";

mod admin_media;
mod artifact;
mod config;
mod cut;
mod dispatcher;
mod encoder;
#[cfg(test)]
mod golden;
mod probe;
mod process;
mod protocol;
mod render_audio;
mod render_clip;
mod render_stock;
mod transform;
mod native;

pub use dispatcher::{process, run_stdio};
