use serde::{Deserialize, Serialize};

use crate::error::MuscleError;

use crate::config;
use crate::render_audio::CompiledAudioPlan;
use crate::render_clip::SealedClipPlan;
use crate::render_stock::SealedCanonicalPlan;

pub const PROTOCOL_VERSION: &str = "mediaexec.v1";

/// Assembly-ready output contract identity, V1 (frozen).
///
/// This is the ONE declaration of the string in this crate. It is the same
/// identity the Go assembly contract carries
/// (`internal/kernel/media.AssemblyMediaContractID`) and that the RenderingGen
/// media profile registry exposes; the certification and copy-assembly paths
/// must reference this constant instead of restating the literal, so a rename
/// cannot land in one language and not the others.
pub const VELOX_ASSEMBLY_READY_V1: &str = "VELOX_ASSEMBLY_READY_V1";

/// Assembly-ready output contract identity, V2: the canonical contract for
/// newly produced assembly-ready clips, and the id that gates the stricter
/// extradata/level checks.
pub const VELOX_ASSEMBLY_READY_V2: &str = "VELOX_ASSEMBLY_READY_V2";

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
    RenderClip,
    AdminRender,
    MergeInputs,
    RemoveSilence,
    RenderAudioPlan,
    MuxAudioCopy,
    AssembleCopy,
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
            Self::RenderClip => "render_clip",
            Self::AdminRender => "admin_render",
            Self::MergeInputs => "merge_inputs",
            Self::RemoveSilence => "remove_silence",
            Self::RenderAudioPlan => "render_audio_plan",
            Self::MuxAudioCopy => "mux_audio_copy",
            Self::AssembleCopy => "assemble_copy",
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
    // Watermark scaling + green-screen chroma key (YouTube watermark flow).
    // scale_percent is the overlay size as a percentage of the MAIN frame
    // width (0 = leave the overlay at its native size). green_screen_*
    // drive the ffmpeg chromakey filter that removes the backdrop before
    // the alpha/opacity pass.
    pub scale_percent: Option<u32>,
    pub green_screen_color: Option<String>,
    pub green_screen_similarity: Option<f64>,
    pub green_screen_blend: Option<f64>,
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
    pub font: Option<String>,
    pub effects: Option<Vec<RenderEffect>>,
    pub overlays: Option<Vec<RenderOverlay>>,
    pub overlay_manifest: Option<OverlayManifest>,
    pub max_duration_sec: Option<f64>,
    /// Typed compiled audio plan (compiled-audio-plan.v1/v2). A malformed
    /// plan fails request parsing fail-closed instead of travelling as
    /// untyped JSON.
    pub(crate) audio_plan: Option<CompiledAudioPlan>,
    pub audio_assets: Option<Vec<AudioAsset>>,
    pub copy_certification: Option<CopyCertification>,
    // The Go boundary sends a sealed render plan. Rust treats it as an
    // audited contract and never derives timing or asset selection from it.
    /// Typed sealed canonical render plan. A malformed plan fails request
    /// parsing fail-closed instead of travelling as untyped JSON.
    pub(crate) render_plan: Option<SealedCanonicalPlan>,
    // The Go boundary sends the sealed ClipRenderPlanV1 for render_clip.
    // Same audited-contract rule: Rust validates and executes it verbatim,
    // making zero business selections (background/watermark/subtitles/audio
    // policy/geometry all arrive resolved).
    /// Typed sealed clip render plan. A malformed plan fails request
    /// parsing fail-closed instead of travelling as untyped JSON.
    pub(crate) clip_plan: Option<SealedClipPlan>,
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
            Operation::RenderClip => {
                require_source()?;
                require_output()?;
                if self.clip_plan.is_none() {
                    return Err(MuscleError::generic("clip_plan is required"));
                }
                Ok(())
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

pub const OVERLAY_MANIFEST_SCHEMA_V1: &str = "pipelinegen.overlay-manifest.v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverlayManifestAsset {
    pub asset_id: String,
    pub drive_file_id: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub frame_count: i64,
    pub duration_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OverlayManifestEntry {
    pub item_id: String,
    pub track: u32,
    pub in_us: i64,
    pub out_us: i64,
    pub content_hash: String,
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
        }
        Ok(())
    }
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
    // render_clip phase timings (the benchmark decomposition): startup_ms
    // spans plan decode + source probe + filter-graph build + process spawn
    // (the "renderer startup" BEFORE the ffmpeg wall); publish_ms is the
    // output publish/rename; op_ms is the whole render_clip wall inside the
    // Rust process. None elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_ms: Option<i64>,
    // SHA256 of the published final audio, computed by Rust so the Go adapter
    // never re-hashes the output it just rendered (single ownership of the
    // hash operation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_audio_sha256: Option<String>,
    // render_clip audio copy policy outcome: whether the source audio stream
    // was copied verbatim (true) or one certified conversion ran (false).
    // None everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_copy_eligible: Option<bool>,
    // render_clip audio encode passes (0 = copied, 1 = exactly one certified
    // conversion) and render_audio_plan master AAC encodes (always 1: the
    // stem fan-out mixes to lossless PCM and only the final pass encodes).
    // None everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_encode_passes: Option<i32>,
    // render_clip subtitle raster stage: true when mode=burn rasterized
    // libass inside the single render pass (a CPU stage — never claimed as
    // GPU). None when subtitles are disabled or sidecar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_raster_cpu: Option<bool>,

    // Fine-grained render phases measured by the owning Rust boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_graph_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_raster_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark_raster_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_conversion_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encode_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_mux_ms: Option<i64>,
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

