use super::CopyCertification;
use super::{Operation, Response, PROTOCOL_VERSION};
use crate::error::ErrorCode;
use crate::protocol::MediaMetadata;

#[test]
fn implemented_operations_are_declared_in_the_contract_schema() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contract/mediaexec.v2/request.schema.json"
    ))
    .expect("request schema");
    assert_eq!(
        schema["properties"]["version"]["const"].as_str(),
        Some(PROTOCOL_VERSION)
    );
    let declared: Vec<&str> = schema["properties"]["operation"]["enum"]
        .as_array()
        .expect("operation enum")
        .iter()
        .map(|value| value.as_str().expect("operation string"))
        .collect();
    for operation in Operation::ALL {
        assert!(
            declared.contains(&operation.as_str()),
            "{} is implemented but absent from request.schema.json",
            operation.as_str()
        );
    }
}

#[test]
fn response_envelope_contains_every_schema_required_field() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contract/mediaexec.v2/response.schema.json"
    ))
    .expect("response schema");
    let response = serde_json::to_value(Response {
        ok: true,
        operation: "health".to_string(),
        source_path: None,
        items: Vec::new(),
        metadata: None,
        preflight: None,
        error: None,
    })
    .expect("serialize response");
    for field in schema["required"].as_array().expect("required fields") {
        let field = field.as_str().expect("required field name");
        assert!(
            response.get(field).is_some(),
            "Response is missing schema-required field {field}"
        );
    }
}

const TEST_EXTRADATA: &str = "7201ce80a2b2b32a0c1a89501851099f267c1767d59ff799db744f72032aae5a";

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
        video_extradata_sha256: Some(TEST_EXTRADATA.to_string()),
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
        video_extradata_sha256: Some(TEST_EXTRADATA.to_string()),
        mix_ms: None,
        aac_encode_ms: None,
        probe_ms: None,
        hash_ms: None,
        ffmpeg_ms: None,
        final_audio_sha256: None,
        audio_copy_eligible: None,
        audio_encode_passes: None,
        decode_ms: None,
        encode_ms: None,
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
        CopyCertification {
            video_extradata_sha256: None,
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

    let mut wrong_extradata = metadata();
    wrong_extradata.video_extradata_sha256 = Some("00".repeat(32));
    assert!(cert
        .verify_metadata(&wrong_extradata)
        .unwrap_err()
        .to_string()
        .contains("extradata"));

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

use super::{
    OverlayManifest, OverlayManifestEntry, OverlayManifestPlacement, OVERLAY_MANIFEST_SCHEMA_V1,
};

fn manifest_entry() -> OverlayManifestEntry {
    OverlayManifestEntry {
        item_id: "item-01".to_string(),
        track: 0,
        in_us: 10_000_000,
        out_us: 12_000_000,
        content_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_string(),
        placement: None,
        artifact_in_us: None,
        artifact_out_us: None,
    }
}

fn placement() -> OverlayManifestPlacement {
    OverlayManifestPlacement {
        x_px: 40,
        y_px: 800,
        w_px: 800,
        h_px: 150,
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

#[test]
fn multi_artifact_set_requires_certification_and_unique_ownership() {
    use super::{OverlayManifestAsset, OverlayManifestSet, OverlayManifestWire};
    let hash = "a".repeat(64);
    let mut first = manifest(vec![manifest_entry()]);
    first.timeline_sha256 = Some(hash.clone());
    first.asset = Some(OverlayManifestAsset {
        asset_id: "one".into(),
        drive_file_id: String::new(),
        url: String::new(),
        sha256: hash,
        size_bytes: 1,
        frame_count: 48,
        duration_ms: 2000,
    });
    let mut second = first.clone();
    second.entries[0].item_id = "item-02".into();
    second.entries[0].content_hash = "b".repeat(64);
    let mut wire = OverlayManifestWire::Set(OverlayManifestSet {
        schema_version: "pipelinegen.overlay-manifest-set.v1".into(),
        manifests: vec![first, second],
    });
    assert!(wire.validate().is_ok());
    if let OverlayManifestWire::Set(set) = &mut wire {
        set.manifests[1].entries[0].item_id = "item-01".into();
    }
    assert!(wire.validate().is_err());
    if let OverlayManifestWire::Set(set) = &mut wire {
        set.manifests[1].entries[0].item_id = "item-02".into();
        set.manifests[1].asset = None;
    }
    assert!(wire.validate().is_err());
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

// Wire-parity gate with the Go producer (PipelineGen
// ManifestPlacement.Validate): a rect-projected entry carries a positive
// size and a non-negative origin; an absent rect means full-canvas and is
// always valid. The rect is placement metadata and must not gate the
// entry's plan window validation.
#[test]
fn manifest_spatial_placement_validates() {
    let mut entry = manifest_entry();
    entry.placement = Some(placement());
    assert!(manifest(vec![entry]).validate().is_ok());

    let mut full_canvas = manifest_entry();
    full_canvas.placement = None;
    assert!(manifest(vec![full_canvas]).validate().is_ok());
}

#[test]
fn manifest_spatial_placement_size_must_be_positive() {
    let mut zero_width = manifest_entry();
    zero_width.placement = Some(OverlayManifestPlacement {
        w_px: 0,
        ..placement()
    });
    let err = manifest(vec![zero_width]).validate().unwrap_err();
    assert!(
        err.contains("w_px/h_px must be > 0"),
        "unexpected error: {err}"
    );

    let mut negative_height = manifest_entry();
    negative_height.placement = Some(OverlayManifestPlacement {
        h_px: -1,
        ..placement()
    });
    let err = manifest(vec![negative_height]).validate().unwrap_err();
    assert!(
        err.contains("w_px/h_px must be > 0"),
        "unexpected error: {err}"
    );
}

#[test]
fn manifest_spatial_placement_origin_must_be_non_negative() {
    let mut negative_x = manifest_entry();
    negative_x.placement = Some(OverlayManifestPlacement {
        x_px: -1,
        ..placement()
    });
    let err = manifest(vec![negative_x]).validate().unwrap_err();
    assert!(
        err.contains("x_px/y_px must be >= 0"),
        "unexpected error: {err}"
    );

    let mut negative_y = manifest_entry();
    negative_y.placement = Some(OverlayManifestPlacement {
        y_px: -1,
        ..placement()
    });
    let err = manifest(vec![negative_y]).validate().unwrap_err();
    assert!(
        err.contains("x_px/y_px must be >= 0"),
        "unexpected error: {err}"
    );
}

// A half-set rect cannot be expressed on the wire: the object is
// all-or-nothing, so a partial placement fails deserialization instead of
// validating with zero-filled fields (fail-closed at parse time).
#[test]
fn manifest_spatial_placement_is_all_or_nothing_on_the_wire() {
    let wire = serde_json::json!({
        "schema_version": OVERLAY_MANIFEST_SCHEMA_V1,
        "plan_id": "plan-01",
        "video_id": "video-01",
        "entries": [{
            "item_id": "item-01",
            "track": 0,
            "in_us": 10_000_000,
            "out_us": 12_000_000,
            "content_hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "placement": { "x_px": 40, "y_px": 800 }
        }]
    });
    let parsed: Result<OverlayManifest, _> = serde_json::from_value(wire);
    assert!(
        parsed.is_err(),
        "a partial placement must fail parsing, not default to zero fields"
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
