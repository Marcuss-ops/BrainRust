//! Native execution components for PipelineGen.
//!
//! This crate is intentionally an empty boundary at the start of the Rust
//! migration. Components added here may perform CPU-heavy or process-level
//! work, but must not become an alternate source of canonical state.
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

#[cfg(test)]
mod tests {
    use super::COMPONENT;

    #[test]
    fn component_name_is_stable() {
        assert_eq!(COMPONENT, "pipelinegen-muscles");
    }
}
