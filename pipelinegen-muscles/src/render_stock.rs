use crate::artifact::failed_response;
use crate::protocol::{Request, Response};
use std::fs;
use std::path::Path;

#[path = "render_stock_effects.rs"]
mod effects;
#[path = "render_stock_canonical_executor.rs"]
mod executor;
#[path = "render_stock_canonical_plan.rs"]
pub(crate) mod plan;

/// Sealed canonical plan, re-exported so the protocol boundary holds the
/// typed plan instead of an untyped JSON value.
pub(crate) use plan::CanonicalRenderPlan as SealedCanonicalPlan;

// Runtime stock rendering is canonical-plan-only. Legacy render implementations
// have been physically removed; requests without render_plan fail closed.
pub(crate) fn execute(request: Request) -> Response {
    // The plan arrives typed from the protocol boundary; only the semantic
    // validation runs here (shape parsing already failed the request
    // fail-closed when malformed).
    let plan = match request.render_plan.clone() {
        Some(value) => value,
        None => {
            return failed_response(
                None,
                "render_stock requires a canonical render_plan".to_string(),
            );
        }
    };
    if let Err(error) = plan::validate_canonical_plan(&plan) {
        return failed_response(None, error);
    }
    let output = match request.output_path.as_deref() {
        Some(path) if !path.is_empty() => path,
        _ => return failed_response(None, "output_path is required".to_string()),
    };
    let profile = match request.media.profile() {
        Ok(profile) => profile,
        Err(error) => return failed_response(None, error),
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    if let Some(parent) = Path::new(output).parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return failed_response(None, format!("create output directory: {error}"));
        }
    }
    executor::execute_canonical_render(&plan, &request, output, ffmpeg, &profile)
}

// These two are consumed by the dispatcher tests through the render_stock
// path; production code reaches them through the effects module directly.
#[cfg(test)]
pub(crate) use effects::{supported_transition, validate_resolved_render_plan};
