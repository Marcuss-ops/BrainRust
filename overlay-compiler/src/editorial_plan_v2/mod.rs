//! Versioned request/response envelope for the TICKET-14 single-call cutover.
//!
//! The target shape is one `compile_editorial_plan` request whose request is
//! the complete planning input and whose response is the final overlay plan:
//!
//! ```text
//! manifest + scenes + timing + entities + media refs + canvas/toggles/budgets
//!         -> compile_editorial_plan (one request)
//!         -> final OverlayPlan (items, placements, intake, duration)
//! ```
//!
//! The envelope is additive: the identity/language/canvas/scene/editorial/
//! chapter slice landed first and keeps its meaning; TICKET-14 WP1 added the
//! seal extent, scene annotations, entity timeline, media refs, budgets and
//! canonical scene windows. The version is bumped only for a breaking change.
//!
//! Fail-closed by construction: unknown schema versions, malformed sections and
//! missing compile inputs are typed errors. There is no fallback planner and no
//! "empty plan" path here.

mod editorial;
mod error;
mod planner;
mod projection;
mod request;
mod response;

pub use editorial::project_editorial_scenes;
pub use error::EditorialPlanError;
pub use request::*;
pub use response::*;

/// Schema version of the full-plan request/response envelope.
pub const EDITORIAL_PLAN_V2_SCHEMA_VERSION: &str = "editorial_plan.v2";
/// The one wire operation name for both the legacy batch and the full call.
pub const EDITORIAL_PLAN_OPERATION: &str = "compile_editorial_plan";
/// Version of the per-scene editorial manifest carried by the request.
pub const EDITORIAL_MANIFEST_SCHEMA_VERSION: &str = "editorial.v1";
/// Version of the chapter manifest carried by the request.
pub const CHAPTER_MANIFEST_SCHEMA_VERSION: &str = "chapter_manifest.v1";
/// Certified editorial profile the projection accepts.
pub const EDITORIAL_PROFILE_VERSION: &str = "highlights.v1";

/// Entry point of the single-call planner: validate the complete request and
/// return the sealed plan. The request half is final; the planner phases run
/// in candidate order (editorial, entity cards, admission, cascade, seal) and
/// never fall back to a Go-side or empty plan.
pub fn compile_overlay_plan_v2(
    request: &EditorialPlanV2Request,
) -> Result<EditorialPlanV2Compiled, EditorialPlanError> {
    planner::compile_plan(request)
}
