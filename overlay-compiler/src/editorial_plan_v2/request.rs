//! The complete `editorial_plan.v2` request: everything the Rust planner needs
//! to make every overlay decision for a run.
//!
//! The envelope is additive. Fields carried by the first landed slice keep
//! their meaning; the sections added for TICKET-14 WP1 are the planner's real
//! inputs (seal extent, scene annotations, entity timeline, media refs, run
//! budgets and canonical scene windows). Nothing here is a precomputed
//! selection: `media` holds only materialized/verified refs and certified plate
//! or image candidates, and `annotations` holds certified semantic surfaces,
//! never the admitted item list.
//!
//! Request JSON is replayable: no pointers, no filesystem paths, no logger
//! state. Fields the planner genuinely treats as opaque render metadata
//! (`background`, `animation_directives`) stay `serde_json::Value`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::EditorialPlanError;
use super::{
    CHAPTER_MANIFEST_SCHEMA_VERSION, EDITORIAL_MANIFEST_SCHEMA_VERSION, EDITORIAL_PLAN_OPERATION,
    EDITORIAL_PLAN_V2_SCHEMA_VERSION, EDITORIAL_PROFILE_VERSION,
};

const OVERLAY_CATEGORIES: [&str; 10] = [
    "phrases",
    "images",
    "numbers",
    "quotes",
    "brand_text",
    "maps",
    "charts",
    "recaps",
    "titles",
    "bullets",
];

/// One complete overlay-plan compilation request.
///
/// `schema_version` is required and validated; every other section defaults so
/// the envelope can grow additively. A section that is present but malformed
/// fails closed, and [`EditorialPlanV2Request::validate_compilable`] requires
/// the sections the single call cannot run without.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialPlanV2Request {
    /// Optional operation echo. When present it must be
    /// `compile_editorial_plan`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub operation: String,
    pub schema_version: String,
    pub plan: PlanIdentityV2,
    pub language: String,
    #[serde(default)]
    pub source_language: String,
    /// Canonical plan extent and output profile. The seal derives `duration_ms`
    /// from these extents; Go must not re-derive a different duration.
    #[serde(default)]
    pub seal: Option<SealV2>,
    #[serde(default)]
    pub canvas: CanvasV2,
    #[serde(default)]
    pub editorial: Option<EditorialManifestV2>,
    #[serde(default)]
    pub chapters: Option<ChapterManifestV2>,
    #[serde(default)]
    pub scenes: Vec<SceneV2>,
    #[serde(default)]
    pub entity_timeline: Option<EntityTimelineV2>,
    #[serde(default)]
    pub media: Option<MediaV2>,
    #[serde(default)]
    pub budgets: Option<BudgetsV2>,
    /// Canonical `[start_us, end_us]` window per scene id. This is the trimmer
    /// and overlap resolver authority; it is not recomputed in Rust.
    #[serde(default)]
    pub scene_windows: BTreeMap<String, [i64; 2]>,
}

/// Immutable identity of one compiled plan. Motion entropy is derived from
/// this identity by the planner, never from wall-clock state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanIdentityV2 {
    pub plan_id: String,
    pub video_id: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub motion_seed: String,
    #[serde(default)]
    pub coherence_seed: String,
    pub width: i64,
    pub height: i64,
    pub fps_num: i64,
    pub fps_den: i64,
}

/// Canonical plan extent and the certified output profile of the artifact the
/// plan describes. `background` is renderer metadata and is copied verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealV2 {
    #[serde(default)]
    pub audio_duration_us: i64,
    #[serde(default)]
    pub output_profile_id: String,
    #[serde(default)]
    pub background: Option<Value>,
}

/// Run-level canvas controls that change overlay selection.
///
/// Rendering metadata (background, animation directives, opaque style
/// payloads) stays opaque and is intentionally not re-derived here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CanvasV2 {
    #[serde(default)]
    pub overlay_categories: BTreeMap<String, String>,
    /// Per-group admission caps keyed by the planner's group name.
    #[serde(default)]
    pub group_limits: BTreeMap<String, i64>,
    #[serde(default)]
    pub max_overlays_per_scene: i64,
    #[serde(default)]
    pub max_phrase_overlays: i64,
    #[serde(default)]
    pub max_image_overlays: i64,
    #[serde(default)]
    pub max_map_overlays: i64,
    /// Resolved unified map configuration. The zero value keeps the certified
    /// defaults (narrative grouping on, routes on, five pins per group).
    #[serde(default)]
    pub map_presentation: MapPresentationV2,
    #[serde(default)]
    pub animation_counts: BTreeMap<String, i64>,
    #[serde(default)]
    pub phrase_motions: Vec<String>,
    #[serde(default)]
    pub short_phrase_motion: String,
    #[serde(default)]
    pub long_phrase_motion: String,
    #[serde(default)]
    pub image_motions: Vec<String>,
    #[serde(default)]
    pub image_with_text_motions: Vec<String>,
    #[serde(default)]
    pub entity_style_id: String,
    /// Typed animation directives are validated by the animation catalog; the
    /// payload itself is renderer metadata and survives unchanged.
    #[serde(default)]
    pub animation_directives: Vec<Value>,
    #[serde(default)]
    pub animation_contract_version: i64,
    #[serde(default)]
    pub animation_catalog_digest: String,
    #[serde(default)]
    pub disable_editorial_overlays: bool,
    #[serde(default)]
    pub disable_recap_overlays: bool,
    #[serde(default)]
    pub enable_charts: bool,
    #[serde(default)]
    pub disable_chart_overlays: bool,
    #[serde(default)]
    pub maps_only: bool,
}

