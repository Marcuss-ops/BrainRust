//! Shared lookup for the generated per-language pack tables.
//!
//! Language packs and legal packs have different payload types, but their
//! index file and generated `(language, json)` table have the same shape.
//! This module owns that lookup so a missing or renamed language is handled
//! once instead of in two near-identical loaders.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawIndex {
    languages: Vec<String>,
}

/// Calls `visit` once per language present in both the committed index and
/// the generated table, in index order. Unknown languages are skipped; a
/// malformed index yields no entries (declared degradation, never a panic).
pub(crate) fn for_each_generated_pack(
    index_json: &str,
    entries: &'static [(&'static str, &'static str)],
    mut visit: impl FnMut(&str, &str),
) {
    let index: RawIndex = serde_json::from_str(index_json).unwrap_or(RawIndex {
        languages: Vec::new(),
    });
    let available: std::collections::HashMap<&str, &str> = entries.iter().copied().collect();
    for language in index.languages {
        let Some(json) = available.get(language.as_str()) else {
            continue;
        };
        visit(&language, json);
    }
}
