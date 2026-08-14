use crate::protocol::{Request, Response};

#[path = "transform_assemble.rs"]
mod assemble;
#[path = "transform_merge.rs"]
mod merge;
#[path = "transform_mux.rs"]
mod mux;
#[path = "transform_operations.rs"]
mod operations;

pub(crate) fn execute(request: Request, operation: &str) -> Response {
    match operation {
        "merge_inputs" => merge::execute(request),
        "mux_audio_copy" => mux::execute(request),
        "assemble_copy" => assemble::execute(request),
        _ => operations::execute(request, operation),
    }
}