/// The resolved map presentation policy. Only the fields that change planner
/// decisions are typed; the renderer's own parameters stay in Go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MapPresentationV2 {
    #[serde(default)]
    pub map_type: String,
    #[serde(default)]
    pub camera_animation: String,
    #[serde(default)]
    pub label_animation: String,
    #[serde(default)]
    pub animation: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub max_pins_per_group: i64,
    #[serde(default)]
    pub group_nearby_places: bool,
    #[serde(default)]
    pub allow_route: bool,
}

/// One timed scene. `timeline_start_us`/`duration_us` are the canonical plan
/// offsets; they are the only authority for clipping a candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneV2 {
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub execution_mode: String,
    #[serde(default)]
    pub short_phrase_overlay: bool,
    #[serde(default)]
    pub overlay_categories: BTreeMap<String, String>,
    #[serde(default)]
    pub voiceover: Option<VoiceoverV2>,
    #[serde(default)]
    pub timeline_start_us: i64,
    #[serde(default)]
    pub duration_us: i64,
    /// Certified semantic surfaces with exact timing. These are candidates,
    /// not admitted items: Rust still applies lanes, budgets and selection.
    #[serde(default)]
    pub annotations: Option<SceneAnnotationsV2>,
}

/// The scene's certified voiceover projection. Only the timing is needed by
/// the editorial/chapter phase; audio paths and hashes stay in Go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceoverV2 {
    #[serde(default)]
    pub timing: Option<TimingV2>,
}

/// Certified word-level timing, copied verbatim from the audio authority.
/// It is never estimated from text length or scene duration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingV2 {
    pub duration_us: i64,
    #[serde(default)]
    pub words: Vec<WordTimingV2>,
}

/// One word boundary, in integer microseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTimingV2 {
    pub index: i64,
    #[serde(default)]
    pub text: String,
    pub start_us: i64,
    pub end_us: i64,
}

/// The planner's certified annotation surfaces for one scene.
///
/// `start_ms`/`end_ms` are the millisecond projection; `start_us`/`duration_us`
/// are the authoritative timing, in absolute plan microseconds (the same base
/// as `SceneV2::timeline_start_us`). Both are carried so the planner can
/// reproduce the current admitted windows without re-deriving them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SceneAnnotationsV2 {
    #[serde(default)]
    pub phrases: Vec<TimedAnnotationV2>,
    #[serde(default)]
    pub keywords: Vec<TimedAnnotationV2>,
    #[serde(default)]
    pub images: Vec<ImageCandidateV2>,
    #[serde(default)]
    pub numbers: Vec<TimedAnnotationV2>,
    #[serde(default)]
    pub chart_numbers: Vec<TimedAnnotationV2>,
    #[serde(default)]
    pub brand_texts: Vec<TimedAnnotationV2>,
    #[serde(default)]
    pub quotes: Vec<TimedAnnotationV2>,
    #[serde(default)]
    pub products: Vec<ImageCandidateV2>,
    #[serde(default)]
    pub logos: Vec<ImageCandidateV2>,
    #[serde(default)]
    pub maps: Vec<MapCandidateV2>,
    #[serde(default)]
    pub multi_entity_groups: Vec<MultiEntityGroupV2>,
    /// Absolute plan end of the owning scene; date cards are capped there.
    #[serde(default)]
    pub end_us: i64,
}

/// One grounded text surface with exact timing. `presentation_role` and
/// `editorial_candidate` are producer hints for the editorial lanes; they do
/// not create a new renderer kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedAnnotationV2 {
    pub text: String,
    #[serde(default, rename = "type")]
    pub annotation_type: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default)]
    pub start_us: i64,
    #[serde(default)]
    pub duration_us: i64,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub presentation_role: String,
    #[serde(default)]
    pub editorial_candidate: String,
}

/// One certified image candidate. `url` must be fetchable: a semantic
/// placeholder (`semantic/...`) is not a render asset and fails validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageCandidateV2 {
    #[serde(default)]
    pub asset_id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default)]
    pub start_us: i64,
    #[serde(default)]
    pub duration_us: i64,
    #[serde(default)]
    pub score: f64,
}

/// One geocoder-validated place candidate. Rust never invents a coordinate,
/// plate or asset: the plate must be covered by the certified media refs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapCandidateV2 {
    pub entity_id: String,
    pub label: String,
    #[serde(default)]
    pub scope: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default)]
    pub start_us: i64,
    #[serde(default)]
    pub duration_us: i64,
    #[serde(default)]
    pub score: f64,
}

/// Explicitly authored adjacent image/phrase group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiEntityGroupV2 {
    pub group_id: String,
    #[serde(default)]
    pub scene_id: String,
    #[serde(default)]
    pub group_type: String,
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub items: Vec<MultiEntityCandidateV2>,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default)]
    pub focus_strategy: String,
}

/// One member of a multi-entity group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiEntityCandidateV2 {
    pub entity_id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub asset_ref: Option<AssetRefV2>,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
}

/// Canonical, location-free media identity plus the fetchable URL. It mirrors
/// the `capmodel.OverlayAssetRef` JSON tags; `local_path` is deliberately not
/// on the wire because Rust never reads the filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRefV2 {
    pub asset_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media_type: String,
}

/// The canonical entity timeline: scene-level entity occurrences projected on
/// the final combined timeline. Every occurrence carries real word-timing
/// boundaries, never text-length estimates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityTimelineV2 {
    #[serde(default = "default_entity_timeline_version")]
    pub version: i64,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub duration_us: i64,
    #[serde(default)]
    pub scenes: Vec<SceneEntityTimelineV2>,
}

fn default_entity_timeline_version() -> i64 {
    1
}

/// One scene's entity occurrences plus the scene's canonical offset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneEntityTimelineV2 {
    pub scene_id: String,
    #[serde(default)]
    pub scene_index: i64,
    #[serde(default)]
    pub timeline_start_us: i64,
    #[serde(default)]
    pub voiceover_asset_id: String,
    #[serde(default)]
    pub entities: Vec<EntityOccurrenceV2>,
}

