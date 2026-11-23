use serde::Deserialize;

use crate::config;
use crate::error::MuscleError;
use crate::preflight::PreflightRequest;
use crate::render_audio_plan::Plan as CompiledAudioPlan;
use crate::render_stock_canonical_plan::CanonicalRenderPlan as SealedCanonicalPlan;

use super::certification::CopyCertification;
use super::overlay::OverlayManifestWire;
use super::PROTOCOL_VERSION;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Health,
    Probe,
    CutBatch,
    Normalize,
    CutCopy,
    CutAndNormalize,
    ExtractFrame,
    GenerateProxy,
    GenerateStoryboard,
    RemuxHls,
    Trim,
    RenderStock,
    MergeInputs,
    RemoveSilence,
    RenderAudioPlan,
    MuxAudioCopy,
    AssembleCopy,
    ProjectOverlayManifest,
    ExtractTimelineWindow,
    Preflight,
}

impl Operation {
    /// Every wire operation. The contract drift test uses this to prove that
    /// each implemented operation is declared in request.schema.json.
    pub const ALL: [Self; 20] = [
        Self::Health,
        Self::Probe,
        Self::CutBatch,
        Self::Normalize,
        Self::CutCopy,
        Self::CutAndNormalize,
        Self::ExtractFrame,
        Self::GenerateProxy,
        Self::GenerateStoryboard,
        Self::RemuxHls,
        Self::Trim,
        Self::RenderStock,
        Self::MergeInputs,
        Self::RemoveSilence,
        Self::RenderAudioPlan,
        Self::MuxAudioCopy,
        Self::AssembleCopy,
        Self::ProjectOverlayManifest,
        Self::ExtractTimelineWindow,
        Self::Preflight,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Probe => "probe",
            Self::CutBatch => "cut_batch",
            Self::Normalize => "normalize",
            Self::CutCopy => "cut_copy",
            Self::CutAndNormalize => "cut_and_normalize",
            Self::ExtractFrame => "extract_frame",
            Self::GenerateProxy => "generate_proxy",
            Self::GenerateStoryboard => "generate_storyboard",
            Self::RemuxHls => "remux_hls",
            Self::Trim => "trim",
            Self::RenderStock => "render_stock",
            Self::MergeInputs => "merge_inputs",
            Self::RemoveSilence => "remove_silence",
            Self::RenderAudioPlan => "render_audio_plan",
            Self::MuxAudioCopy => "mux_audio_copy",
            Self::AssembleCopy => "assemble_copy",
            Self::ProjectOverlayManifest => "project_overlay_manifest",
            Self::ExtractTimelineWindow => "extract_timeline_window",
            Self::Preflight => "preflight",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Request {
    pub version: String,
    pub operation: Operation,
    pub ffmpeg_path: Option<String>,
    pub source_path: Option<String>,
    pub output_path: Option<String>,
    pub timestamp_sec: Option<f64>,
    pub start_sec: Option<f64>,
    pub end_sec: Option<f64>,
    pub interval_frames: Option<u32>,
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub jobs: Option<Vec<CutJob>>,
    #[serde(flatten)]
    pub media: config::MediaConfig,
    pub no_audio: Option<bool>,
    pub keep_audio: Option<bool>,
    pub input_paths: Option<Vec<String>>,
    pub no_transitions: Option<bool>,
    pub clip_duration_sec: Option<i32>,
    pub transitions: Option<Vec<RenderTransition>>,
    // Legacy unresolved-selection inputs (transition_every / effects_dir /
    // effect_every / effect_index_hint) were removed once the Go transport
    // stopped emitting them: the sealed render_plan is the only selection
    // source, so a request that still carried them is now indistinguishable
    // from any other planless render_stock and fails closed on the required
    // render_plan instead of a bespoke legacy-field error.
    pub no_effects: Option<bool>,
    pub effect_paths: Option<Vec<RenderEffectPath>>,
    pub overlay_opacity: Option<f64>,
    pub overlay_manifest: Option<OverlayManifestWire>,
    /// Manifest window reference for `extract_timeline_window` — the
    /// `scene.clip.{timeline_item_id,in_us,out_us}` wire of the final worker
    /// handoff (OVERLAY-MANIFEST-CONTRACT.md §4): one entry of the manifest,
    /// optionally narrowed to a plan sub-window.
    pub timeline_window: Option<TimelineWindowRef>,
    pub max_duration_sec: Option<f64>,
    /// Typed compiled audio plan (compiled-audio-plan.v1/v2). A malformed
    /// plan fails request parsing fail-closed instead of travelling as
    /// untyped JSON.
    pub(crate) audio_plan: Option<CompiledAudioPlan>,
    pub audio_assets: Option<Vec<AudioAsset>>,
    pub copy_certification: Option<CopyCertification>,
    /// Typed preflight payload for the `preflight` operation. The engine is
    /// read-only and the blocking decision stays with Go, so the request has
    /// no output path and the response carries the structured report.
    pub preflight: Option<PreflightRequest>,
    // The Go boundary sends a sealed render plan. Rust treats it as an
    // audited contract and never derives timing or asset selection from it.
    /// Typed sealed canonical render plan. A malformed plan fails request
    /// parsing fail-closed instead of travelling as untyped JSON.
    pub(crate) render_plan: Option<SealedCanonicalPlan>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AudioAsset {
    pub asset_id: String,
    pub path: String,
}

impl Request {
    pub fn validate(&self) -> Result<(), MuscleError> {
        if self.version != PROTOCOL_VERSION {
            return Err(MuscleError::generic(format!(
                "unsupported protocol version: {}",
                self.version
            )));
        }
        if let Some(manifest) = &self.overlay_manifest {
            manifest.validate().map_err(MuscleError::generic)?;
        }
        let require_source = || {
            if self.source_path.as_deref().unwrap_or("").trim().is_empty() {
                Err(MuscleError::generic("source_path is required"))
            } else {
                Ok(())
            }
        };
        let require_output = || {
            if self.output_path.as_deref().unwrap_or("").trim().is_empty() {
                Err(MuscleError::generic("output_path is required"))
            } else {
                Ok(())
            }
        };
        match self.operation {
            Operation::Health => Ok(()),
            Operation::Probe => require_source(),
            Operation::CutBatch => {
                require_source()?;
                if self.jobs.as_ref().is_none_or(Vec::is_empty) {
                    return Err(MuscleError::generic("jobs are required"));
                }
                Ok(())
            }
            Operation::RenderStock => {
                if self.input_paths.as_ref().is_none_or(Vec::is_empty) {
                    return Err(MuscleError::generic("input_paths are required"));
                }
                require_output()
            }
            Operation::CutCopy
            | Operation::Normalize
            | Operation::CutAndNormalize
            | Operation::ExtractFrame
            | Operation::GenerateProxy
            | Operation::GenerateStoryboard
            | Operation::RemuxHls
            | Operation::Trim => {
                require_source()?;
                require_output()
            }
            Operation::RemoveSilence => {
                require_source()?;
                require_output()
            }
            Operation::MergeInputs => {
                if self.input_paths.as_ref().is_none_or(Vec::is_empty) {
                    return Err(MuscleError::generic("input_paths are required"));
                }
                require_output()
            }
            Operation::RenderAudioPlan => {
                require_output()?;
                if self.audio_plan.is_none() {
                    return Err(MuscleError::generic("audio_plan is required"));
                }
                if self.audio_assets.as_ref().is_none_or(Vec::is_empty) {
                    return Err(MuscleError::generic("audio_assets are required"));
                }
                Ok(())
            }
            Operation::MuxAudioCopy => {
                require_output()?;
                if self
                    .input_paths
                    .as_ref()
                    .is_none_or(|paths| paths.len() != 2)
                {
                    return Err(MuscleError::generic(
                        "exactly video and final audio inputs are required",
                    ));
                }
                Ok(())
            }
            Operation::AssembleCopy => {
                if self.input_paths.as_ref().is_none_or(Vec::is_empty) {
                    return Err(MuscleError::generic("input_paths are required"));
                }
                require_output()?;
                if self.copy_certification.is_none() {
                    return Err(MuscleError::generic("copy_certification is required"));
                }
                Ok(())
            }
            Operation::ProjectOverlayManifest => {
                require_source()?;
                require_output()?;
                let manifest = self
                    .overlay_manifest
                    .as_ref()
                    .ok_or_else(|| MuscleError::generic("overlay_manifest is required"))?;
                if self
                    .input_paths
                    .as_ref()
                    .is_none_or(|paths| paths.len() != manifest.manifests().len())
                {
                    return Err(MuscleError::generic(
                        "one artifact input per manifest is required",
                    ));
                }
                Ok(())
            }
            Operation::ExtractTimelineWindow => {
                require_source()?;
                require_output()?;
                if self.overlay_manifest.is_none() {
                    return Err(MuscleError::generic("overlay_manifest is required"));
                }
                match self.timeline_window.as_ref() {
                    None => Err(MuscleError::generic("timeline_window is required")),
                    Some(window) => window.validate().map_err(MuscleError::generic),
                }
            }
            Operation::Preflight => {
                if self.preflight.is_none() {
                    return Err(MuscleError::generic("preflight is required"));
                }
                Ok(())
            }
        }
    }
}

/// Manifest window reference (`scene.clip.{timeline_item_id,in_us,out_us}`):
/// one manifest entry, optionally narrowed to a plan sub-window. The
/// sub-window maps through the entry's packed offset like any §3 window.
#[derive(Clone, Debug, Deserialize)]
pub struct TimelineWindowRef {
    pub timeline_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_us: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_us: Option<i64>,
}

impl TimelineWindowRef {
    fn validate(&self) -> Result<(), String> {
        if self.timeline_item_id.trim().is_empty() {
            return Err("timeline_window timeline_item_id is required".to_string());
        }
        match (self.in_us, self.out_us) {
            (Some(in_us), Some(out_us)) => {
                if in_us < 0 {
                    return Err("timeline_window in_us must be >= 0".to_string());
                }
                if out_us <= in_us {
                    return Err("timeline_window out_us must be > in_us".to_string());
                }
            }
            (None, None) => {}
            _ => {
                return Err("timeline_window in_us/out_us must be set together".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct RenderTransition {
    pub clip_index: usize,
    pub segment: String,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RenderEffectPath {
    pub clip_index: usize,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CutJob {
    pub job_id: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub output_path: String,
}
