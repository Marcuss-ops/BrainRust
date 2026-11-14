// render_clip.rs — the single-pass clip render operation (feature spec §6/§7).
//
// Executes a sealed ClipRenderPlanV1 with ONE FFmpeg invocation: background
// (none | blur_source | asset), aspect-fitted foreground, watermark overlay,
// optional libass subtitle burn, and the encoder from the canonical
// encoder/media policy (encoder.rs — the only builder allowed to emit video
// encoder arguments). Rust makes NO business selections: every value arrives
// resolved in the plan (geometry, watermark position/opacity/margin, subtitle
// mode, audio policy); the media config carries only the encoder policy.
//
// Audio copy policy (§9): copy_if_compatible probes the source audio stream
// and copies it verbatim when it already satisfies the plan's audio contract;
// otherwise exactly one certified conversion runs. The outcome is reported in
// the response metadata (audio_copy_eligible, audio_encode_passes).
//
// Honest GPU accounting (§7): burn subtitles rasterize via libass (CPU); the
// response reports subtitle_raster_cpu=true — the operation never claims a
// 100% GPU path when libass is in the graph.

// The operation is split by topic into normal modules: `exec` owns the
// request-level entry point, `helpers` owns the pure graph/audio builders and
// `plan` owns the sealed plan types. Each file has explicit imports and
// visibility; there is no `include!` fragment.
#[path = "render_clip_exec.rs"]
mod exec;
#[path = "render_clip_helpers.rs"]
mod helpers;
#[path = "render_clip_plan.rs"]
mod plan;

/// Sealed clip plan, re-exported so the protocol boundary holds the typed
/// plan instead of an untyped JSON value.
pub(crate) use plan::ClipRenderPlan as SealedClipPlan;

/// Request-level entry point used by the dispatcher.
pub(crate) use exec::render_clip;

#[cfg(test)]
#[path = "render_clip_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "render_clip_tests_support.rs"]
mod tests_support;