/// One grounded entity occurrence. The local→global mapping is authoritative
/// and is validated before the planner sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityOccurrenceV2 {
    pub entity_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    #[serde(default)]
    pub scene_id: String,
    #[serde(default)]
    pub scene_index: i64,
    #[serde(default)]
    pub text_start: i64,
    #[serde(default)]
    pub text_end: i64,
    #[serde(default)]
    pub word_start: i64,
    #[serde(default)]
    pub word_end: i64,
    #[serde(default)]
    pub local_start_us: i64,
    #[serde(default)]
    pub local_end_us: i64,
    #[serde(default)]
    pub timeline_start_us: i64,
    #[serde(default)]
    pub audio_start_us: i64,
    #[serde(default)]
    pub audio_end_us: i64,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub canonical_entity_id: String,
}

/// Materialized/verified media and certified scene image candidates. This
/// section never carries a pre-admitted overlay item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaV2 {
    #[serde(default)]
    pub materialized_assets: Vec<AssetRefV2>,
    #[serde(default)]
    pub verified_entity_assets: Vec<VerifiedEntityAssetV2>,
    #[serde(default)]
    pub scene_image_candidates: Vec<SceneImageCandidateSetV2>,
}

/// The verified assets one canonical entity may render with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifiedEntityAssetV2 {
    pub entity_id: String,
    #[serde(default)]
    pub canonical_entity_id: String,
    #[serde(default)]
    pub assets: Vec<AssetRefV2>,
}

/// One scene's certified image candidates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneImageCandidateSetV2 {
    pub scene_id: String,
    #[serde(default)]
    pub candidates: Vec<ImageCandidateV2>,
}

/// The resolved production budgets. These are the requested values, not
/// Go-computed candidate counts: Rust owns admission and drop accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BudgetsV2 {
    #[serde(default)]
    pub phrase_overlays: i64,
    #[serde(default)]
    pub image_overlays: i64,
    #[serde(default)]
    pub map_overlays: i64,
    #[serde(default)]
    pub max_per_scene: i64,
    #[serde(default)]
    pub short_phrase: i64,
    #[serde(default)]
    pub long_phrase: i64,
    #[serde(default)]
    pub bullet: i64,
}

/// The editorial manifest is a pure projection of the Rust extractor. The
/// request carries it so the scene projection can ground titles and bullets
/// without a second extraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialManifestV2 {
    pub schema_version: String,
    #[serde(default)]
    pub source_language: String,
    #[serde(default)]
    pub profile_version: String,
    #[serde(default)]
    pub scenes: Vec<EditorialSceneV2>,
    #[serde(default)]
    pub certified: bool,
    #[serde(default)]
    pub fingerprint: String,
}

/// One editorial scene, in manifest order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorialSceneV2 {
    pub scene_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub title_status: String,
    #[serde(default)]
    pub title_source: String,
    #[serde(default)]
    pub globally_indexed: bool,
    #[serde(default)]
    pub bullets: Vec<EditorialBulletV2>,
}

/// One extractive editorial bullet. Sentence indices are transcript-global
/// when the manifest says so, otherwise scene-local.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorialBulletV2 {
    pub sentence_start: i64,
    pub sentence_end: i64,
    pub text: String,
}

/// The chapter manifest is an extractive projection of the same transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterManifestV2 {
    pub schema_version: String,
    #[serde(default)]
    pub chapters: Vec<ChapterV2>,
}

/// One chapter; only its bullets participate in the overlay projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterV2 {
    #[serde(default)]
    pub bullets: Vec<ChapterBulletV2>,
}

/// One chapter bullet. The text must ground verbatim in the owning body scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterBulletV2 {
    pub text: String,
    #[serde(default)]
    pub start_sentence: i64,
    #[serde(default)]
    pub end_sentence: i64,
}

