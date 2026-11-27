//! `semantic_manifest.v4` — the normalized identity table.
//!
//! v3 copies the whole accumulated identity registry into every
//! [`SceneManifest`](super::SceneManifest). With N scenes and identities that
//! keep appearing, that is O(N²) copies of the same objects on the wire, in
//! the persistent cache, in the NDJSON response and in the consumer's memory.
//!
//! v4 keeps the same document shape but moves the registry to ONE document
//! table (`SemanticManifest.identities` equivalent) and lets each scene carry
//! only the ids it actually mentions (`identity_ids`). The conversion is a
//! wire projection: the engine still analyses with the v3 registry, so v3
//! output remains byte-identical while PipelineGen migrates.
//!
//! Invariants enforced by [`to_v4_value`] and the contract tests:
//!
//! * identities keep registration order (ids are part of byte-for-byte
//!   parity);
//! * every identity object appears once, with every mention;
//! * `identity_ids` are the ids whose mentions include that scene, in global
//!   table order;
//! * a scene never carries an `identities` key;
//! * the conversion never mutates the v3 manifest.
//!
//! The canonical contract (schema, golden, validator) lives in
//! `contract/semantic-manifest.v4/` and is shared byte-for-byte with
//! PipelineGen's `testdata/semantic-manifest/v4/`.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::identity::CanonicalIdentity;

use super::{SemanticManifest, SCHEMA_VERSION};

/// Wire version of the normalized manifest.
pub const SCHEMA_VERSION_V4: &str = "semantic_manifest.v4";

/// Versions this engine can produce. `v3` is the default; `v4` is the
/// opt-in cutover version. A version outside this list is rejected, never
/// interpreted as v3 by shape.
pub const PRODUCIBLE_SCHEMA_VERSIONS: &[&str] = &[SCHEMA_VERSION, SCHEMA_VERSION_V4];

/// Output wire version requested by a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestVersion {
    V3,
    V4,
}

impl ManifestVersion {
    /// Parse the optional `manifest_version` request field. Absent or empty
    /// means v3, so every existing caller keeps the current contract.
    /// Both the short (`v4`) and the wire (`semantic_manifest.v4`) forms are
    /// accepted; anything else fails closed with a typed error.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim) {
            None | Some("") | Some("v3") | Some(SCHEMA_VERSION) => Ok(Self::V3),
            Some("v4") | Some(SCHEMA_VERSION_V4) => Ok(Self::V4),
            Some(other) => Err(format!(
                "unsupported manifest_version {other:?}; supported versions: {SCHEMA_VERSION}, {SCHEMA_VERSION_V4}"
            )),
        }
    }

    pub fn schema_version(self) -> &'static str {
        match self {
            Self::V3 => SCHEMA_VERSION,
            Self::V4 => SCHEMA_VERSION_V4,
        }
    }

    /// Stable cache-key suffix. v3 keeps the historical key (`""`), so a
    /// cutover request can never collide with a v3 cache entry.
    pub fn cache_suffix(self) -> &'static str {
        match self {
            Self::V3 => "",
            Self::V4 => ":semantic_manifest.v4",
        }
    }
}

/// One identity table for the whole manifest, in first-registration order.
///
/// v3 scene lists are cumulative, so the LAST occurrence of an id carries the
/// complete mention list (a mention added in a later scene is missing from the
/// earlier copy). Replacing by id while keeping first-seen order preserves both
/// registration order and every mention.
pub fn global_identities(manifest: &SemanticManifest) -> Vec<CanonicalIdentity> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, CanonicalIdentity> = HashMap::new();
    for scene in &manifest.scenes {
        for identity in &scene.identities {
            if !by_id.contains_key(&identity.id) {
                order.push(identity.id.clone());
            }
            by_id.insert(identity.id.clone(), identity.clone());
        }
    }
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

/// Ids of the identities this scene mentions, in global-table order.
pub fn identity_ids_for_scene(identities: &[CanonicalIdentity], scene_id: &str) -> Vec<String> {
    identities
        .iter()
        .filter(|identity| {
            identity
                .mentions
                .iter()
                .any(|mention| mention.scene_id == scene_id)
        })
        .map(|identity| identity.id.clone())
        .collect()
}

