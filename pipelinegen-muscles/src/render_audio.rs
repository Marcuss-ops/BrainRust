#[path = "render_audio_execution.rs"]
mod execution;
#[path = "render_audio_plan.rs"]
mod plan;
#[path = "render_audio_probe.rs"]
mod probe;

/// Sealed compiled audio plan, re-exported so the protocol boundary holds
/// the typed plan instead of an untyped JSON value.
pub(crate) use plan::Plan as CompiledAudioPlan;

pub(crate) fn execute(request: crate::protocol::Request) -> crate::protocol::Response {
    execution::execute(request)
}
