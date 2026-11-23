use serde::{Deserialize, Serialize};

use super::OVERLAY_MANIFEST_SCHEMA_V1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverlayManifestAsset {
    pub asset_id: String,
    #[serde(default)]
    pub drive_file_id: String,
    #[serde(default)]
    pub url: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub frame_count: i64,
    pub duration_ms: i64,
}

// Spatial projection of an entry (OVERLAY-MANIFEST-CONTRACT.md §3.2,
// TICKET-PARADIGM-ALPHA-LAYER-HUD.md A1): the pixel rect on the plan canvas
// where the entry's content draws. Absent = full-canvas (the classic path).
// All four values live in one object, so a half-set rect cannot be expressed
// on the wire; a rect with a non-positive size or a negative origin is a
// malformed manifest and fails validation below (fail-closed, like the Go
// producer PipelineGen ManifestPlacement validation).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OverlayManifestPlacement {
    pub x_px: i64,
    pub y_px: i64,
    pub w_px: i64,
    pub h_px: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverlayManifestEntry {
    pub item_id: String,
    pub track: u32,
    pub in_us: i64,
    pub out_us: i64,
    pub content_hash: String,
    // Spatial projection (§3.2): both present or both absent is not
    // expressible — the object is all-or-nothing. Absent = full-canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<OverlayManifestPlacement>,
    // Packed-timeline positions (OVERLAY-MANIFEST-CONTRACT.md §3): where the
    // entry's content actually lives on the CERTIFIED artifact's own frames.
    // Both present or both absent — like the Go producer
    // (PipelineGen OverlayManifestEntry.Validate), a half-set pair is a
    // malformed manifest and fails validation below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_in_us: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_out_us: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum OverlayManifestWire {
    Set(OverlayManifestSet),
    Single(OverlayManifest),
}

#[derive(Clone, Debug, Deserialize)]
pub struct OverlayManifestSet {
    pub schema_version: String,
    pub manifests: Vec<OverlayManifest>,
}

impl OverlayManifestWire {
    pub fn manifests(&self) -> Vec<&OverlayManifest> {
        match self {
            Self::Single(manifest) => vec![manifest],
            Self::Set(set) => set.manifests.iter().collect(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Self::Set(set) = self {
            if set.schema_version != "pipelinegen.overlay-manifest-set.v1"
                || set.manifests.is_empty()
            {
                return Err("invalid or empty overlay manifest set".to_string());
            }
        }
        let mut items = std::collections::HashSet::new();
        let mut contents = std::collections::HashSet::new();
        for manifest in self.manifests() {
            manifest.validate()?;
            if let Self::Set(_) = self {
                let asset = manifest
                    .asset
                    .as_ref()
                    .ok_or("multi-artifact manifest requires an asset")?;
                if asset.asset_id.trim().is_empty()
                    || asset.sha256.len() != 64
                    || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    || asset.size_bytes <= 0
                    || asset.frame_count <= 0
                    || asset.duration_ms <= 0
                {
                    return Err("multi-artifact manifest has invalid asset facts".to_string());
                }
                if manifest
                    .timeline_sha256
                    .as_deref()
                    .is_none_or(|hash| !hash.eq_ignore_ascii_case(&asset.sha256))
                {
                    return Err("manifest timeline SHA-256 does not match asset".to_string());
                }
            }
            for entry in &manifest.entries {
                if matches!(self, Self::Set(_))
                    && (!items.insert(&entry.item_id)
                        || !contents.insert(entry.content_hash.to_ascii_lowercase()))
                {
                    return Err("duplicate item or content across overlay manifests".to_string());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverlayManifest {
    pub schema_version: String,
    pub plan_id: String,
    pub video_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<OverlayManifestAsset>,
    pub entries: Vec<OverlayManifestEntry>,
}

impl OverlayManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != OVERLAY_MANIFEST_SCHEMA_V1 {
            return Err(format!(
                "invalid overlay manifest schema: got {:?}, expected {:?}",
                self.schema_version, OVERLAY_MANIFEST_SCHEMA_V1
            ));
        }
        if self.plan_id.trim().is_empty() {
            return Err("overlay manifest plan_id is required".to_string());
        }
        for entry in &self.entries {
            if entry.item_id.trim().is_empty() {
                return Err("overlay manifest entry item_id is required".to_string());
            }
            if entry.in_us < 0 {
                return Err(format!(
                    "overlay manifest entry {} in_us must be >= 0",
                    entry.item_id
                ));
            }
            if entry.out_us <= entry.in_us {
                return Err(format!(
                    "overlay manifest entry {} out_us must be > in_us",
                    entry.item_id
                ));
            }
            if entry.content_hash.len() != 64 {
                return Err(format!(
                    "overlay manifest entry {} content_hash must be 64-char sha256",
                    entry.item_id
                ));
            }
            match (entry.artifact_in_us, entry.artifact_out_us) {
                (Some(artifact_in), Some(artifact_out)) => {
                    if artifact_in < 0 {
                        return Err(format!(
                            "overlay manifest entry {} artifact_in_us must be >= 0",
                            entry.item_id
                        ));
                    }
                    if artifact_out <= artifact_in {
                        return Err(format!(
                            "overlay manifest entry {} artifact_out_us must be > artifact_in_us",
                            entry.item_id
                        ));
                    }
                    if artifact_out - artifact_in > entry.out_us - entry.in_us {
                        // Packing only shrinks the canvas (the union of real
                        // overlay windows), so the packed span can never
                        // outlast the entry's literal plan window.
                        return Err(format!(
                            "overlay manifest entry {} packed span is longer than its plan window",
                            entry.item_id
                        ));
                    }
                }
                (None, None) => {}
                _ => {
                    return Err(format!(
                        "overlay manifest entry {} artifact_in_us/artifact_out_us must be set together",
                        entry.item_id
                    ));
                }
            }
            if let Some(placement) = &entry.placement {
                if placement.x_px < 0 || placement.y_px < 0 {
                    return Err(format!(
                        "overlay manifest entry {} placement x_px/y_px must be >= 0",
                        entry.item_id
                    ));
                }
                if placement.w_px <= 0 || placement.h_px <= 0 {
                    return Err(format!(
                        "overlay manifest entry {} placement w_px/h_px must be > 0",
                        entry.item_id
                    ));
                }
            }
        }
        Ok(())
    }
}
