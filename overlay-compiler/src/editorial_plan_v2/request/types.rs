use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Producer-side media facts and the run-level image ceiling. When present,
    /// Rust allocates the certified scene-image candidates; Go only verified
    /// the bytes. The plan is additive so existing fixtures keep their
    /// pre-projected annotations.
    #[serde(default)]
    pub image_plan: Option<ImagePlanV2>,
    /// Certified phrase-impact ranking; Rust raises phrase scores from it.
    #[serde(default)]
    pub heavy_sentences: Vec<HeavySentenceV2>,
    /// Data-only numeric vocabulary used by the raw-surface projector.
    #[serde(default)]
    pub numeric_lexicon: Option<NumericLexiconV2>,
    #[serde(default)]
    pub scenes: Vec<SceneV2>,
    /// Canonical video clips in timeline order. They are planner input for the
    /// clip-start light-leak lane; they never carry an admitted overlay.
    #[serde(default)]
    pub clips: Vec<ClipV2>,
    /// Scene-level recap sources projected by Rust from already-narrated
    /// scenes. The planner owns placement, dedupe, score order and card timing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recap_briefs: Vec<RecapBriefV2>,
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
    pub short_phrase_motion: String,
    /// Certified short-phrase entrance pool keyed by the phrase's word count
    /// ("1".."5"). With no pin the planner rotates the bucket that matches the
    /// item's own length, so a two-word phrase and a five-word one cannot land
    /// on the same entrance by construction. An absent bucket leaves that
    /// length on the presentation default.
    #[serde(default)]
    pub short_phrase_motions_by_word: BTreeMap<String, Vec<String>>,
    /// Certified entrance pool for long phrases (6+ words). The long lane has no
    /// per-length buckets, so the planner rotates this single pool; an empty
    /// pool leaves the lane on the presentation default, which is the single
    /// entrance every long phrase used to share.
    #[serde(default)]
    pub long_phrase_motions: Vec<String>,
    #[serde(default)]
    pub image_motions: Vec<String>,
    #[serde(default)]
    pub image_with_text_motions: Vec<String>,
    /// Certified entrance pool for entity-image cards. The planner rotates it
    /// per admitted card; a run that admits a card without a pool fails closed.
    #[serde(default)]
    pub entity_image_motions: Vec<String>,
    /// Certified entrance pool for the caption layer, rotated independently
    /// from the portrait so neighboring cards never repeat a caption move.
    #[serde(default)]
    pub entity_caption_motions: Vec<String>,
    /// Run-level rotation offset. The Go retry seam advances it; wall-clock
    /// time never does, so replaying a request selects the same motions.
    #[serde(default)]
    pub image_motion_offset: i64,
    #[serde(default)]
    pub entity_style_id: String,
    /// Optional #RRGGBB plate behind the caption of every generated entity
    /// card. Empty preserves the existing caption presentation.
    #[serde(default)]
    pub entity_badge_color: String,
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
    pub disable_number_overlays: bool,
    #[serde(default)]
    pub maps_only: bool,
    /// Optional certified date-motion family (`typewriter`, `didone`,
    /// `classic`). Empty/`auto` keeps the full certified date pool.
    #[serde(default)]
    pub date_motion_family: String,
    /// Emit one deterministic light leak at every real video clip start.
    #[serde(default)]
    pub light_leak_on_clip_start: bool,
    /// Emit one deterministic light leak at every admitted overlay start.
    #[serde(default)]
    pub light_leak_on_overlay_start: bool,
    /// Per-scene editorial bullet pins. A scene carries at most one projected
    /// bullet, so `scene_id` is the bullet identity. The compiler applies the
    /// pin after the documentary presentation pass, which makes this the
    /// single owner of the bullet's font, size and entrance motion.
    #[serde(default)]
    pub editorial_bullet_styles: Vec<EditorialBulletStyleV2>,
}

/// One per-scene editorial bullet presentation pin. Empty fields keep the
/// presentation default; the Go ingress validator owns the accepted font and
/// size ranges before a request reaches this compiler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EditorialBulletStyleV2 {
    #[serde(default)]
    pub scene_id: String,
    #[serde(default)]
    pub font_family: String,
    #[serde(default)]
    pub font_size_px: f64,
    #[serde(default)]
    pub motion_id: String,
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
    pub animation_pool: Vec<String>,
    #[serde(default)]
    pub style_pool: Vec<String>,
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
    pub index: i64,
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
    /// Pre-selection surface. When present Rust projects it into
    /// `annotations`; Go must not ground or time it first.
    #[serde(default)]
    pub raw: Option<RawSceneV2>,
}

/// One real video clip on the canonical timeline. `freeze` clips are held
/// frames and never receive a light leak; the producer already excluded them
/// from the source data, and this flag keeps the contract explicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipV2 {
    pub scene_id: String,
    #[serde(default)]
    pub asset_id: String,
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub start_us: i64,
    #[serde(default)]
    pub duration_us: i64,
    #[serde(default)]
    pub freeze: bool,
}

/// One already-narrated scene title eligible for a recap card. This is a
/// source candidate, not a selected card: Rust owns placement and admission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecapBriefV2 {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub score: f64,
    pub start_us: i64,
    pub end_us: i64,
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

/// Pre-selection semantic surface for one scene. This is the WP4 boundary:
/// Rust owns phrase anchoring, entity admission, numeric/date projection,
/// map placement and chart narrowing. The producer only supplies grounded
/// lexicon data and media facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RawSceneV2 {
    #[serde(default)]
    pub important_phrases: Vec<RawPhraseV2>,
    #[serde(default)]
    pub primary_entities: Vec<RawEntityV2>,
    #[serde(default)]
    pub secondary_entities: Vec<RawEntityV2>,
    #[serde(default)]
    pub chart_values: Vec<String>,
    #[serde(default)]
    pub charts_certified: bool,
    #[serde(default)]
    pub source_numeric_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawPhraseV2 {
    pub text: String,
    #[serde(default)]
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawEntityV2 {
    #[serde(default)]
    pub stable_entity_id: String,
    #[serde(default)]
    pub canonical_entity_id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub canonical_name: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub geo: Option<RawGeoV2>,
    #[serde(default)]
    pub image: Option<RawEntityImageV2>,
    #[serde(default)]
    pub mentions: Vec<RawMentionV2>,
    #[serde(default)]
    pub plate_covered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawGeoV2 {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawEntityImageV2 {
    #[serde(default)]
    pub asset_id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMentionV2 {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub start_rune: i64,
    #[serde(default)]
    pub end_rune: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeavySentenceV2 {
    pub text: String,
    #[serde(default)]
    pub importance: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NumericLexiconV2 {
    #[serde(default)]
    pub words: Vec<String>,
    #[serde(default)]
    pub context: Vec<String>,
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

/// The producer-computed eligibility and run-level ceiling for generic scene
/// images. Rust performs the actual round-robin allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePlanV2 {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub ceiling: i64,
    #[serde(default)]
    pub excluded_sha256: Vec<String>,
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