/// CopyCertification is the copy-only certification of an overlay artifact:
/// it declares that the artifact is a self-contained, correctly-encoded video
/// segment (closed GOP, first frame keyframe) that can be assembled by
/// packet-copy with zero decode/encode.
///
/// PRODUCTION STATUS (2026-09-23): this certification and the gate below are
/// the VeloxEditing-side half of a SHARED contract and the LIVE assembly path
/// for the durable video.create workflow (see transform_assemble.rs).
/// RenderingGen's `ParentFinalizer` -> daemon `ASSEMBLE_SEGMENTS`, whose
/// `queue.ValidateChildren` enforces the same three facts (copy_eligible,
/// closed_gop, first_frame_keyframe) before assembling, keeps assembling the
/// chunks of ONE render job — so the two paths cannot disagree about what is
/// assemblable.
///
/// Assembly-ready gate fields (added Wave 5, Aug 2026):
///   - contract_id: MUST be "VELOX_ASSEMBLY_READY_V1"
///   - stream_signature_sha256: all inputs in a batch MUST share identical signature
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CopyCertification {
    pub copy_eligible: bool,
    pub profile_id: Option<String>,
    pub codec: Option<String>,
    pub codec_profile: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps_num: Option<u32>,
    pub fps_den: Option<u32>,
    pub closed_gop: Option<bool>,
    pub first_frame_keyframe: Option<bool>,
    /// Assembly-ready contract identity ("VELOX_ASSEMBLY_READY_V1").
    /// The assembly gate rejects a missing, empty, or unknown contract, so
    /// every cert that reaches execution carries the canonical identity.
    #[serde(default)]
    pub contract_id: Option<String>,
    /// Stream signature SHA-256 fingerprint (canonical JSON of all video+audio+layout facts).
    /// Empty on legacy certifications; gate enforces it when present on all inputs.
    #[serde(default)]
    pub stream_signature_sha256: Option<String>,
    #[serde(default)]
    pub video_extradata_sha256: Option<String>,
    #[serde(default)]
    pub audio_extradata_sha256: Option<String>,
}

