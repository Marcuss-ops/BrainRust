//! Content fingerprint of every embedded lexicon asset.
//!
//! The persistent worker cache outlives a process, so its key cannot trust the
//! hand-written version labels in [`AnalyzerVersions`](crate::AnalyzerVersions):
//! a lexicon edit that changes behaviour without a label bump would let a new
//! build serve a manifest produced by the old data. This module hashes the
//! bytes that are actually compiled into the binary, so any content change
//! moves the key.
//!
//! The list below mirrors the `include_str!` assets used by `lexicon`,
//! `entity_linker` and the generated packs. It is intentionally explicit: a
//! path that moves fails to compile instead of silently dropping out of the
//! fingerprint.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};

/// SHA-256 over a deterministic set of named assets.
///
/// Each entry contributes its name, its length and its bytes, and the entries
/// are sorted by name, so the result does not depend on insertion order.
pub fn hash_assets<'a, I>(assets: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    let mut entries: Vec<(&str, &[u8])> = assets.into_iter().collect();
    entries.sort_by_key(|(name, _)| *name);
    let mut hasher = Sha256::new();
    for (name, bytes) in entries {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    lowercase_hex(&hasher.finalize())
}

/// Fingerprint of the assets compiled into this build.
pub fn embedded_content_fingerprint() -> &'static str {
    static FINGERPRINT: OnceLock<String> = OnceLock::new();
    FINGERPRINT
        .get_or_init(|| {
            let assets = embedded_assets();
            hash_assets(
                assets
                    .iter()
                    .map(|(name, body)| (name.as_str(), body.as_bytes())),
            )
        })
        .as_str()
}

/// Every embedded asset that can change an analysis result, with its build
/// path. Public so a test (or a diagnostic command) can hash a modified copy
/// and prove the fingerprint is content-derived.
pub fn embedded_assets() -> Vec<(String, &'static str)> {
    let mut assets: Vec<(String, &'static str)> = vec![
        (
            "lexicon/entity_gazetteer.json".to_string(),
            include_str!("../../lexicon/entity_gazetteer.json"),
        ),
        (
            "lexicon/entity_gazetteer_delta.json".to_string(),
            include_str!("../../lexicon/entity_gazetteer_delta.json"),
        ),
        (
            "lexicon/place_inflections.json".to_string(),
            include_str!("../../lexicon/place_inflections.json"),
        ),
        (
            "lexicon/months.json".to_string(),
            include_str!("../../lexicon/months.json"),
        ),
        (
            "lexicon/stopwords.json".to_string(),
            include_str!("../../lexicon/stopwords.json"),
        ),
        (
            "lexicon/object_vocabulary.json".to_string(),
            include_str!("../../lexicon/object_vocabulary.json"),
        ),
        (
            "lexicon/orthography.json".to_string(),
            include_str!("../../lexicon/orthography.json"),
        ),
        (
            "lexicon/language_profiles.json".to_string(),
            include_str!("../../lexicon/language_profiles.json"),
        ),
        (
            "lexicon/ordered_steps.json".to_string(),
            include_str!("../../lexicon/ordered_steps.json"),
        ),
        (
            "lexicon/scale_words.json".to_string(),
            include_str!("../../lexicon/scale_words.json"),
        ),
        (
            "lexicon/number_words.json".to_string(),
            include_str!("../../lexicon/number_words.json"),
        ),
        (
            "lexicon/units.json".to_string(),
            include_str!("../../lexicon/units.json"),
        ),
        (
            "lexicon/relative_time.json".to_string(),
            include_str!("../../lexicon/relative_time.json"),
        ),
        (
            "lexicon/location_cues.json".to_string(),
            include_str!("../../lexicon/location_cues.json"),
        ),
        (
            "lexicon/process_phases.json".to_string(),
            include_str!("../../lexicon/process_phases.json"),
        ),
        (
            "lexicon/facility_facts.json".to_string(),
            include_str!("../../lexicon/facility_facts.json"),
        ),
        (
            "lexicon/discourse_cues.json".to_string(),
            include_str!("../../lexicon/discourse_cues.json"),
        ),
        (
            "lexicon/relation_cues.json".to_string(),
            include_str!("../../lexicon/relation_cues.json"),
        ),
        (
            "lexicon/entity_index_v1.json".to_string(),
            include_str!("../../lexicon/entity_index_v1.json"),
        ),
        (
            "lexicon/link_weights_v1.json".to_string(),
            include_str!("../../lexicon/link_weights_v1.json"),
        ),
        // The legal pack index decides which domain packs load and is parsed at
        // startup, so it is a compiled asset like the packs themselves.
        (
            "lexicon/legal/index.json".to_string(),
            include_str!("../../lexicon/legal/index.json"),
        ),
    ];
    assets.push((
        "lexicon/units/index.json".to_string(),
        include_str!("../../lexicon/units/index.json"),
    ));
    assets.extend([
        (
            "lexicon/units/de.json".to_string(),
            include_str!("../../lexicon/units/de.json"),
        ),
        (
            "lexicon/units/en.json".to_string(),
            include_str!("../../lexicon/units/en.json"),
        ),
        (
            "lexicon/units/es.json".to_string(),
            include_str!("../../lexicon/units/es.json"),
        ),
        (
            "lexicon/units/fr.json".to_string(),
            include_str!("../../lexicon/units/fr.json"),
        ),
        (
            "lexicon/units/id.json".to_string(),
            include_str!("../../lexicon/units/id.json"),
        ),
        (
            "lexicon/units/it.json".to_string(),
            include_str!("../../lexicon/units/it.json"),
        ),
        (
            "lexicon/units/nl.json".to_string(),
            include_str!("../../lexicon/units/nl.json"),
        ),
        (
            "lexicon/units/pl.json".to_string(),
            include_str!("../../lexicon/units/pl.json"),
        ),
        (
            "lexicon/units/pt.json".to_string(),
            include_str!("../../lexicon/units/pt.json"),
        ),
        (
            "lexicon/units/ru.json".to_string(),
            include_str!("../../lexicon/units/ru.json"),
        ),
        (
            "lexicon/units/tr.json".to_string(),
            include_str!("../../lexicon/units/tr.json"),
        ),
    ]);
    for (language, body) in crate::generated_language_packs::PACKS {
        assets.push((format!("lexicon/packs/{language}.json"), body));
    }
    for (language, body) in crate::generated_language_profiles::PROFILES {
        assets.push((format!("lexicon/profiles/{language}.json"), body));
    }
    for (language, body) in crate::generated_legal_packs::LEGAL_PACKS {
        assets.push((format!("lexicon/legal/{language}.json"), body));
    }
    assets
}

