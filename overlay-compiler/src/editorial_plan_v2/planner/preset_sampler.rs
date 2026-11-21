//! The seeded preset sampler, ported from PipelineGen's
//! `overlays.DeterministicPresetSampler`.
//!
//! The port exists because entity-card promotion moved into the planner: a
//! promoted card needs the same entrance preset the Go pass used to select, or
//! the same request would change its entrance rotation. Parity is byte-exact,
//! including the Go implementation's quirk of reading the **ASCII hex string**
//! of the digest rather than the digest bytes:
//!
//! ```text
//! seed   = JSON {"job_fingerprint","scene_id","semantic_id","preset_family"}
//! digest = SHA-256(seed) as lowercase hex
//! index  = u64::from_be_bytes(digest.as_bytes()[0..8]) % presets.len()
//! ```
//!
//! The candidate vocabulary is Chronon-owned and arrives in
//! `canvas.image_presets`: this module never hardcodes a preset id, so the Go
//! and Rust sides cannot drift from the registry. The four vectors in the
//! tests were measured with the Go sampler on the real `preset.image` pool.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The four-component seed. Field order is the serialized key order and is part
/// of the parity contract.
#[derive(Serialize)]
struct SampleSeed<'a> {
    job_fingerprint: &'a str,
    scene_id: &'a str,
    semantic_id: &'a str,
    preset_family: &'a str,
}

/// Selects the preset for one semantic item. An empty pool yields an empty
/// preset, exactly like the Go sampler.
pub(super) fn select_preset(
    job_fingerprint: &str,
    scene_id: &str,
    semantic_id: &str,
    preset_family: &str,
    presets: &[String],
) -> String {
    if presets.is_empty() {
        return String::new();
    }
    let seed = SampleSeed {
        job_fingerprint,
        scene_id,
        semantic_id,
        preset_family,
    };
    let encoded = serde_json::to_vec(&seed).expect("preset sampler seed serializes");
    let digest = Sha256::digest(&encoded);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    // Parity with Go: `binary.BigEndian.Uint64([]byte(sum[0:8]))` reads the
    // first eight hex *characters*, not the first eight digest bytes.
    let mut window = [0_u8; 8];
    window.copy_from_slice(&hex.as_bytes()[0..8]);
    let index = u64::from_be_bytes(window) % presets.len() as u64;
    presets[index as usize].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `preset.image` pool as authored in the Chronon registry
    /// (`animation_registry.v1.json`, family `preset.image`).
    fn image_presets() -> Vec<String> {
        [
            "image_fade_in",
            "image_focus_in",
            "image_scale_in",
            "image_slide_left",
            "image_slide_right",
        ]
        .iter()
        .map(|id| id.to_string())
        .collect()
    }

    /// Vectors measured with the Go sampler on `refactored@029f9b972`. A change
    /// to the seed order, the digest window or the family changes every one.
    #[test]
    fn matches_the_go_sampler_vectors() {
        let presets = image_presets();
        for (job, scene, item, want) in [
            (
                "job_1795166123889306562_acaae690",
                "scene-01",
                "scene-01:chain-image:0",
                "image_slide_left",
            ),
            (
                "job-fp-123",
                "scene_03",
                "sem_scene03_person_01",
                "image_focus_in",
            ),
            ("plan-chain-2", "scene-1", "ov-chain-2", "image_slide_right"),
            ("p", "s", "i", "image_focus_in"),
        ] {
            let got = select_preset(job, scene, item, "entity_image", &presets);
            assert_eq!(want, got, "seed ({job}, {scene}, {item}) drifted");
        }
    }

    #[test]
    fn an_empty_pool_selects_no_preset() {
        assert_eq!(
            "",
            select_preset("job", "scene", "item", "entity_image", &[])
        );
    }

    #[test]
    fn selection_is_stable_across_calls() {
        let presets = image_presets();
        let first = select_preset("job", "scene", "item", "entity_image", &presets);
        for _ in 0..8 {
            assert_eq!(
                first,
                select_preset("job", "scene", "item", "entity_image", &presets)
            );
        }
    }
}