impl CopyCertification {
    /// Validates the copy-only certification. It fails closed: a missing or
    /// false field rejects the packet-copy path so the encoded composite path
    /// remains responsible for anything uncertified.
    pub fn validate(&self) -> Result<(), MuscleError> {
        if !self.copy_eligible {
            return Err(MuscleError::copy_not_eligible("copy_eligible is false"));
        }
        if self.profile_id.as_deref().unwrap_or("").trim().is_empty() {
            return Err(MuscleError::certification_required(
                "profile_id is required",
            ));
        }
        if self.codec.as_deref().unwrap_or("").trim().is_empty() {
            return Err(MuscleError::certification_required("codec is required"));
        }
        if self.width.unwrap_or(0) == 0 || self.height.unwrap_or(0) == 0 {
            return Err(MuscleError::certification_required(
                "width/height are required",
            ));
        }
        if self.fps_num.unwrap_or(0) == 0 || self.fps_den.unwrap_or(0) == 0 {
            return Err(MuscleError::certification_required(
                "fps_num/fps_den are required",
            ));
        }
        if self.contract_id.as_deref() == Some(VELOX_ASSEMBLY_READY_V2) {
            if self.width != Some(1920) || self.height != Some(1080) {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 requires 1920x1080",
                ));
            }
            if self.fps_num != Some(24) || self.fps_den != Some(1) {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 requires fps=24/1",
                ));
            }
            if self.codec.as_deref() != Some("h264")
                || self.codec_profile.as_deref() != Some("high")
            {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 requires h264 High",
                ));
            }
            if self
                .video_extradata_sha256
                .as_deref()
                .unwrap_or("")
                .is_empty()
                || self
                    .audio_extradata_sha256
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
            {
                return Err(MuscleError::certification_required(
                    "V2 requires audio/video extradata hashes",
                ));
            }
        }
        if !self.closed_gop.unwrap_or(false) {
            return Err(MuscleError::certification_required(
                "closed_gop must be true",
            ));
        }
        if !self.first_frame_keyframe.unwrap_or(false) {
            return Err(MuscleError::certification_required(
                "first_frame_keyframe must be true",
            ));
        }
        Ok(())
    }

    /// Verifies that probed media matches the certified copy-only profile.
    pub fn verify_metadata(&self, metadata: &MediaMetadata) -> Result<(), MuscleError> {
        if !metadata.has_video {
            return Err(MuscleError::copy_certification_violation(
                "input has no video stream",
            ));
        }
        let width = self.width.unwrap_or(0);
        let height = self.height.unwrap_or(0);
        if metadata.width != width || metadata.height != height {
            return Err(MuscleError::copy_certification_violation(format!(
                "geometry {}x{} != certified {}x{}",
                metadata.width, metadata.height, width, height
            )));
        }
        let expected_num = self.fps_num.unwrap_or(0);
        let expected_den = self.fps_den.unwrap_or(0);
        if expected_num == 0
            || expected_den == 0
            || metadata.fps_num == 0
            || metadata.fps_den == 0
            || metadata.fps_num * expected_den != expected_num * metadata.fps_den
        {
            return Err(MuscleError::copy_certification_violation(format!(
                "fps {}/{} != certified {}/{}",
                metadata.fps_num, metadata.fps_den, expected_num, expected_den
            )));
        }
        if self.contract_id.as_deref() == Some(VELOX_ASSEMBLY_READY_V2) {
            let checks = [
                (
                    metadata.video_profile.as_deref(),
                    Some("high"),
                    "video profile",
                ),
                (metadata.video_level.as_deref(), Some("4.1"), "video level"),
                (
                    metadata.pixel_format.as_deref(),
                    Some("yuv420p"),
                    "pixel format",
                ),
            ];
            for (actual, expected, label) in checks {
                if actual != expected {
                    return Err(MuscleError::output_contract_mismatch(label));
                }
            }
            if metadata.video_time_base_num != Some(1)
                || metadata.video_time_base_den != Some(90000)
                || metadata.sar_num != Some(1)
                || metadata.sar_den != Some(1)
                || metadata.field_order.as_deref() != Some("progressive")
                || metadata.keyframe_interval != Some(48)
                || metadata.b_frames != Some(0)
                || metadata.closed_gop != Some(true)
            {
                return Err(MuscleError::output_contract_mismatch(
                    "V2 video timing/GOP properties",
                ));
            }
            if metadata.audio_codec.as_deref() != Some("aac")
                || metadata.audio_profile.as_deref() != Some("LC")
                || metadata.audio_time_base_num != Some(1)
                || metadata.audio_time_base_den != Some(48000)
                || metadata.sample_rate != Some(48000)
                || metadata.channels != Some(2)
                || metadata.channel_layout.as_deref() != Some("stereo")
                || metadata.audio_bitrate != Some(192000)
            {
                return Err(MuscleError::output_contract_mismatch("V2 audio properties"));
            }
            if metadata.video_extradata_sha256.as_deref() != self.video_extradata_sha256.as_deref()
                || metadata.audio_extradata_sha256.as_deref()
                    != self.audio_extradata_sha256.as_deref()
            {
                return Err(MuscleError::output_contract_mismatch("extradata hash"));
            }
        }
        if !copy_codec_matches(
            metadata.video_codec.as_deref(),
            self.codec.as_deref().unwrap_or(""),
        ) {
            return Err(MuscleError::copy_certification_violation(format!(
                "codec {:?} != certified {}",
                metadata.video_codec,
                self.codec.as_deref().unwrap_or("")
            )));
        }
        Ok(())
    }
}