impl EditorialPlanV2Request {
    /// Structural, fail-closed validation. A request that fails here must not
    /// reach any decision code: a malformed manifest is a caller bug, not a
    /// silent no-op.
    pub fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != EDITORIAL_PLAN_V2_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            });
        }
        if !self.operation.trim().is_empty() && self.operation.trim() != EDITORIAL_PLAN_OPERATION {
            return Err(EditorialPlanError::invalid(
                "operation",
                format!("expected {EDITORIAL_PLAN_OPERATION:?}"),
            ));
        }
        if self.plan.plan_id.trim().is_empty() || self.plan.video_id.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                "plan",
                "plan_id and video_id are required",
            ));
        }
        if self.plan.width <= 0 || self.plan.height <= 0 {
            return Err(EditorialPlanError::invalid(
                "plan",
                "width and height must be positive",
            ));
        }
        if self.plan.fps_num <= 0 || self.plan.fps_den <= 0 {
            return Err(EditorialPlanError::invalid(
                "plan",
                "fps_num and fps_den must be positive",
            ));
        }
        if self.language.trim().is_empty() {
            return Err(EditorialPlanError::invalid("language", "is required"));
        }
        if let Some(seal) = &self.seal {
            if seal.audio_duration_us < 0 {
                return Err(EditorialPlanError::invalid(
                    "seal.audio_duration_us",
                    "must not be negative",
                ));
            }
        }
        self.canvas.validate()?;

        let mut scene_ids = BTreeSet::new();
        for scene in &self.scenes {
            let id = scene.id.trim();
            if id.is_empty() {
                return Err(EditorialPlanError::invalid(
                    "scenes",
                    "scene id is required",
                ));
            }
            if !scene_ids.insert(id.to_string()) {
                return Err(EditorialPlanError::invalid(
                    "scenes",
                    format!("duplicate scene id {id:?}"),
                ));
            }
            if scene.duration_ms < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("scenes.{id}.duration_ms"),
                    "must not be negative",
                ));
            }
            if scene.timeline_start_us < 0 || scene.duration_us < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("scenes.{id}"),
                    "canonical bounds must not be negative",
                ));
            }
            validate_categories(id, &scene.overlay_categories)?;
            if let Some(voiceover) = &scene.voiceover {
                if let Some(timing) = &voiceover.timing {
                    validate_timing(id, timing)?;
                }
            }
            if let Some(annotations) = &scene.annotations {
                validate_annotations(id, scene, annotations)?;
            }
        }
        for scene_id in self.scene_windows.keys() {
            if !scene_ids.contains(scene_id.trim()) {
                return Err(EditorialPlanError::invalid(
                    "scene_windows",
                    format!("unknown scene id {scene_id:?}"),
                ));
            }
        }
        let known_scenes: BTreeSet<&str> = scene_ids.iter().map(String::as_str).collect();
        if let Some(timeline) = &self.entity_timeline {
            timeline.validate(&known_scenes)?;
        }
        if let Some(media) = &self.media {
            media.validate(&known_scenes)?;
        }
        if let Some(budgets) = &self.budgets {
            budgets.validate()?;
        }
        if let Some(editorial) = &self.editorial {
            editorial.validate()?;
        }
        if let Some(chapters) = &self.chapters {
            chapters.validate()?;
        }
        Ok(())
    }

    /// Requires the sections the single-call planner cannot run without.
    ///
    /// [`Self::validate`] accepts the partial envelope the first landed slice
    /// shipped (identity, scenes, manifests). The one-call compiler is stricter:
    /// a missing extent, media index, budget or entity timeline is an error,
    /// never an implicit empty fallback.
    pub fn validate_compilable(&self) -> Result<(), EditorialPlanError> {
        self.validate()?;
        if self.seal.is_none() {
            return Err(EditorialPlanError::MissingSection("seal"));
        }
        if self.media.is_none() {
            return Err(EditorialPlanError::MissingSection("media"));
        }
        if self.budgets.is_none() {
            return Err(EditorialPlanError::MissingSection("budgets"));
        }
        if self.entity_timeline.is_none() {
            return Err(EditorialPlanError::MissingSection("entity_timeline"));
        }
        if self.scenes.is_empty() {
            return Err(EditorialPlanError::invalid(
                "scenes",
                "at least one scene is required",
            ));
        }
        for scene in &self.scenes {
            if scene.duration_us <= 0 {
                continue;
            }
            let Some(window) = self.scene_windows.get(&scene.id) else {
                return Err(EditorialPlanError::invalid(
                    "scene_windows",
                    format!("missing canonical window for scene {:?}", scene.id),
                ));
            };
            if window[1] - window[0] != scene.duration_us {
                return Err(EditorialPlanError::invalid(
                    format!("scene_windows.{}", scene.id),
                    "window extent does not match the scene duration",
                ));
            }
            if window[0] != scene.timeline_start_us {
                return Err(EditorialPlanError::invalid(
                    format!("scene_windows.{}", scene.id),
                    "window start does not match the scene timeline start",
                ));
            }
        }
        Ok(())
    }
}

impl CanvasV2 {
    fn validate(&self) -> Result<(), EditorialPlanError> {
        validate_categories("canvas", &self.overlay_categories)?;
        for (name, limit) in &self.group_limits {
            if name.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "canvas.group_limits",
                    "empty group name",
                ));
            }
            if *limit < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("canvas.group_limits.{name}"),
                    "must not be negative",
                ));
            }
        }
        for (lane, count) in &self.animation_counts {
            if lane.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "canvas.animation_counts",
                    "empty lane name",
                ));
            }
            if *count < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("canvas.animation_counts.{lane}"),
                    "must not be negative",
                ));
            }
        }
        for (field, value) in [
            ("max_overlays_per_scene", self.max_overlays_per_scene),
            ("max_phrase_overlays", self.max_phrase_overlays),
            ("max_image_overlays", self.max_image_overlays),
            ("max_map_overlays", self.max_map_overlays),
            (
                "map_presentation.max_pins_per_group",
                self.map_presentation.max_pins_per_group,
            ),
        ] {
            if value < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("canvas.{field}"),
                    "must not be negative",
                ));
            }
        }
        Ok(())
    }
}

impl BudgetsV2 {
    fn validate(&self) -> Result<(), EditorialPlanError> {
        for (field, value) in [
            ("phrase_overlays", self.phrase_overlays),
            ("image_overlays", self.image_overlays),
            ("map_overlays", self.map_overlays),
            ("max_per_scene", self.max_per_scene),
            ("short_phrase", self.short_phrase),
            ("long_phrase", self.long_phrase),
            ("bullet", self.bullet),
        ] {
            if value < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("budgets.{field}"),
                    "must not be negative",
                ));
            }
        }
        Ok(())
    }
}

impl EntityTimelineV2 {
    fn validate(&self, known_scenes: &BTreeSet<&str>) -> Result<(), EditorialPlanError> {
        if self.version < 1 {
            return Err(EditorialPlanError::invalid(
                "entity_timeline.version",
                "must be at least 1",
            ));
        }
        if self.duration_us < 0 {
            return Err(EditorialPlanError::invalid(
                "entity_timeline.duration_us",
                "must not be negative",
            ));
        }
        let mut scene_ids = BTreeSet::new();
        let mut previous_index = -1_i64;
        for scene in &self.scenes {
            if scene.scene_id.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    "scene id is required",
                ));
            }
            if !scene_ids.insert(scene.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    format!("duplicate scene {:?}", scene.scene_id),
                ));
            }
            if !known_scenes.contains(scene.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    format!("unknown scene {:?}", scene.scene_id),
                ));
            }
            if scene.scene_index <= previous_index {
                return Err(EditorialPlanError::invalid(
                    "entity_timeline.scenes",
                    "scenes are not strictly ordered by index",
                ));
            }
            previous_index = scene.scene_index;
            if scene.timeline_start_us < 0 {
                return Err(EditorialPlanError::invalid(
                    format!(
                        "entity_timeline.scenes.{}.timeline_start_us",
                        scene.scene_id
                    ),
                    "must not be negative",
                ));
            }
            for occurrence in &scene.entities {
                occurrence.validate(&scene.scene_id, scene.timeline_start_us, self.duration_us)?;
            }
        }
        Ok(())
    }
}

