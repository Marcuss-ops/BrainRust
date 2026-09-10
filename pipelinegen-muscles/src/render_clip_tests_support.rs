use crate::config::VideoProfile;
use crate::protocol::MediaMetadata;
use crate::render_clip::plan::{
    ClipPlanAudio, ClipPlanBackground, ClipPlanOutput, ClipPlanSource, ClipPlanSubtitles,
    ClipPlanWatermark, ClipRenderPlan,
};
pub(super) fn plan() -> ClipRenderPlan {
    ClipRenderPlan {
        version: "clip-render-plan.v1".to_string(),
        run_id: "job-1".to_string(),
        source: ClipPlanSource {
            asset_id: "asset-src".to_string(),
            path: "/tmp/source.mp4".to_string(),
            sha256: "a".repeat(64),
        },
        background: Some(ClipPlanBackground {
            mode: "blur_source".to_string(),
            asset_id: None,
            path: None,
            sha256: None,
        }),
        watermark: None,
        subtitles: None,
        output: ClipPlanOutput {
            contract_id: "VELOX_ASSEMBLY_READY_V1".to_string(),
            container: "mp4".to_string(),
            video_codec: "h264".to_string(),
            video_profile: Some("high".to_string()),
            pixel_format: "yuv420p".to_string(),
            width: 1080,
            height: 1920,
            fps_num: 60,
            fps_den: 1,
            foreground_scale_percent: 100,
        },
        audio: ClipPlanAudio {
            mode: "copy_if_compatible".to_string(),
            codec: "aac".to_string(),
            sample_rate: 48000,
            channels: 2,
        },
        output_path: "/tmp/out.mp4".to_string(),
        plan_sha256: "a".repeat(64),
    }
}

pub(super) fn profile() -> VideoProfile {
    VideoProfile {
        width: 1080,
        height: 1920,
        fps_num: 60,
        fps_den: 1,
        keyframe_interval: 120,
        audio_codec: "aac".to_string(),
        audio_bitrate: "128k".to_string(),
        sample_rate: 48000,
        channels: 2,
    }
}

pub(super) fn source_metadata(has_audio: bool) -> MediaMetadata {
    MediaMetadata {
        duration_sec: 30.0,
        bitrate: None,
        width: 1920,
        height: 1080,
        fps: 24.0,
        video_codec: Some("h264".to_string()),
        pixel_format: Some("yuv420p".to_string()),
        format_name: None,
        stream_count: 1,
        video_stream_count: 1,
        audio_stream_count: u32::from(has_audio),
        fps_num: 24,
        fps_den: 1,
        audio_codec: if has_audio {
            Some("aac".to_string())
        } else {
            None
        },
        audio_profile: None,
        sample_rate: if has_audio { Some(48000) } else { None },
        channels: if has_audio { Some(2) } else { None },
        start_pts: None,
        has_video: true,
        has_audio,
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
