//! Canonical request encoding and the wire-visible manifest fingerprint.
//!
//! The in-memory fingerprint and the on-disk cache key hash the same bytes, so
//! the two cannot disagree about what "the same request" means. This module
//! owns that encoding; [`super::disk`] adds its wider SHA-256 cache key on top
//! without re-serializing the request.

use semantic_core::{AnalyzerVersions, ScriptRequest};

/// Canonical bytes of every input that can change a manifest: language, scenes,
/// editorial options, safety cap, reference date and analyzer versions.
pub(crate) fn request_body(request: &ScriptRequest, analyzers: &AnalyzerVersions) -> Vec<u8> {
    serde_json::to_vec(&(
        request.language.trim(),
        &request.scenes,
        &request.editorial,
        request.entity_safety_cap,
        &request.reference_date,
        analyzers,
    ))
    .unwrap_or_default()
}

/// FNV-1a fingerprint over every input that can change a manifest.
///
/// This is the wire-visible `manifest.fingerprint` and stays a 64-bit FNV value
/// for contract compatibility; the persistent cache uses a wider SHA-256 key
/// over the same [`request_body`] bytes (see [`super::disk`]).
pub fn request_fingerprint(request: &ScriptRequest, analyzers: &AnalyzerVersions) -> String {
    fnv1a_hex(&request_body(request, analyzers))
}

/// One FNV-1a 64 implementation for every manifest fingerprint.
fn fnv1a_hex(body: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}
