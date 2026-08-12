use crate::artifact::failed_response;
use crate::protocol::{Request, Response};
use std::fs;
use std::path::Path;

#[path = "render_stock_canonical_plan.rs"]
mod plan;
#[path = "render_stock_canonical_executor.rs"]
mod executor;

pub(super) fn render_stock_canonical(request: Request) -> Response {
    let raw_plan = match request.render_plan.clone() {
        Some(value) => value,
        None => return failed_response(None, "render_plan is required".to_string()),
    };
    let plan = match plan::decode_and_validate(raw_plan) {
        Ok(plan) => plan,
        Err(error) => return failed_response(None, error),
    };
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
