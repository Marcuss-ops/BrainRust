use crate::protocol::{Operation, Request, Response};
use crate::render_audio;
use std::io;

/// Processes one capability request. The protocol exposes media capabilities,
/// never arbitrary command execution.
pub fn process(request: Request) -> Response {
    // Typed operation: every capability call below is dispatched through the
    // `Operation` enum, never a re-parsed string. Only the JSON response
    // envelope renders it back to its stable wire name.
    let operation = request.operation;
    if let Err(error) = request.validate() {
        return Response {
            ok: false,
            operation: operation.as_str().to_string(),
            source_path: request.source_path,
            items: Vec::new(),
            metadata: None,
            error: Some(error.to_string()),
        };
    }
    let mut response = match operation {
        Operation::Health => Response {
            ok: true,
            operation: "health".to_string(),
            source_path: None,
            items: Vec::new(),
            metadata: None,
            error: None,
        },
        Operation::CutBatch => crate::cut::execute(request),
        Operation::Probe => crate::probe::execute(request),
        Operation::Normalize => crate::transform::execute(request, Operation::Normalize),
        Operation::CutCopy => crate::transform::execute(request, Operation::CutCopy),
        Operation::CutAndNormalize => {
            crate::transform::execute(request, Operation::CutAndNormalize)
        }
        Operation::Watermark => crate::transform::execute(request, Operation::Watermark),
        Operation::ExtractFrame => crate::transform::execute(request, Operation::ExtractFrame),
        Operation::GenerateProxy => crate::transform::execute(request, Operation::GenerateProxy),
        Operation::GenerateStoryboard => {
            crate::transform::execute(request, Operation::GenerateStoryboard)
        }
        Operation::RemuxHls => crate::transform::execute(request, Operation::RemuxHls),
        Operation::Trim => crate::transform::execute(request, Operation::Trim),
        Operation::RenderStock => crate::render_stock::execute(request),
        Operation::RenderClip => crate::render_clip::render_clip(request),
        Operation::AdminRender => crate::admin_media::execute(request),
        Operation::MergeInputs => crate::transform::execute(request, Operation::MergeInputs),
        Operation::RemoveSilence => crate::transform::execute(request, Operation::RemoveSilence),
        Operation::RenderAudioPlan => render_audio::execute(request),
        Operation::MuxAudioCopy => crate::transform::execute(request, Operation::MuxAudioCopy),
        Operation::AssembleCopy => crate::transform::execute(request, Operation::AssembleCopy),
    };
    if !response.ok {
        response.operation = operation.as_str().to_string();
    }
    response
}

pub fn run_stdio() -> io::Result<()> {
    // Requests are independent, so the persistent worker serves them from a
    // bounded thread pool. Responses stay in input order and byte-identical
    // to the serial loop; `PIPELINE_MUSCLES_WORKERS=1` restores serial.
    ndjson_stdio::run_stdio_parallel(
        |line| {
            let response = match serde_json::from_str::<Request>(line) {
                Ok(request) => process(request),
                Err(error) => Response {
                    ok: false,
                    operation: "invalid".to_string(),
                    source_path: None,
                    items: Vec::new(),
                    metadata: None,
                    error: Some(format!("invalid request: {error}")),
                },
            };
            match serde_json::to_string(&response) {
                Ok(line) => line,
                Err(error) => serde_json::json!({
                    "ok": false,
                    "operation": "invalid",
                    "error": format!("dispatcher encode: {error}"),
                })
                .to_string(),
            }
        },
        ndjson_stdio::workers_from_env("PIPELINE_MUSCLES_WORKERS"),
    )
}

#[cfg(test)]
mod tests {
    use super::process;
    use crate::protocol::{Operation, RenderEffectPath, RenderTransition, Request};
    use crate::render_stock::validate_resolved_render_plan;
    use crate::COMPONENT;

    #[test]
    fn component_name_is_stable() {
        assert_eq!(COMPONENT, "pipelinegen-muscles");
    }

    #[test]
    fn render_stock_without_a_plan_fails_closed_without_rust_side_selection() {
        // The legacy unresolved-selection fields are gone; a render_stock with
        // no sealed render_plan is now the single fail-closed shape. Rust never
        // chooses assets: it fails with the typed canonical-plan requirement.
        let request = Request {
            version: "mediaexec.v1".to_string(),
            operation: Operation::RenderStock,
            ffmpeg_path: None,
            source_path: None,
            output_path: Some("/tmp/out.mp4".to_string()),
            timestamp_sec: None,
            start_sec: None,
            end_sec: None,
            interval_frames: None,
            columns: None,
            rows: None,
            jobs: None,
            media: crate::config::MediaConfig::default(),
            no_audio: None,
            keep_audio: None,
            overlay_path: None,
            opacity: None,
            scale_percent: None,
            green_screen_color: None,
            green_screen_similarity: None,
            green_screen_blend: None,
            input_paths: Some(vec!["/tmp/in.mp4".to_string()]),
            no_transitions: None,
            clip_duration_sec: None,
            transitions: None,
            no_effects: None,
            effect_paths: None,
            overlay_opacity: None,
            font: None,
            effects: None,
            overlays: None,
            max_duration_sec: None,
            audio_plan: None,
            audio_assets: None,
            copy_certification: None,
            render_plan: None,
            clip_plan: None,
        };
        assert_eq!(process(request.clone()).operation, "render_stock");
        let response = process(request);
        assert!(!response.ok);
        assert_eq!(response.operation, "render_stock");
        assert_eq!(
            response.error.as_deref(),
            Some("render_stock requires a canonical render_plan")
        );
    }

    #[test]
    fn resolved_render_plan_rejects_unknown_ids_and_invalid_paths() {
        let bad_transition = vec![RenderTransition {
            clip_index: 0,
            segment: "end".to_string(),
            id: "not-supported".to_string(),
        }];
        assert!(validate_resolved_render_plan(1, false, &bad_transition, true, &[]).is_err());
        let bad_effect = vec![RenderEffectPath {
            clip_index: 1,
            path: "/effects/one.mp4".to_string(),
        }];
        assert!(validate_resolved_render_plan(1, true, &[], false, &bad_effect).is_err());
    }

    #[test]
    fn supported_transition_catalog_matches_go_selection_ids() {
        for id in [
            "fadeblack",
            "fadewhite",
            "flash",
            "blur",
            "gray",
            "colorred",
            "colorblue",
            "colorgreen",
            "coloryellow",
            "colorpurple",
            "colororange",
            "colorpink",
            "negate",
            "vignette",
            "fastblur",
        ] {
            assert!(
                crate::render_stock::supported_transition(id),
                "unsupported contract ID: {id}"
            );
        }
    }

    #[test]
    fn unsupported_operations_fail_closed() {
        let invalid = serde_json::from_str::<Request>(
            r#"{
            "version":"mediaexec.v1",
            "operation":"run_command"
        }"#,
        );
        assert!(invalid.is_err());
    }
}