fn lowercase_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_is_a_stable_sha256() {
        let first = embedded_content_fingerprint();
        let second = embedded_content_fingerprint();
        assert_eq!(first, second, "the fingerprint is computed once");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn changing_one_byte_of_an_asset_changes_the_fingerprint() {
        let assets: Vec<(String, Vec<u8>)> = embedded_assets()
            .into_iter()
            .map(|(name, body)| (name, body.as_bytes().to_vec()))
            .collect();
        let baseline = hash_assets(
            assets
                .iter()
                .map(|(name, body)| (name.as_str(), body.as_slice())),
        );

        // Append one byte to one asset, as an edit to the lexicon would.
        let mut edited = assets.clone();
        let index = edited
            .iter()
            .position(|(name, _)| name.ends_with("stopwords.json"))
            .expect("stopwords asset is embedded");
        edited[index].1.push(b' ');
        let after_edit = hash_assets(
            edited
                .iter()
                .map(|(name, body)| (name.as_str(), body.as_slice())),
        );

        assert_ne!(
            baseline, after_edit,
            "a content change must move the fingerprint without a label bump"
        );
    }

    #[test]
    fn asset_order_does_not_change_the_fingerprint() {
        let forward = hash_assets([("a", b"one".as_slice()), ("b", b"two".as_slice())]);
        let reversed = hash_assets([("b", b"two".as_slice()), ("a", b"one".as_slice())]);
        assert_eq!(forward, reversed);
    }

    #[test]
    fn the_asset_list_covers_every_embedded_pack() {
        let assets = embedded_assets();
        let has = |name: &str| assets.iter().any(|(asset, _)| asset == name);

        assert!(has("lexicon/entity_gazetteer_delta.json"));
        assert!(has("lexicon/entity_index_v1.json"));
        assert!(has("lexicon/link_weights_v1.json"));
        assert!(has("lexicon/legal/index.json"));
        for (language, _) in crate::generated_language_packs::PACKS {
            assert!(has(&format!("lexicon/packs/{language}.json")));
        }
        for (language, _) in crate::generated_language_profiles::PROFILES {
            assert!(has(&format!("lexicon/profiles/{language}.json")));
        }
        for (language, _) in crate::generated_legal_packs::LEGAL_PACKS {
            assert!(has(&format!("lexicon/legal/{language}.json")));
        }
    }

    /// Guards the invariant the fingerprint relies on: a table that is added to
    /// `lexicon/` and compiled into the binary must appear in the digest. The
    /// two `index.json` files under `packs/` and `profiles/` are build-tool
    /// manifests; they are not compiled and are the only explicit exemption.
    #[test]
    fn every_compiled_lexicon_table_is_in_the_fingerprint() {
        const BUILD_TOOL_ONLY: &[&str] = &[
            "packs/index.json",
            "profiles/index.json",
            "entity_index_sources_v1.json",
            "entity_index_collisions_v1.json",
            "entity_index_confusables_v1.json",
        ];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lexicon");
        let Ok(top) = std::fs::read_dir(&root) else {
            return; // packaged crate without the repository data
        };
        let names: Vec<String> = embedded_assets()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let mut checked = 0usize;
        for entry in top.flatten() {
            let relative = format!("lexicon/{}", entry.file_name().to_string_lossy());
            if entry.path().is_dir() {
                for inner in std::fs::read_dir(entry.path())
                    .expect("read lexicon subdir")
                    .flatten()
                {
                    let child = format!("{relative}/{}", inner.file_name().to_string_lossy());
                    if !child.ends_with(".json") || BUILD_TOOL_ONLY.contains(&&child[8..]) {
                        continue;
                    }
                    assert!(
                        names.contains(&child),
                        "{child} is embedded but missing from the content fingerprint"
                    );
                    checked += 1;
                }
            } else if relative.ends_with(".json") && !BUILD_TOOL_ONLY.contains(&&relative[8..]) {
                assert!(
                    names.contains(&relative),
                    "{relative} is embedded but missing from the content fingerprint"
                );
                checked += 1;
            }
        }
        assert!(checked >= 40, "scanned only {checked} lexicon tables");
    }
}