impl EntityOccurrenceV2 {
    fn validate(
        &self,
        scene_id: &str,
        scene_timeline_start_us: i64,
        timeline_duration_us: i64,
    ) -> Result<(), EditorialPlanError> {
        let label = format!("entity_timeline entity {:?}", self.name);
        if self.entity_id.trim().is_empty()
            || self.name.trim().is_empty()
            || self.entity_type.trim().is_empty()
        {
            return Err(EditorialPlanError::invalid(
                label,
                "entity_id, name and type are required",
            ));
        }
        if !self.scene_id.is_empty() && self.scene_id != scene_id {
            return Err(EditorialPlanError::invalid(
                label,
                format!("occurrence scene {:?} does not own it", self.scene_id),
            ));
        }
        if self.text_start < 0 || self.text_end <= self.text_start {
            return Err(EditorialPlanError::invalid(label, "invalid text span"));
        }
        if self.word_start < 0 || self.word_end < self.word_start {
            return Err(EditorialPlanError::invalid(label, "invalid word span"));
        }
        if self.local_start_us < 0 || self.local_end_us < self.local_start_us {
            return Err(EditorialPlanError::invalid(label, "inverted local span"));
        }
        if self.timeline_start_us != scene_timeline_start_us {
            return Err(EditorialPlanError::invalid(
                label,
                "carries a different timeline offset than its scene",
            ));
        }
        if self.audio_start_us != self.timeline_start_us + self.local_start_us
            || self.audio_end_us != self.timeline_start_us + self.local_end_us
        {
            return Err(EditorialPlanError::invalid(
                label,
                "audio window is not the canonical local->global mapping",
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(EditorialPlanError::invalid(
                label,
                "confidence is outside [0,1]",
            ));
        }
        if self.audio_end_us > timeline_duration_us {
            return Err(EditorialPlanError::invalid(
                label,
                "audio window ends past the timeline duration",
            ));
        }
        Ok(())
    }
}

impl MediaV2 {
    fn validate(&self, known_scenes: &BTreeSet<&str>) -> Result<(), EditorialPlanError> {
        for asset in &self.materialized_assets {
            validate_asset_ref("media.materialized_assets", asset)?;
        }
        for verified in &self.verified_entity_assets {
            if verified.entity_id.trim().is_empty()
                && verified.canonical_entity_id.trim().is_empty()
            {
                return Err(EditorialPlanError::invalid(
                    "media.verified_entity_assets",
                    "entity_id or canonical_entity_id is required",
                ));
            }
            for asset in &verified.assets {
                validate_asset_ref("media.verified_entity_assets", asset)?;
            }
        }
        let mut seen_scenes = BTreeSet::new();
        for set in &self.scene_image_candidates {
            if !known_scenes.contains(set.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "media.scene_image_candidates",
                    format!("unknown scene {:?}", set.scene_id),
                ));
            }
            if !seen_scenes.insert(set.scene_id.as_str()) {
                return Err(EditorialPlanError::invalid(
                    "media.scene_image_candidates",
                    format!("duplicate scene {:?}", set.scene_id),
                ));
            }
            for candidate in &set.candidates {
                validate_image_candidate("media.scene_image_candidates", candidate)?;
            }
        }
        Ok(())
    }
}

fn validate_asset_ref(label: &str, asset: &AssetRefV2) -> Result<(), EditorialPlanError> {
    if asset.asset_id.trim().is_empty() {
        return Err(EditorialPlanError::invalid(label, "asset_id is required"));
    }
    if !asset.sha256.trim().is_empty() && !is_lower_hex_64(&asset.sha256) {
        return Err(EditorialPlanError::invalid(
            label,
            "sha256 must be 64 lowercase hex characters",
        ));
    }
    reject_placeholder_url(label, &asset.url)
}

fn validate_image_candidate(
    label: &str,
    candidate: &ImageCandidateV2,
) -> Result<(), EditorialPlanError> {
    if candidate.asset_id.trim().is_empty() && candidate.url.trim().is_empty() {
        return Err(EditorialPlanError::invalid(
            label,
            "image candidate needs an asset_id or a url",
        ));
    }
    if !candidate.sha256.trim().is_empty() && !is_lower_hex_64(&candidate.sha256) {
        return Err(EditorialPlanError::invalid(
            label,
            "sha256 must be 64 lowercase hex characters",
        ));
    }
    if candidate.duration_us < 0 || candidate.start_us < 0 {
        return Err(EditorialPlanError::invalid(label, "negative image window"));
    }
    reject_placeholder_url(label, &candidate.url)
}

/// A semantic catalog reference is a database identity until its bytes are
/// materialized. It can never be a render asset, so it is a contract error
/// here instead of a silently skipped item later.
fn reject_placeholder_url(label: &str, url: &str) -> Result<(), EditorialPlanError> {
    let url = url.trim();
    if url.starts_with("assets/semantic/") || url.starts_with("semantic/") {
        return Err(EditorialPlanError::invalid(
            label,
            format!("unmaterialized semantic placeholder URL {url:?}"),
        ));
    }
    Ok(())
}