/// Project a v3 manifest onto the normalized v4 wire document.
///
/// The input is never mutated. The returned [`Value`] is deterministic: the
/// same manifest always produces the same bytes, because identities keep
/// registration order and scenes keep source order.
pub fn to_v4_value(manifest: &SemanticManifest) -> Result<Value, String> {
    // Project from the canonical v3 WIRE bytes, not from `to_value`: converting
    // f32 scores to a Value would widen them to f64 and change their textual
    // representation (0.76000005 -> 0.7600000500679016). Parsing the v3 bytes
    // keeps every non-identity field byte-for-byte identical in v4.
    let v3_bytes =
        serde_json::to_vec(manifest).map_err(|err| format!("encode v3 manifest: {err}"))?;
    let mut value: Value = serde_json::from_slice(&v3_bytes)
        .map_err(|err| format!("decode v3 manifest for normalization: {err}"))?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| "v3 manifest is not a JSON object".to_string())?;
    root.insert("schema_version".to_string(), json!(SCHEMA_VERSION_V4));

    let identities = global_identities(manifest);
    let scenes = root
        .get_mut("scenes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "v3 manifest has no scenes array".to_string())?;
    if scenes.len() != manifest.scenes.len() {
        return Err(format!(
            "encoded scenes {} do not match manifest scenes {}",
            scenes.len(),
            manifest.scenes.len()
        ));
    }
    for (scene_value, scene) in scenes.iter_mut().zip(&manifest.scenes) {
        let scene_object = scene_value
            .as_object_mut()
            .ok_or_else(|| format!("scene {} is not a JSON object", scene.scene_id))?;
        // v4 must not carry the cumulative copy; the key is removed entirely,
        // not emptied, so a v3-shaped reader cannot mistake it for "no
        // identities in this scene".
        scene_object.remove("identities");
        scene_object.insert(
            "identity_ids".to_string(),
            json!(identity_ids_for_scene(&identities, &scene.scene_id)),
        );
    }
    root.insert(
        "identities".to_string(),
        serde_json::to_value(&identities)
            .map_err(|err| format!("encode v4 identity table: {err}"))?,
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Mention;

    fn identity(id: &str, scene_ids: &[&str]) -> CanonicalIdentity {
        CanonicalIdentity {
            id: id.to_string(),
            canonical_id: format!("person:{id}"),
            canonical_surface: format!("Surface {id}"),
            kind: "PERSON".to_string(),
            mentions: scene_ids
                .iter()
                .map(|scene_id| Mention {
                    scene_id: scene_id.to_string(),
                    surface: format!("Surface {id}"),
                    start_byte: 0,
                    end_byte: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn version_parsing_defaults_to_v3_and_rejects_unknown() {
        assert_eq!(ManifestVersion::parse(None), Ok(ManifestVersion::V3));
        assert_eq!(ManifestVersion::parse(Some("")), Ok(ManifestVersion::V3));
        assert_eq!(
            ManifestVersion::parse(Some("semantic_manifest.v3")),
            Ok(ManifestVersion::V3)
        );
        assert_eq!(ManifestVersion::parse(Some("v4")), Ok(ManifestVersion::V4));
        assert_eq!(
            ManifestVersion::parse(Some("semantic_manifest.v4")),
            Ok(ManifestVersion::V4)
        );
        let error = ManifestVersion::parse(Some("semantic_manifest.v5")).unwrap_err();
        assert!(error.contains("unsupported manifest_version"));
        assert!(
            !error.contains("semantic_manifest.v3\", "),
            "error names v3 as a fallback"
        );
    }

    /// The shared golden is the wire contract PipelineGen copies into its
    /// testdata. The producer certifies the same invariants the Go consumer
    /// and `validate.py` enforce, without depending on the analyzer stack.
    #[test]
    fn shared_contract_golden_satisfies_v4_invariants() {
        use std::collections::BTreeSet;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../contract/semantic-manifest.v4/goldens");
        let v4: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("manifest-v4.json")).expect("v4 golden"),
        )
        .expect("v4 golden json");
        let v3: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("manifest-v3.json")).expect("v3 golden"),
        )
        .expect("v3 golden json");

        assert_eq!(v4["schema_version"], SCHEMA_VERSION_V4);
        assert_eq!(v3["schema_version"], SCHEMA_VERSION);
        let identities = v4["identities"].as_array().expect("v4 identity table");
        let ids: BTreeSet<&str> = identities
            .iter()
            .map(|identity| identity["id"].as_str().expect("identity id"))
            .collect();
        assert_eq!(ids.len(), identities.len(), "duplicate identity id");

        let scenes = v4["scenes"].as_array().expect("v4 scenes");
        let mut referenced = BTreeSet::new();
        for scene in scenes {
            assert!(
                scene.get("identities").is_none(),
                "v4 golden scene carries a per-scene copy"
            );
            for id in scene["identity_ids"].as_array().expect("identity_ids") {
                let id = id.as_str().expect("identity id string");
                assert!(ids.contains(id), "dangling identity {id}");
                referenced.insert(id);
            }
        }
        assert_eq!(
            referenced.len(),
            ids.len(),
            "every golden identity must be referenced"
        );

        let v3_scenes = v3["scenes"].as_array().expect("v3 scenes");
        assert_eq!(v3_scenes.len(), scenes.len());
        for scene in v3_scenes {
            assert!(scene.get("identities").is_some());
            assert!(scene.get("identity_ids").is_none());
        }
    }

    #[test]
    fn identity_ids_follow_global_order_and_scene_mentions() {
        let identities = vec![
            identity("person_01", &["s1", "s2"]),
            identity("org_01", &["s2"]),
            identity("person_02", &["s3"]),
        ];
        assert_eq!(
            identity_ids_for_scene(&identities, "s1"),
            vec!["person_01".to_string()]
        );
        assert_eq!(
            identity_ids_for_scene(&identities, "s2"),
            vec!["person_01".to_string(), "org_01".to_string()]
        );
        assert!(identity_ids_for_scene(&identities, "s4").is_empty());
    }
}
