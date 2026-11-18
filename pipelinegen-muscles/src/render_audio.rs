#[path = "render_audio_execution.rs"]
mod execution;
#[path = "render_audio_probe.rs"]
mod probe;

pub(crate) fn execute(request: crate::protocol::Request) -> crate::protocol::Response {
    execution::execute(request)
}
