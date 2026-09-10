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

use crate::artifact::{failed_response, part_path, publish_output};
use crate::config::VideoProfile;
use crate::encoder::append_video_args;
use crate::probe;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[path = "render_clip_plan.rs"]
mod plan;

use plan::{
    ClipPlanAudio, ClipPlanWatermark, ClipRenderPlan, AUDIO_COPY_IF_COMPATIBLE, BACKGROUND_ASSET,
    BACKGROUND_NONE, SUBTITLE_BURN,
};

// The module body is composed from sibling files so that every physical file
// stays ≤400 lines while the compiled module keeps the former single-file
// semantics (same items, same visibility, same order):
//   render_clip_exec.rs    — include! fragment: pub(super) fn render_clip
//   render_clip_helpers.rs — include! fragment: BenchAccumulator + builders
//   render_clip_tests.rs   — #[cfg(test)] mod tests (the test assertions)
//   render_clip_tests_support.rs — #[cfg(test)] test fixtures (plan/profile/…)
include!("render_clip_exec.rs");
include!("render_clip_helpers.rs");

#[cfg(test)]
#[path = "render_clip_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "render_clip_tests_support.rs"]
mod tests_support;
