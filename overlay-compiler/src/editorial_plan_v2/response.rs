//! The complete `editorial_plan.v2` response: the final, sealed overlay plan.
//!
//! The response is complete by contract. `items` is the admitted set in final
//! render order; `intake` is the same drop accounting the Go tracker produces;
//! `duration_ms` is derived from the request's canonical extents. A missing
//! item section, a duplicate id, an item outside its scene window or an
//! unvalidated map overlay is an error, never something Go may repair.
//!
//! Item fields mirror the `capmodel.OverlayItem` JSON tags one for one, so a
//! plan produced here round-trips through the current Go plan document without
//! losing opaque render metadata (`params`, `map`, `image_layers`, `style`).
//! The one addition is `entity_ref`: the planner owns WHO a card is about, Go
//! projects it onto the producer-internal `OverlayItem.EntityRef`, and it never
//! reaches the `overlay-plan.v1` wire the worker renders.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::EditorialPlanError;
use super::request::{is_lower_hex_64, AssetRefV2, EditorialPlanV2Request, SceneV2};
use super::EDITORIAL_PLAN_V2_SCHEMA_VERSION;

/// Top-level worker response. `ok` is always `true` on this type; failures use
/// the protocol's `{"ok":false,"error":"..."}` envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialPlanV2Compiled {
    pub ok: bool,
    pub schema_version: String,
    pub plan: FinalPlanV2,
}

/// The sealed plan. `schema_version` is repeated here so a persisted plan can
/// be validated without the transport envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinalPlanV2 {
    pub schema_version: String,
    pub plan_id: String,
    pub video_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_id: String,
    pub language: String,
    pub width: i64,
    pub height: i64,
    pub fps_num: i64,
    pub fps_den: i64,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Value>,
    #[serde(default)]
    pub items: Vec<PlanItemV2>,
    #[serde(default)]
    pub recap_placements: i64,
    #[serde(default)]
    pub recap_cards: i64,
    #[serde(default)]
    pub chart_placements: i64,
    #[serde(default)]
    pub chart_cards: i64,
    #[serde(default)]
    pub timed_scenes: i64,
    #[serde(default)]
    pub intake: IntakeV2,
    // Additive decision log (ROADMAP 2026-11-19 §4): every candidate the
    // planner discarded carries one entry with a stable reason code. An empty
    // log is omitted, so a v2 consumer that knows nothing about it decodes the
    // same document it decoded before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<PlanDecisionV2>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output_profile_id: String,
}

/// One planner decision. `reason_code` is a stable enum spelling, never free
/// text, so tests and consumers can match it deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlanDecisionV2 {
    pub candidate_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scene_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub canonical_entity_id: String,
    pub stage: String,
    pub outcome: String,
    pub reason_code: String,
}

/// One final overlay item. The typed fields are the ones the planner decides;
/// the opaque ones carry renderer metadata through unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlanItemV2 {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scene_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subgroup_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entity_id: String,
    // The planner-owned canonical identity of the card. It travels on this
    // response only: Go projects it onto `OverlayItem.EntityRef`, which is
    // PipelineGen-internal and never part of the overlay-plan.v1 wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_ref: Option<EntityRefV2>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start_us: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration_us: i64,
    #[serde(default)]
    pub template_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preset_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image_preset_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub motion_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entity_style_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_params: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entity_caption: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption_motion_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_motion_params: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption_font_family: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption_layout: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption_color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asset_refs: Vec<AssetRefV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_effect: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_layers: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group_motion_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub render_key: String,
}

impl PlanItemV2 {
    /// Absolute microsecond window; falls back to the millisecond projection
    /// exactly like the current plan contract does for legacy items.
    pub fn window_us(&self) -> (i64, i64) {
        if self.duration_us > 0 {
            (self.start_us, self.start_us + self.duration_us)
        } else {
            (self.start_ms * 1_000, self.end_ms * 1_000)
        }
    }
}

/// Content-addressed identity of one entity card: the stable occurrence id,
/// the canonical type and name, and the canonical entity id the media resolver
/// indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRefV2 {
    pub entity_id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub canonical_entity_id: String,
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

/// Run-level drop accounting. It is observability, not a second selection
/// stage: `planned` is always the length of `items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IntakeV2 {
    #[serde(default)]
    pub scenes: i64,
    #[serde(default)]
    pub timed_scenes: i64,
    #[serde(default)]
    pub untimed_scenes: i64,
    #[serde(default)]
    pub budget: IntakeBudgetV2,
    #[serde(default)]
    pub candidates: i64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub candidates_by_kind: BTreeMap<String, i64>,
    #[serde(default)]
    pub planned: i64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub planned_by_kind: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dropped_by_reason: BTreeMap<String, i64>,
}

/// The budgets the canvas selected, in the same spelling the Go tracker uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IntakeBudgetV2 {
    #[serde(default)]
    pub phrase_overlays: i64,
    #[serde(default)]
    pub image_overlays: i64,
    #[serde(default)]
    pub map_overlays: i64,
    #[serde(default)]
    pub max_overlays_per_scene: i64,
}

