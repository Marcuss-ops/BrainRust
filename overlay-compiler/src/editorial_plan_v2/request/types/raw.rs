use super::*;

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
    /// Grounded chart layout hint aligned to `ChartsPack.hpp`
    /// (`timeline_growth`, `segmented_total_bar`, `bounded_range_comparison`,
    /// `bar_comparison`, `ranking_bar`, `before_after_metric`, ...). Empty
    /// when absent so frozen requests stay byte-identical.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chart_layout: String,
    /// Grounded non-chart visual recipe (`relationship_graph`, ...). Empty when
    /// absent so frozen requests stay byte-identical.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub visual_recipe: String,
    /// Opaque producer-owned payload for `visual_recipe`; renderer metadata is
    /// copied verbatim, exactly like `animation_directives`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_payload: Option<Value>,
    /// Every grounded recipe the scene carries, in producer priority order.
    /// Additive: `visual_recipe` stays the first element for frozen requests,
    /// and an absent list serializes exactly like before. Duplicate recipes are
    /// allowed (two quotes in one scene) because the payload identifies the
    /// fact; the planner applies each once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual_recipes: Vec<VisualRecipeSpecV2>,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chart_layout: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub visual_recipe: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_payload: Option<Value>,
    /// All grounded recipes of the scene, ordered by producer priority. The
    /// legacy pair above stays the first entry so older producers keep working.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual_recipes: Vec<VisualRecipeSpecV2>,
    #[serde(default)]
    pub charts_certified: bool,
    #[serde(default)]
    pub source_numeric_values: Vec<String>,
}

/// One grounded visual recipe plus its opaque payload. The recipe name is
/// validated against the closed set; the payload is renderer metadata and is
/// never interpreted by the planner beyond the fact keys it documents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualRecipeSpecV2 {
    pub recipe: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
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