fn copy_codec_matches(actual: Option<&str>, expected: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    match actual {
        Some(codec) if expected == "h264" || expected == "libx264" || expected == "h264_nvenc" => {
            codec == "h264"
        }
        Some(codec) => codec == expected,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::CopyCertification;
    use crate::error::ErrorCode;
    use crate::protocol::MediaMetadata;

    fn certified() -> CopyCertification {
        CopyCertification {
            copy_eligible: true,
            profile_id: Some("velox-h264-copy-v1".to_string()),
            codec: Some("h264".to_string()),
            codec_profile: Some("high".to_string()),
            width: Some(1920),
            height: Some(1080),
            fps_num: Some(24),
            fps_den: Some(1),
            closed_gop: Some(true),
            first_frame_keyframe: Some(true),
            contract_id: None,
            stream_signature_sha256: None,
            video_extradata_sha256: None,
            audio_extradata_sha256: None,
        }
    }

    fn metadata() -> MediaMetadata {
        MediaMetadata {
            duration_sec: 18.2,
            bitrate: None,
            width: 1920,
            height: 1080,
            fps: 24.0,
            video_codec: Some("h264".to_string()),
            pixel_format: Some("yuv420p".to_string()),
            format_name: Some("mov,mp4,m4a,3gp,3g2,mj2".to_string()),
            stream_count: 1,
            video_stream_count: 1,
            audio_stream_count: 0,
            fps_num: 24,
            fps_den: 1,
            audio_codec: None,
            audio_profile: None,
            sample_rate: None,
            channels: None,
            start_pts: None,
            has_video: true,
            has_audio: false,
            mix_ms: None,
            aac_encode_ms: None,
            probe_ms: None,
            hash_ms: None,
            ffmpeg_ms: None,
            startup_ms: None,
            publish_ms: None,
            op_ms: None,
            final_audio_sha256: None,
            audio_copy_eligible: None,
            audio_encode_passes: None,
            subtitle_raster_cpu: None,
            decode_ms: None,
            filter_graph_ms: None,
            subtitle_raster_ms: None,
            watermark_raster_ms: None,
            frame_conversion_ms: None,
            encode_ms: None,
            audio_mux_ms: None,
            ..Default::default()
        }
    }

    #[test]
    fn certified_metadata_matches_copy_only_profile() {
        assert!(certified().validate().is_ok());
        assert!(certified().verify_metadata(&metadata()).is_ok());
    }

    #[test]
    fn copy_not_eligible_fails_closed() {
        let mut cert = certified();
        cert.copy_eligible = false;
        assert_eq!(
            cert.validate().unwrap_err().code(),
            ErrorCode::CopyNotEligible
        );
    }

    #[test]
    fn missing_certification_fields_fail_closed() {
        for cert in [
            CopyCertification {
                profile_id: None,
                ..certified()
            },
            CopyCertification {
                codec: None,
                ..certified()
            },
            CopyCertification {
                width: None,
                ..certified()
            },
            CopyCertification {
                fps_num: None,
                ..certified()
            },
            CopyCertification {
                closed_gop: None,
                ..certified()
            },
            CopyCertification {
                first_frame_keyframe: None,
                ..certified()
            },
        ] {
            assert!(cert.validate().is_err(), "expected certification to fail");
        }
    }

    #[test]
    fn metadata_violations_fail_closed() {
        let cert = certified();

        let mut wrong_geometry = metadata();
        wrong_geometry.width = 1280;
        assert!(cert
            .verify_metadata(&wrong_geometry)
            .unwrap_err()
            .to_string()
            .contains("geometry"));

        let mut wrong_fps = metadata();
        wrong_fps.fps_num = 30;
        assert!(cert
            .verify_metadata(&wrong_fps)
            .unwrap_err()
            .to_string()
            .contains("fps"));

        let mut wrong_codec = metadata();
        wrong_codec.video_codec = Some("hevc".to_string());
        assert!(cert
            .verify_metadata(&wrong_codec)
            .unwrap_err()
            .to_string()
            .contains("codec"));

        let mut no_video = metadata();
        no_video.has_video = false;
        assert!(cert
            .verify_metadata(&no_video)
            .unwrap_err()
            .to_string()
            .contains("no video"));
    }

    use super::{OverlayManifest, OverlayManifestEntry, OVERLAY_MANIFEST_SCHEMA_V1};

    fn manifest_entry() -> OverlayManifestEntry {
        OverlayManifestEntry {
            item_id: "item-01".to_string(),
            track: 0,
            in_us: 10_000_000,
            out_us: 12_000_000,
            content_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            artifact_in_us: None,
            artifact_out_us: None,
        }
    }

    fn manifest(entries: Vec<OverlayManifestEntry>) -> OverlayManifest {
        OverlayManifest {
            schema_version: OVERLAY_MANIFEST_SCHEMA_V1.to_string(),
            plan_id: "plan-01".to_string(),
            video_id: "video-01".to_string(),
            timeline_sha256: None,
            asset: None,
            entries,
        }
    }

    // Wire-parity gate with the Go producer (PipelineGen
    // OverlayManifestEntry.Validate): a packed manifest carries
    // artifact_in_us/artifact_out_us together, both windows are positive, and
    // the packed span never outlasts the entry's literal plan window. The
    // plan window itself (in_us/out_us) stays verbatim and is validated
    // independently.
    #[test]
    fn manifest_packed_placements_validate() {
        let mut entry = manifest_entry();
        entry.artifact_in_us = Some(0);
        entry.artifact_out_us = Some(2_000_000);
        assert!(manifest(vec![entry]).validate().is_ok());
    }

    #[test]
    fn manifest_packed_placements_must_be_set_together() {
        let mut half_set = manifest_entry();
        half_set.artifact_in_us = Some(0);
        let err = manifest(vec![half_set]).validate().unwrap_err();
        assert!(err.contains("set together"), "unexpected error: {err}");

        let mut other_half = manifest_entry();
        other_half.artifact_out_us = Some(2_000_000);
        let err = manifest(vec![other_half]).validate().unwrap_err();
        assert!(err.contains("set together"), "unexpected error: {err}");
    }

    #[test]
    fn manifest_packed_span_never_outlasts_the_plan_window() {
        let mut entry = manifest_entry();
        entry.artifact_in_us = Some(0);
        entry.artifact_out_us = Some(3_000_001);
        let err = manifest(vec![entry]).validate().unwrap_err();
        assert!(
            err.contains("longer than its plan window"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn manifest_packed_window_must_be_positive() {
        let mut inverted = manifest_entry();
        inverted.artifact_in_us = Some(2_000_000);
        inverted.artifact_out_us = Some(2_000_000);
        let err = manifest(vec![inverted]).validate().unwrap_err();
        assert!(
            err.contains("artifact_out_us must be > artifact_in_us"),
            "unexpected error: {err}"
        );

        let mut negative = manifest_entry();
        negative.artifact_in_us = Some(-1);
        negative.artifact_out_us = Some(1);
        let err = manifest(vec![negative]).validate().unwrap_err();
        assert!(
            err.contains("artifact_in_us must be >= 0"),
            "unexpected error: {err}"
        );
    }
}
