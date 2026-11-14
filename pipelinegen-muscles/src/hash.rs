//! The crate-wide canonical file digest.
//!
//! One implementation owns the sha256sum contract (lowercase hex, exactly 64
//! characters) for execution, canonical-plan validation and certification
//! benches. The subprocess is deliberate: the Go producer and the
//! RenderingGen profiler compute the same digest with the same CLI, so the
//! contract is the tool's output rather than a second Rust implementation
//! that could drift.
use crate::error::MuscleError;
use std::process::Command;

/// Computes the lowercase SHA256 digest of `path` through `sha256sum`.
pub fn sha256_file(path: &str) -> Result<String, MuscleError> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| MuscleError::generic(format!("compute SHA256 for {path}: {error}")))?;
    if !output.status.success() {
        return Err(MuscleError::generic(format!(
            "compute SHA256 for {path} failed"
        )));
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if digest.len() != 64
        || !digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(MuscleError::generic(format!(
            "sha256sum returned an invalid digest for {path}"
        )));
    }
    Ok(digest)
}
