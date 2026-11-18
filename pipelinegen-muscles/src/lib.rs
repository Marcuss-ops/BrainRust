//! Native execution components for PipelineGen.
//!
//! This crate owns capability-scoped media execution for the Rust migration.
//! Components here may perform CPU-heavy or process-level work, but must not
//! become an alternate source of canonical state.
//!
//! Initial boundary rules:
//! - SQLite remains owned by the Go application.
//! - Transactional outbox decisions remain owned by the Go application.
//! - Google Drive and external vector-store access must arrive through explicit contracts.
//! - FFmpeg invocation belongs behind a typed execution interface.
//! - No direct credential or environment-file loading from this crate.

#![deny(unsafe_code)]

mod artifact;
mod audio_copy;
mod config;
mod cut;
mod dispatcher;
mod encoder;
pub mod error;
#[cfg(test)]
mod golden;
mod gpu;
pub mod hash;
mod manifest_projection;
mod parallel;
mod probe;
mod process;
mod protocol;
mod render_audio;
// Compiled plans are contract inputs, not executor internals: the request DTO
// holds them, so they must sit *below* `protocol` in the module graph. They
// were compiled as submodules of the executors, which made the wire contract
// depend on the implementation and closed a module cycle.
mod render_audio_plan;
mod render_stock;
mod render_stock_canonical_plan;
mod transform;

pub use dispatcher::{process, run_stdio};
// The wire request type is reusable outside the crate (Go drives it, but
// benches and certification tooling must build the SAME type, not a second
// untyped shape that could drift).
pub use error::{ErrorCode, MuscleError};
pub use protocol::Request;