fn validate_annotations(
    scene_id: &str,
    scene: &SceneV2,
    annotations: &SceneAnnotationsV2,
) -> Result<(), EditorialPlanError> {
    let label = format!("scenes.{scene_id}.annotations");
    for (name, spans) in [
        ("phrases", &annotations.phrases),
        ("keywords", &annotations.keywords),
        ("numbers", &annotations.numbers),
        ("chart_numbers", &annotations.chart_numbers),
        ("brand_texts", &annotations.brand_texts),
        ("quotes", &annotations.quotes),
    ] {
        for span in spans {
            if span.text.trim().is_empty() {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    "annotation text is required",
                ));
            }
            if span.start_us < 0 || span.duration_us < 0 {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    "negative annotation window",
                ));
            }
            if span.end_ms < span.start_ms && span.end_ms != 0 {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    "annotation end_ms precedes start_ms",
                ));
            }
        }
    }
    for (name, candidates) in [
        ("images", &annotations.images),
        ("products", &annotations.products),
        ("logos", &annotations.logos),
    ] {
        for candidate in candidates {
            validate_image_candidate(&format!("{label}.{name}"), candidate)?;
        }
    }
    for map in &annotations.maps {
        if map.entity_id.trim().is_empty() || map.label.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                format!("{label}.maps"),
                "entity_id and label are required",
            ));
        }
        if !map.latitude.is_finite()
            || !map.longitude.is_finite()
            || !(-90.0..=90.0).contains(&map.latitude)
            || !(-180.0..=180.0).contains(&map.longitude)
        {
            return Err(EditorialPlanError::invalid(
                format!("{label}.maps"),
                "coordinates are not valid WGS84 degrees",
            ));
        }
        if map.start_us < 0 || map.duration_us < 0 {
            return Err(EditorialPlanError::invalid(
                format!("{label}.maps"),
                "negative map window",
            ));
        }
    }
    for group in &annotations.multi_entity_groups {
        if group.group_id.trim().is_empty() {
            return Err(EditorialPlanError::invalid(
                format!("{label}.multi_entity_groups"),
                "group_id is required",
            ));
        }
        if !group.scene_id.is_empty() && group.scene_id != scene_id {
            return Err(EditorialPlanError::invalid(
                format!("{label}.multi_entity_groups"),
                format!("group {:?} belongs to another scene", group.group_id),
            ));
        }
        if group.end_ms < group.start_ms {
            return Err(EditorialPlanError::invalid(
                format!("{label}.multi_entity_groups"),
                "group end precedes start",
            ));
        }
    }
    let window_start = scene.timeline_start_us;
    let window_end = window_start.saturating_add(scene.duration_us);
    for (name, spans) in [
        ("phrases", &annotations.phrases),
        ("keywords", &annotations.keywords),
        ("numbers", &annotations.numbers),
        ("chart_numbers", &annotations.chart_numbers),
        ("brand_texts", &annotations.brand_texts),
        ("quotes", &annotations.quotes),
    ] {
        for span in spans {
            if span.duration_us > 0
                && scene.duration_us > 0
                && (span.start_us < window_start
                    || span.start_us.saturating_add(span.duration_us) > window_end)
            {
                return Err(EditorialPlanError::invalid(
                    format!("{label}.{name}"),
                    format!("annotation {:?} escapes the owning scene window", span.text),
                ));
            }
        }
    }
    if annotations.end_us < 0 {
        return Err(EditorialPlanError::invalid(
            format!("{label}.end_us"),
            "must not be negative",
        ));
    }
    if annotations.end_us > 0 && scene.duration_us > 0 && annotations.end_us > window_end {
        return Err(EditorialPlanError::invalid(
            format!("{label}.end_us"),
            "ends past the owning scene window",
        ));
    }
    Ok(())
}

