use serde::{Deserialize, Serialize};

use crate::config;

pub const PROTOCOL_VERSION: &str = "mediaexec.v1";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Health,
    Probe,
    CutBatch,
    Normalize,
    CutCopy,
    CutAndNormalize,
    Watermark,
    ExtractFrame,
    GenerateProxy,
    GenerateStoryboard,
    RemuxHls,
    Trim,
    RenderStock,
    AdminRender,
    MergeInputs,
    RemoveSilence,
}

impl Operation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Probe => "probe",
            Self::CutBatch => "cut_batch",
            Self::Normalize => "normalize",
            Self::CutCopy => "cut_copy",
            Self::CutAndNormalize => "cut_and_normalize",
            Self::Watermark => "watermark",
            Self::ExtractFrame => "extract_frame",
            Self::GenerateProxy => "generate_proxy",
            Self::GenerateStoryboard => "generate_storyboard",
            Self::RemuxHls => "remux_hls",
            Self::Trim => "trim",
            Self::RenderStock => "render_stock",
            Self::AdminRender => "admin_render",
            Self::MergeInputs => "merge_inputs",
            Self::RemoveSilence => "remove_silence",
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
    pub overlay_path: Option<String>,
    pub opacity: Option<f64>,
    pub input_paths: Option<Vec<String>>,
    pub no_transitions: Option<bool>,
    pub clip_duration_sec: Option<i32>,
    pub transitions: Option<Vec<RenderTransition>>,
    // Legacy selection inputs are accepted only so the protocol can reject
    // unresolved requests explicitly; Rust never uses them to choose assets.
    pub transition_every: Option<i32>,
    pub effects_dir: Option<String>,
    pub effect_every: Option<i32>,
    pub effect_index_hint: Option<i32>,
    pub no_effects: Option<bool>,
    pub effect_paths: Option<Vec<RenderEffectPath>>,
    pub overlay_opacity: Option<f64>,
    pub font: Option<String>,
    pub effects: Option<Vec<RenderEffect>>,
    pub overlays: Option<Vec<RenderOverlay>>,
    pub max_duration_sec: Option<f64>,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != PROTOCOL_VERSION {
            return Err(format!("unsupported protocol version: {}", self.version));
        }
        let require_source = || {
            if self.source_path.as_deref().unwrap_or("").trim().is_empty() {
                Err("source_path is required".to_string())
            } else {
                Ok(())
            }
        };
        let require_output = || {
            if self.output_path.as_deref().unwrap_or("").trim().is_empty() {
                Err("output_path is required".to_string())
            } else {
                Ok(())
            }
        };
        match self.operation {
            Operation::Health => Ok(()),
            Operation::Probe => require_source(),
            Operation::CutBatch => {
                require_source()?;
                if self.jobs.as_ref().map_or(true, Vec::is_empty) {
                    return Err("jobs are required".to_string());
                }
                Ok(())
            }
            Operation::RenderStock => {
                if self.input_paths.as_ref().map_or(true, Vec::is_empty) {
                    return Err("input_paths are required".to_string());
                }
                require_output()
            }
            Operation::AdminRender
            | Operation::CutCopy
            | Operation::Normalize
            | Operation::CutAndNormalize
            | Operation::Watermark
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
                if self.input_paths.as_ref().map_or(true, Vec::is_empty) {
                    return Err("input_paths are required".to_string());
                }
                require_output()
            }
        }
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
pub struct RenderEffect {
    pub path: String,
    pub delay_ms: i32,
    pub duration: f64,
    pub volume: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RenderOverlay {
    pub text: String,
    pub start: String,
    pub end: String,
    pub size: String,
    pub y: String,
    pub color: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CutJob {
    pub job_id: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub output_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub ok: bool,
    pub operation: String,
    pub source_path: Option<String>,
    pub items: Vec<CutItem>,
    pub metadata: Option<MediaMetadata>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MediaMetadata {
    pub duration_sec: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CutItem {
    pub job_id: String,
    pub output_path: Option<String>,
    pub status: String,
    pub size_bytes: u64,
    pub duration_sec: f64,
    pub error: Option<String>,
}
