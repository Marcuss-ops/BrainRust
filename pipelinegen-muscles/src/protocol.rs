use serde::{Deserialize, Serialize};

use crate::config;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub version: String,
    pub operation: String,
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
    pub transition_every: Option<i32>,
    pub clip_duration_sec: Option<i32>,
    pub no_effects: Option<bool>,
    pub effects_dir: Option<String>,
    pub effect_every: Option<i32>,
    pub effect_index_hint: Option<i32>,
    pub overlay_opacity: Option<f64>,
    pub keyframe_interval: Option<u32>,
    pub font: Option<String>,
    pub effects: Option<Vec<RenderEffect>>,
    pub overlays: Option<Vec<RenderOverlay>>,
    pub max_duration_sec: Option<f64>,
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

#[derive(Debug, Deserialize)]
pub struct CutJob {
    pub job_id: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub output_path: String,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub ok: bool,
    pub operation: String,
    pub source_path: Option<String>,
    pub items: Vec<CutItem>,
    pub metadata: Option<MediaMetadata>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct CutItem {
    pub job_id: String,
    pub output_path: Option<String>,
    pub status: String,
    pub size_bytes: u64,
    pub duration_sec: f64,
    pub error: Option<String>,
}