/// The projection response of the landed slice: only the phrases the scene
/// projection added and the scene IDs whose bullet slot is now owned by a
/// chapter card. The caller keeps its complete scene DTOs and appends these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialPlanV2Response {
    pub schema_version: String,
    #[serde(default)]
    pub scene_phrases: BTreeMap<String, Vec<ScenePhraseV2>>,
    #[serde(default)]
    pub covered_scenes: Vec<String>,
}

/// One projected editorial/chapter phrase, with certified timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenePhraseV2 {
    pub text: String,
    pub start_us: i64,
    pub duration_us: i64,
    pub score: f64,
    pub presentation_role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub candidate: String,
}

impl EditorialPlanV2Compiled {
    /// Validates the envelope, then the plan against its own invariants.
    pub fn validate(&self) -> Result<(), EditorialPlanError> {
        if !self.ok {
            return Err(EditorialPlanError::invalid("response.ok", "must be true"));
        }
        if self.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            });
        }
        self.plan.validate()
    }

    /// Validates that this plan is the one the request asked for, and that no
    /// item escapes its owning scene. Go may call this and adopt the plan; it
    /// may not repair one.
    pub fn validate_for(&self, request: &EditorialPlanV2Request) -> Result<(), EditorialPlanError> {
        self.validate()?;
        request.validate()?;
        let plan = &self.plan;
        if plan.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: plan.schema_version.clone(),
                expected: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            });
        }
        for (field, left, right) in [
            (
                "plan_id",
                plan.plan_id.as_str(),
                request.plan.plan_id.as_str(),
            ),
            (
                "video_id",
                plan.video_id.as_str(),
                request.plan.video_id.as_str(),
            ),
            (
                "language",
                plan.language.as_str(),
                request.language.as_str(),
            ),
        ] {
            if !left.eq_ignore_ascii_case(right) {
                return Err(EditorialPlanError::invalid(
                    format!("plan.{field}"),
                    format!("{left:?} does not match the request {right:?}"),
                ));
            }
        }
        for (field, left, right) in [
            ("width", plan.width, request.plan.width),
            ("height", plan.height, request.plan.height),
            ("fps_num", plan.fps_num, request.plan.fps_num),
            ("fps_den", plan.fps_den, request.plan.fps_den),
        ] {
            if left != right {
                return Err(EditorialPlanError::invalid(
                    format!("plan.{field}"),
                    format!("{left} does not match the request {right}"),
                ));
            }
        }
        if let Some(seal) = &request.seal {
            let audio_ms = seal.audio_duration_us / 1_000;
            if plan.duration_ms < 0 || plan.duration_ms + 1_000 < audio_ms {
                return Err(EditorialPlanError::invalid(
                    "plan.duration_ms",
                    "is shorter than the canonical audio extent",
                ));
            }
            if !seal.output_profile_id.is_empty()
                && plan.output_profile_id != seal.output_profile_id
            {
                return Err(EditorialPlanError::invalid(
                    "plan.output_profile_id",
                    format!(
                        "{:?} does not match the request {:?}",
                        plan.output_profile_id, seal.output_profile_id
                    ),
                ));
            }
        }
        self.plan.validate_against_scenes(&request.scenes)
    }
}

