//! Native execution components for PipelineGen.
//!
//! This crate owns capability-scoped media execution for the Rust migration.
//! Components here may perform CPU-heavy or process-level work, but must not
//! become an alternate source of canonical state.
//!
//! Initial boundary rules:
//! - SQLite remains owned by the Go application.
//! - Transactional outbox decisions remain owned by the Go application.
//! - Google Drive and Qdrant access must arrive through explicit contracts.
//! - FFmpeg invocation belongs behind a typed execution interface.
//! - No direct credentials or environment-file loading from this crate.

#![deny(unsafe_code)]

/// Stable placeholder proving that the crate is wired and testable.
pub const COMPONENT: &str = "pipelinegen-muscles";

mod admin_media;
mod artifact;
mod config;
mod cut;
mod encoder;
#[cfg(test)]
mod golden;
mod probe;
mod protocol;
mod render_stock;
mod transform;

use protocol::{Operation, Request, Response};
use std::io::{self, BufRead, Write};

pub(crate) use render_stock::reject_unresolved_selection;
#[cfg(test)]
pub(crate) use render_stock::{supported_transition, validate_resolved_render_plan};

/// Processes one capability request. The protocol exposes media capabilities,
/// never arbitrary command execution.
pub fn process(request: Request) -> Response {
    let operation = request.operation.as_str().to_string();
    if let Err(error) = request.validate() {
        return Response {
            ok: false,
            operation,
            source_path: request.source_path,
            items: Vec::new(),
            metadata: None,
            error: Some(error),
        };
    }
    if let Some(error) = reject_unresolved_selection(&request) {
        return Response {
            ok: false,
            operation,
            source_path: request.source_path,
            items: Vec::new(),
            metadata: None,
            error: Some(error),
        };
    }
    let mut response = match request.operation {
        Operation::Health => Response {
            ok: true,
            operation: "health".to_string(),
            source_path: None,
            items: Vec::new(),
            metadata: None,
            error: None,
        },
        Operation::CutBatch => cut::execute(request),
        Operation::Probe => probe::execute(request),
        Operation::Normalize => transform::execute(request, "normalize"),
        Operation::CutCopy => transform::execute(request, "cut_copy"),
        Operation::CutAndNormalize => transform::execute(request, "cut_and_normalize"),
        Operation::Watermark => transform::execute(request, "watermark"),
        Operation::ExtractFrame => transform::execute(request, "extract_frame"),
        Operation::GenerateProxy => transform::execute(request, "generate_proxy"),
        Operation::GenerateStoryboard => transform::execute(request, "generate_storyboard"),
        Operation::RemuxHls => transform::execute(request, "remux_hls"),
        Operation::Trim => transform::execute(request, "trim"),
        Operation::RenderStock => render_stock::execute(request),
        Operation::AdminRender => admin_media::execute(request),
    };
    if !response.ok {
        response.operation = operation;
    }
    response
}
pub fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
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
        serde_json::to_writer(&mut stdout, &response).map_err(io::Error::other)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        process, reject_unresolved_selection, validate_resolved_render_plan, Request, COMPONENT,
    };
    use crate::protocol::{Operation, RenderEffectPath, RenderTransition};

    #[test]
    fn component_name_is_stable() {
        assert_eq!(COMPONENT, "pipelinegen-muscles");
    }

    #[test]
    fn unresolved_selection_is_rejected_without_rust_side_selection() {
        let mut request = Request {
            version: "mediaexec.v1".to_string(),
            operation: Operation::RenderStock,
            ffmpeg_path: None,
            source_path: None,
            output_path: None,
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
            input_paths: None,
            no_transitions: None,
            clip_duration_sec: None,
            transitions: None,
            transition_every: Some(4),
            effects_dir: None,
            effect_every: None,
            effect_index_hint: None,
            no_effects: None,
            effect_paths: None,
            overlay_opacity: None,
            font: None,
            effects: None,
            overlays: None,
            max_duration_sec: None,
        };
        assert!(reject_unresolved_selection(&request).is_some());
        assert_eq!(process(request.clone()).operation, "render_stock");
        request.transition_every = None;
        assert!(reject_unresolved_selection(&request).is_none());
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
                super::supported_transition(id),
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
