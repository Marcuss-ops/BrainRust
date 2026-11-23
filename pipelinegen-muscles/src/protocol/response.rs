use serde::{Deserialize, Serialize};

use crate::preflight::PreflightReport;

use super::request::Operation;

impl Response {
    /// One failure envelope for every capability. The operation is a typed
    /// input, not a string the caller patches afterwards: before this
    /// constructor the shared helper hardcoded `"cut_batch"` and only the
    /// dispatcher rewrote it, so any other entry point reported the wrong
    /// operation.
    pub fn failure(
        operation: Operation,
        source_path: Option<String>,
        error: impl ToString,
    ) -> Self {
        Self {
            ok: false,
            operation: operation.as_str().to_string(),
            source_path,
            items: Vec::new(),
            metadata: None,
            preflight: None,
            error: Some(error.to_string()),
        }
    }

    /// A success envelope with no preflight report. Adding a response field is
    /// now one edit here instead of one per literal site.
    pub fn success(
        operation: Operation,
        source_path: Option<String>,
        items: Vec<CutItem>,
        metadata: Option<MediaMetadata>,
    ) -> Self {
        Self {
            ok: true,
            operation: operation.as_str().to_string(),
            source_path,
            items,
            metadata,
            preflight: None,
            error: None,
        }
    }

    /// Attaches the preflight report; the field stays absent on the wire for
    /// every other operation.
    pub fn with_preflight(mut self, preflight: PreflightReport) -> Self {
        self.preflight = Some(preflight);
        self
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Response {
    pub ok: bool,
    pub operation: String,
    pub source_path: Option<String>,
    pub items: Vec<CutItem>,
    pub metadata: Option<MediaMetadata>,
    /// Structured preflight report. Present only for the `preflight`
    /// operation; every other response omits the field on the wire, so the
    /// existing goldens stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight: Option<PreflightReport>,
    pub error: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct MediaMetadata {
    pub duration_sec: f64,
    pub bitrate: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_level: Option<String>,
    pub pixel_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_time_base_num: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_time_base_den: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sar_num: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sar_den: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_range: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_transfer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_primaries: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_order: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyframe_interval: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_frames: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_gop: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_extradata_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_name: Option<String>,
    #[serde(default)]
    pub stream_count: u32,
    #[serde(default)]
    pub video_stream_count: u32,
    #[serde(default)]
    pub audio_stream_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_edits: Vec<AudioEditMetadata>,
    #[serde(default)]
    pub fps_num: u32,
    #[serde(default)]
    pub fps_den: u32,
    pub audio_codec: Option<String>,
    pub audio_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_time_base_num: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_time_base_den: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_bitrate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_extradata_sha256: Option<String>,
    pub start_pts: Option<i64>,
    pub has_video: bool,
    pub has_audio: bool,
    // Stage timings populated only by the combined-audio render path
    // (render_audio_plan). None everywhere else, and omitted on the wire.
    //
    // prepare_ms and loudness_ms cover the work that happens BEFORE the mix:
    // source probing, plan-to-chain resolution and output preparation
    // (prepare_ms), of which the one-per-path BGM mean-volume measurement is
    // reported separately (loudness_ms) because it decodes the whole music
    // asset. Without them the caller's render wall had a block no stage owned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aac_encode_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_ms: Option<i64>,
    // FFmpeg wall time for the media encode subprocess, measured natively by
    // render_stock (canonical + legacy composite) so callers get the encode
    // wall time without an external timing shim. None elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffmpeg_ms: Option<i64>,
    // SHA256 of the published final audio, computed by Rust so the Go adapter
    // never re-hashes the output it just rendered (single ownership of the
    // hash operation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_audio_sha256: Option<String>,
    // Audio copy policy outcome: whether the source audio stream
    // was copied verbatim (true) or one certified conversion ran (false).
    // None everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_copy_eligible: Option<bool>,
    // Audio encode passes (0 = copied, 1 = exactly one certified
    // conversion) and render_audio_plan master AAC encodes (always 1: the
    // stem fan-out mixes to lossless PCM and only the final pass encodes).
    // None everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_encode_passes: Option<i32>,
    // Fine-grained render phases measured by the owning Rust boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encode_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AudioEditMetadata {
    pub source_start_us: i64,
    pub source_end_us: i64,
    pub output_start_us: i64,
    pub output_end_us: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CutItem {
    pub job_id: String,
    pub output_path: Option<String>,
    pub status: String,
    pub size_bytes: u64,
    pub duration_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_extradata_sha256: Option<String>,
    pub error: Option<String>,
}