impl FinalPlanV2 {
    /// Structural validation of the sealed plan.
    pub fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            });
        }
        if self.plan_id.trim().is_empty() || self.video_id.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                "plan",
                "plan_id and video_id are required",
            ));
        }
        if self.width <= 0 || self.height <= 0 {
            return Err(EditorialPlanError::invalid(
                "plan",
                "width and height must be positive",
            ));
        }
        if self.fps_num <= 0 || self.fps_den <= 0 {
            return Err(EditorialPlanError::invalid(
                "plan",
                "fps_num and fps_den must be positive",
            ));
        }
        if self.duration_ms < 0 {
            return Err(EditorialPlanError::invalid(
                "plan.duration_ms",
                "must not be negative",
            ));
        }
        if self.intake.planned != self.items.len() as i64 {
            return Err(EditorialPlanError::invalid(
                "plan.intake.planned",
                format!(
                    "{} does not match {} final items",
                    self.intake.planned,
                    self.items.len()
                ),
            ));
        }
        if self.intake.candidates < self.intake.planned {
            return Err(EditorialPlanError::invalid(
                "plan.intake.candidates",
                "is smaller than the planned item count",
            ));
        }
        let mut ids = BTreeSet::new();
        for item in &self.items {
            if item.id.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "plan.items",
                    "item id is required",
                ));
            }
            if !ids.insert(item.id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "plan.items",
                    format!("duplicate item id {:?}", item.id),
                ));
            }
            let (start_us, end_us) = item.window_us();
            if start_us < 0 || end_us <= start_us {
                return Err(EditorialPlanError::invalid(
                    format!("plan.items.{}", item.id),
                    "invalid item window",
                ));
            }
            if let Some(map) = &item.map {
                if item.kind != "map" && map.is_null() {
                    return Err(EditorialPlanError::invalid(
                        format!("plan.items.{}", item.id),
                        "map payload is null",
                    ));
                }
            }
            for asset in &item.asset_refs {
                if asset.asset_id.trim().is_empty() {
                    return Err(EditorialPlanError::invalid(
                        format!("plan.items.{}.asset_refs", item.id),
                        "asset_id is required",
                    ));
                }
                if !asset.sha256.trim().is_empty() && !is_lower_hex_64(&asset.sha256) {
                    return Err(EditorialPlanError::invalid(
                        format!("plan.items.{}.asset_refs", item.id),
                        "sha256 must be 64 lowercase hex characters",
                    ));
                }
            }
        }
        // A chain owns its window end to end. An overlapping non-chain item
        // would be composited after it and hide the portraits and captions in
        // the final video; the planner reserves the window before sealing.
        for chain in self
            .items
            .iter()
            .filter(|item| super::is_chain_kind(&item.kind))
        {
            let (chain_start, chain_end) = chain.window_us();
            for other in &self.items {
                if other.id == chain.id || super::is_chain_kind(&other.kind) {
                    continue;
                }
                if matches!(
                    other.template_id.trim().to_ascii_lowercase().as_str(),
                    "background" | "video_background"
                ) {
                    continue;
                }
                let (start_us, end_us) = other.window_us();
                if start_us < chain_end && chain_start < end_us {
                    return Err(EditorialPlanError::invalid(
                        format!("plan.items.{}", chain.id),
                        format!(
                            "chain window [{chain_start},{chain_end}) overlaps item {:?} [{start_us},{end_us})",
                            other.id
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Validates every item against the scene that owns it. A map is allowed to
    /// outlive its scene: the planner extends it to the certified camera beat
    /// and only the plan end bounds that presentation window (a map may start
    /// late, it may not end after the last frame). Every other kind stays
    /// inside its own scene.
    pub fn validate_against_scenes(&self, scenes: &[SceneV2]) -> Result<(), EditorialPlanError> {
        let windows: BTreeMap<&str, (i64, i64)> = scenes
            .iter()
            .filter(|scene| scene.duration_us > 0)
            .map(|scene| {
                (
                    scene.id.as_str(),
                    (
                        scene.timeline_start_us,
                        scene.timeline_start_us + scene.duration_us,
                    ),
                )
            })
            .collect();
        let plan_end_us = windows.values().map(|(_, end)| *end).max().unwrap_or(0);
        for item in &self.items {
            if item.scene_id.trim().is_empty() {
                continue;
            }
            let Some((scene_start, scene_end)) = windows.get(item.scene_id.as_str()) else {
                return Err(EditorialPlanError::invalid(
                    format!("plan.items.{}", item.id),
                    format!("references unknown or untimed scene {:?}", item.scene_id),
                ));
            };
            let (start_us, end_us) = item.window_us();
            let allowed_end = if item.kind == "map" {
                plan_end_us
            } else {
                *scene_end
            };
            if start_us < *scene_start || end_us > allowed_end {
                return Err(EditorialPlanError::invalid(
                    format!("plan.items.{}", item.id),
                    format!(
                        "window [{start_us},{end_us}) escapes its allowed extent [{scene_start},{allowed_end})"
                    ),
                ));
            }
        }
        Ok(())
    }
}

impl EditorialPlanV2Response {
    /// Structural validation of the projection response. The caller must
    /// reject a malformed response before appending any phrase to its scene
    /// DTOs.
    pub fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            });
        }
        let mut covered = BTreeSet::new();
        for scene_id in &self.covered_scenes {
            if scene_id.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "response.covered_scenes",
                    "scene id is empty",
                ));
            }
            if !covered.insert(scene_id) {
                return Err(EditorialPlanError::invalid(
                    "response.covered_scenes",
                    format!("duplicate covered scene {scene_id:?}"),
                ));
            }
        }
        for (scene_id, phrases) in &self.scene_phrases {
            if scene_id.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "response.scene_phrases",
                    "scene id is empty",
                ));
            }
            for phrase in phrases {
                if phrase.text.trim().is_empty() {
                    return Err(EditorialPlanError::invalid(
                        format!("response.scene_phrases.{scene_id}"),
                        "empty phrase text",
                    ));
                }
                if phrase.start_us < 0 || phrase.duration_us <= 0 {
                    return Err(EditorialPlanError::invalid(
                        format!("response.scene_phrases.{scene_id}"),
                        "invalid phrase window",
                    ));
                }
                if !matches!(
                    phrase.presentation_role.as_str(),
                    "scene_title" | "scene_bullet" | "chapter_bullet"
                ) {
                    return Err(EditorialPlanError::invalid(
                        format!("response.scene_phrases.{scene_id}"),
                        format!("invalid presentation_role {:?}", phrase.presentation_role),
                    ));
                }
                if !matches!(
                    phrase.candidate.as_str(),
                    "" | "title" | "bullet" | "opening"
                ) {
                    return Err(EditorialPlanError::invalid(
                        format!("response.scene_phrases.{scene_id}"),
                        format!("invalid candidate {:?}", phrase.candidate),
                    ));
                }
            }
        }
        Ok(())
    }
}
