//! The crate-wide canonical file digest.
//!
//! One implementation owns the sha256sum contract (lowercase hex, exactly 64
//! characters) for execution, canonical-plan validation, extradata
//! fingerprints and certification benches. The subprocess is deliberate: the
//! Go producer and the RenderingGen profiler compute the same digest with the
//! same CLI, so the contract is the tool's output rather than a second Rust
//! implementation that could drift.
use crate::error::MuscleError;
use std::io::Write;
use std::process::{Command, Output, Stdio};

/// True for the canonical digest shape (64 lowercase hex characters).
pub(crate) fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Validates the tool's stdout and returns the digest. The length and
/// lowercase-hex checks are part of the contract, so every caller gets them.
fn parse_digest(output: &Output, subject: &str) -> Result<String, MuscleError> {
    if !output.status.success() {
        return Err(MuscleError::generic(format!(
            "compute SHA256 for {subject} failed"
        )));
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if !is_lower_sha256(&digest) {
        return Err(MuscleError::generic(format!(
            "sha256sum returned an invalid digest for {subject}"
        )));
    }
    Ok(digest)
}

/// Computes the lowercase SHA256 digest of `path` through `sha256sum`.
pub fn sha256_file(path: &str) -> Result<String, MuscleError> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| MuscleError::generic(format!("compute SHA256 for {path}: {error}")))?;
    parse_digest(&output, path)
}

/// Computes the lowercase SHA256 digest of in-memory bytes (the decoded
/// ffprobe extradata) through the same tool and validation contract.
pub(crate) fn sha256_bytes(bytes: &[u8]) -> Result<String, MuscleError> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| MuscleError::generic(format!("compute SHA256 for extradata: {error}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| MuscleError::generic("sha256 stdin unavailable"))?
        .write_all(bytes)
        .map_err(|error| MuscleError::generic(format!("compute SHA256 for extradata: {error}")))?;
    let output = child
        .wait_with_output()
        .map_err(|error| MuscleError::generic(format!("compute SHA256 for extradata: {error}")))?;
    parse_digest(&output, "extradata")
}
