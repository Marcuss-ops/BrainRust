use serde::{Deserialize, Serialize};

/// One `quality_gate` request. Unknown envelope fields (including
/// `operation`) are ignored by serde, so the worker can pass the full line.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GateRequest {
    #[serde(default)]
    pub item: ItemInput,
    #[serde(default)]
    pub plan: PlanInput,
    #[serde(default)]
    pub result: Option<ResultInput>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ItemInput {
    #[serde(default)]
    pub id: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PlanInput {
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub source_text: String,
    #[serde(default)]
    pub target_words: i32,
    #[serde(default)]
    pub translate_to: String,
    #[serde(default)]
    pub grounding_policy: String,
    #[serde(default)]
    pub source_kind: String,
    #[serde(default)]
    pub num_clips: usize,
    #[serde(default)]
    pub segments: Vec<SegmentInput>,
    #[serde(default)]
    pub clip_evidence: Option<ClipEvidenceInput>,
    #[serde(default)]
    pub research_evidence: Option<ResearchEvidenceInput>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SegmentInput {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub source_text: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ClipEvidenceInput {
    #[serde(default)]
    pub accepted_clip_ids: Vec<String>,
    #[serde(default)]
    pub narrative_text: String,
    #[serde(default)]
    pub segment_evidence: Vec<serde_json::Value>,
}

impl ClipEvidenceInput {
    /// Narration-safe projection used by `buildSourceText`: explicit segment
    /// evidence owns the per-segment text, so the global narrative is omitted.
    pub fn model_source_text(&self) -> String {
        if !self.segment_evidence.is_empty() {
            return String::new();
        }
        self.narrative_text.trim().to_string()
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ResearchEvidenceInput {
    #[serde(default)]
    pub candidates: Vec<ResearchCandidateInput>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ResearchCandidateInput {
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ResultInput {
    #[serde(default)]
    pub output: OutputInput,
    #[serde(default)]
    pub artifacts: ArtifactsInput,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct OutputInput {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub word_count: i32,
    #[serde(default)]
    pub specscene: SpecSceneInput,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SpecSceneInput {
    #[serde(default)]
    pub scenes: Vec<SceneInput>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SceneInput {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub segment_id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub bindings: SceneBindingsInput,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SceneBindingsInput {
    #[serde(default)]
    pub clip: Option<ClipBindingInput>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ClipBindingInput {
    #[serde(default)]
    pub clip_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ArtifactsInput {
    #[serde(default)]
    pub entities: Option<EntitiesInput>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EntitiesInput {
    #[serde(default)]
    pub persons: Vec<EntityInput>,
    #[serde(default)]
    pub places: Vec<EntityInput>,
    #[serde(default)]
    pub concepts: Vec<EntityInput>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EntityInput {
    #[serde(default)]
    pub value: String,
}

/// Canonical `GenerationQuality` projection. Field names match the Go JSON
/// contract, so a Go adapter can unmarshal the worker response directly.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Quality {
    #[serde(default)]
    pub language_requested: String,
    #[serde(default)]
    pub language_detected: String,
    #[serde(default)]
    pub source_text_coverage: f64,
    #[serde(default)]
    pub source_text_coverage_status: String,
    #[serde(default)]
    pub clip_evidence_coverage: f64,
    #[serde(default)]
    pub unsupported_claims: i32,
    #[serde(default)]
    pub target_words: i32,
    #[serde(default)]
    pub actual_words: i32,
    #[serde(default)]
    pub passed: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Evaluation {
    pub quality: Quality,
    pub reasons: Vec<String>,
}