impl EditorialManifestV2 {
    fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != EDITORIAL_MANIFEST_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: EDITORIAL_MANIFEST_SCHEMA_VERSION.to_string(),
            });
        }
        if !self
            .profile_version
            .eq_ignore_ascii_case(EDITORIAL_PROFILE_VERSION)
        {
            return Err(EditorialPlanError::invalid(
                "editorial.profile_version",
                format!("unsupported profile {:?}", self.profile_version),
            ));
        }
        if !is_lower_hex_64(&self.fingerprint) {
            return Err(EditorialPlanError::invalid(
                "editorial.fingerprint",
                "must be 64 lowercase hex characters",
            ));
        }
        let mut scene_ids = BTreeSet::new();
        for scene in &self.scenes {
            let id = scene.scene_id.trim();
            if id.is_empty() {
                return Err(EditorialPlanError::invalid(
                    "editorial.scenes",
                    "scene_id is required",
                ));
            }
            if !scene_ids.insert(id.to_string()) {
                return Err(EditorialPlanError::invalid(
                    "editorial.scenes",
                    format!("duplicate editorial scene {id:?}"),
                ));
            }
            for bullet in &scene.bullets {
                if bullet.sentence_start < 0 || bullet.sentence_end <= bullet.sentence_start {
                    return Err(EditorialPlanError::invalid(
                        format!("editorial.scenes.{id}.bullets"),
                        "invalid sentence span",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ChapterManifestV2 {
    fn validate(&self) -> Result<(), EditorialPlanError> {
        if self.schema_version != CHAPTER_MANIFEST_SCHEMA_VERSION {
            return Err(EditorialPlanError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
                expected: CHAPTER_MANIFEST_SCHEMA_VERSION.to_string(),
            });
        }
        for chapter in &self.chapters {
            for bullet in &chapter.bullets {
                if bullet.text.trim().is_empty() {
                    return Err(EditorialPlanError::invalid(
                        "chapters.bullets",
                        "chapter bullet text is required",
                    ));
                }
                if bullet.start_sentence < 0 || bullet.end_sentence <= bullet.start_sentence {
                    return Err(EditorialPlanError::invalid(
                        "chapters.bullets",
                        "invalid sentence span",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_categories(
    label: &str,
    categories: &BTreeMap<String, String>,
) -> Result<(), EditorialPlanError> {
    for (category, toggle) in categories {
        if !OVERLAY_CATEGORIES.contains(&category.as_str()) {
            return Err(EditorialPlanError::invalid(
                format!("{label}.overlay_categories"),
                format!("unknown overlay category {category:?}"),
            ));
        }
        if !matches!(toggle.as_str(), "" | "default" | "enabled" | "disabled") {
            return Err(EditorialPlanError::invalid(
                format!("{label}.overlay_categories.{category}"),
                format!("invalid toggle {toggle:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_timing(scene_id: &str, timing: &TimingV2) -> Result<(), EditorialPlanError> {
    let label = format!("scenes.{scene_id}.voiceover.timing");
    if timing.duration_us <= 0 {
        return Err(EditorialPlanError::invalid(
            label,
            "duration must be positive",
        ));
    }
    if timing.words.is_empty() {
        return Err(EditorialPlanError::invalid(label, "has no words"));
    }
    let mut previous_end = 0_i64;
    for (position, word) in timing.words.iter().enumerate() {
        if word.index != position as i64 {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word index {} at position {position}", word.index),
            ));
        }
        if word.start_us < 0 || word.end_us < word.start_us {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word {position} has an invalid range"),
            ));
        }
        if position > 0 && word.start_us < previous_end {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word {position} is not monotonic"),
            ));
        }
        if word.end_us > timing.duration_us {
            return Err(EditorialPlanError::invalid(
                label,
                format!("word {position} exceeds the audio duration"),
            ));
        }
        previous_end = word.end_us;
    }
    Ok(())
}

/// True for a lowercase 64-character SHA-256 spelling. Anything else is a
/// different identity and is rejected rather than normalized.
pub fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::{
        BudgetsV2, EditorialBulletV2, EditorialManifestV2, EditorialPlanV2Request,
        EditorialSceneV2, EntityTimelineV2, ImageCandidateV2, PlanIdentityV2, SceneV2, SealV2,
        TimingV2, VoiceoverV2, WordTimingV2, EDITORIAL_MANIFEST_SCHEMA_VERSION,
        EDITORIAL_PLAN_V2_SCHEMA_VERSION, EDITORIAL_PROFILE_VERSION,
    };
    use std::collections::BTreeMap;

    fn plan() -> PlanIdentityV2 {
        PlanIdentityV2 {
            plan_id: "plan-1".to_string(),
            video_id: "video-1".to_string(),
            project_id: "project-1".to_string(),
            motion_seed: "seed-1".to_string(),
            coherence_seed: String::new(),
            width: 1920,
            height: 1080,
            fps_num: 24,
            fps_den: 1,
        }
    }

    fn timing() -> TimingV2 {
        TimingV2 {
            duration_us: 2_000_000,
            words: vec![
                WordTimingV2 {
                    index: 0,
                    text: "Nike".to_string(),
                    start_us: 100_000,
                    end_us: 400_000,
                },
                WordTimingV2 {
                    index: 1,
                    text: "corre".to_string(),
                    start_us: 400_000,
                    end_us: 800_000,
                },
            ],
        }
    }

    fn scene() -> SceneV2 {
        SceneV2 {
            id: "scene-1".to_string(),
            text: "Nike corre. La statua è al museo.".to_string(),
            duration_ms: 2_000,
            execution_mode: "body".to_string(),
            short_phrase_overlay: false,
            overlay_categories: BTreeMap::new(),
            voiceover: Some(VoiceoverV2 {
                timing: Some(timing()),
            }),
            timeline_start_us: 0,
            duration_us: 2_000_000,
            annotations: None,
        }
    }

    fn editorial() -> EditorialManifestV2 {
        EditorialManifestV2 {
            schema_version: EDITORIAL_MANIFEST_SCHEMA_VERSION.to_string(),
            source_language: "it".to_string(),
            profile_version: EDITORIAL_PROFILE_VERSION.to_string(),
            certified: true,
            fingerprint: "a".repeat(64),
            scenes: vec![EditorialSceneV2 {
                scene_id: "scene-1".to_string(),
                title: Some("Nike corre".to_string()),
                title_status: "resolved".to_string(),
                title_source: "sentence".to_string(),
                globally_indexed: false,
                bullets: vec![EditorialBulletV2 {
                    sentence_start: 0,
                    sentence_end: 1,
                    text: "Nike corre.".to_string(),
                }],
            }],
        }
    }

    fn request() -> EditorialPlanV2Request {
        EditorialPlanV2Request {
            operation: "compile_editorial_plan".to_string(),
            schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            plan: plan(),
            language: "it".to_string(),
            source_language: "it".to_string(),
            seal: Some(SealV2 {
                audio_duration_us: 2_000_000,
                output_profile_id: "assemblyready.alpha.v1".to_string(),
                background: None,
            }),
            canvas: Default::default(),
            editorial: Some(editorial()),
            chapters: None,
            scenes: vec![scene()],
            entity_timeline: Some(EntityTimelineV2 {
                version: 1,
                project_id: "project-1".to_string(),
                language: "it".to_string(),
                duration_us: 2_000_000,
                scenes: Vec::new(),
            }),
            media: Some(Default::default()),
            budgets: Some(BudgetsV2 {
                phrase_overlays: 8,
                image_overlays: 5,
                map_overlays: 3,
                max_per_scene: 3,
                short_phrase: 3,
                long_phrase: 5,
                bullet: 2,
            }),
            scene_windows: BTreeMap::from([("scene-1".to_string(), [0, 2_000_000])]),
        }
    }

    #[test]
    fn v2_request_round_trips_through_json() {
        let request = request();
        request.validate().expect("valid request");
        let encoded = serde_json::to_string(&request).expect("encode request");
        let decoded: EditorialPlanV2Request =
            serde_json::from_str(&encoded).expect("decode request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn v2_rejects_unknown_schema_version() {
        let mut request = request();
        request.schema_version = "editorial_plan.v1".to_string();
        assert!(matches!(
            request.validate().expect_err("unknown schema"),
            super::super::EditorialPlanError::UnsupportedSchemaVersion { .. }
        ));
    }

    #[test]
    fn v2_rejects_duplicate_scene_ids() {
        let mut request = request();
        request.scenes.push(request.scenes[0].clone());
        assert!(request
            .validate()
            .expect_err("duplicate scene")
            .to_string()
            .contains("duplicate scene id"));
    }

    #[test]
    fn v2_rejects_non_monotonic_word_timing() {
        let mut request = request();
        let timing = request.scenes[0]
            .voiceover
            .as_mut()
            .expect("voiceover")
            .timing
            .as_mut()
            .expect("timing");
        timing.words[1].start_us = 50_000;
        assert!(request
            .validate()
            .expect_err("non-monotonic")
            .to_string()
            .contains("not monotonic"));
    }

    #[test]
    fn v2_rejects_malformed_editorial_fingerprint() {
        let mut request = request();
        request.editorial.as_mut().expect("editorial").fingerprint = "A".repeat(64);
        assert!(request
            .validate()
            .expect_err("fingerprint")
            .to_string()
            .contains("64 lowercase hex"));
    }

    #[test]
    fn v2_rejects_invalid_overlay_category() {
        let mut request = request();
        request
            .canvas
            .overlay_categories
            .insert("unknown".to_string(), "enabled".to_string());
        assert!(request
            .validate()
            .expect_err("category")
            .to_string()
            .contains("unknown overlay category"));
    }

    #[test]
    fn v2_compilable_requires_every_input_section() {
        for missing in ["seal", "media", "budgets", "entity_timeline"] {
            let mut request = request();
            match missing {
                "seal" => request.seal = None,
                "media" => request.media = None,
                "budgets" => request.budgets = None,
                "entity_timeline" => request.entity_timeline = None,
                _ => unreachable!(),
            }
            assert!(
                request
                    .validate_compilable()
                    .expect_err("missing section")
                    .to_string()
                    .contains(missing),
                "{missing} must be required"
            );
        }
    }

    #[test]
    fn v2_rejects_unmaterialized_semantic_asset() {
        let mut request = request();
        request.media = Some(super::MediaV2 {
            materialized_assets: vec![super::AssetRefV2 {
                asset_id: "asset-1".to_string(),
                url: "semantic/hero.png".to_string(),
                sha256: "b".repeat(64),
                media_type: "image/png".to_string(),
            }],
            ..Default::default()
        });
        assert!(request
            .validate()
            .expect_err("placeholder")
            .to_string()
            .contains("unmaterialized semantic placeholder"));
    }

    #[test]
    fn v2_rejects_scene_window_mismatch() {
        let mut request = request();
        request
            .scene_windows
            .insert("scene-1".to_string(), [0, 1_000_000]);
        assert!(request
            .validate_compilable()
            .expect_err("window")
            .to_string()
            .contains("does not match the scene duration"));
    }

    #[test]
    fn v2_rejects_entity_occurrence_with_a_foreign_timeline_offset() {
        let mut request = request();
        let occurrence_scene = super::SceneEntityTimelineV2 {
            scene_id: "scene-1".to_string(),
            scene_index: 0,
            timeline_start_us: 0,
            voiceover_asset_id: String::new(),
            entities: vec![super::EntityOccurrenceV2 {
                entity_id: "person:nike".to_string(),
                name: "Nike".to_string(),
                entity_type: "ORG".to_string(),
                scene_id: "scene-1".to_string(),
                scene_index: 0,
                text_start: 0,
                text_end: 4,
                word_start: 0,
                word_end: 0,
                local_start_us: 100_000,
                local_end_us: 400_000,
                timeline_start_us: 0,
                audio_start_us: 100_000,
                audio_end_us: 400_000,
                confidence: 0.9,
                role: "named_entity".to_string(),
                canonical_entity_id: "org:nike".to_string(),
            }],
        };
        request
            .entity_timeline
            .as_mut()
            .expect("timeline")
            .scenes
            .push(occurrence_scene);
        request.validate().expect("canonical mapping is valid");
        request.entity_timeline.as_mut().expect("timeline").scenes[0].entities[0]
            .timeline_start_us = 500_000;
        assert!(request
            .validate()
            .expect_err("offset drift")
            .to_string()
            .contains("different timeline offset"));
    }

    #[test]
    fn v2_rejects_out_of_range_map_coordinates() {
        let mut request = request();
        request.scenes[0].annotations = Some(super::SceneAnnotationsV2 {
            maps: vec![super::MapCandidateV2 {
                entity_id: "place:paris".to_string(),
                label: "Paris".to_string(),
                scope: "city".to_string(),
                latitude: 91.0,
                longitude: 2.35,
                start_ms: 0,
                end_ms: 1_000,
                start_us: 0,
                duration_us: 1_000_000,
                score: 0.9,
            }],
            ..Default::default()
        });
        assert!(request
            .validate()
            .expect_err("latitude")
            .to_string()
            .contains("WGS84"));
    }

    #[test]
    fn v2_rejects_a_negative_image_window() {
        let mut request = request();
        request.scenes[0].annotations = Some(super::SceneAnnotationsV2 {
            images: vec![ImageCandidateV2 {
                asset_id: "asset-1".to_string(),
                url: "https://cdn.example/hero.png".to_string(),
                sha256: "c".repeat(64),
                media_type: "image/png".to_string(),
                start_ms: 0,
                end_ms: 1_000,
                start_us: -1,
                duration_us: 1_000_000,
                score: 0.8,
            }],
            ..Default::default()
        });
        assert!(request
            .validate()
            .expect_err("negative window")
            .to_string()
            .contains("negative image window"));
    }
}
