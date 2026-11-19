//! `preflight` wire operation.
//!
//! The engine in [`crate::preflight`] only reports facts; this module adapts it
//! to the `mediaexec.v2` envelope. The envelope `ok` stays true whenever the
//! check ran, even if the report contains errors: a `false` envelope would make
//! the Go client discard the report and lose the reason. Translating
//! `PreflightReport.ok` into a blocking decision is the caller's job.

use crate::preflight::check;
use crate::protocol::{Request, Response};

pub(super) fn execute(request: Request) -> Response {
    let Some(mut payload) = request.preflight else {
        // `Request::validate` already rejects this shape; keep the arm
        // fail-closed instead of panicking if the call order ever changes.
        return Response {
            ok: false,
            operation: "preflight".to_string(),
            source_path: request.source_path,
            items: Vec::new(),
            metadata: None,
            preflight: None,
            error: Some("preflight is required".to_string()),
        };
    };
    // Reuse the envelope override the Go client already fills from the shared
    // executor, so a caller never has to repeat the ffmpeg path inside the
    // payload.
    if payload.ffmpeg_path.is_none() {
        payload.ffmpeg_path = request.ffmpeg_path;
    }
    let report = check(&payload);
    Response {
        ok: true,
        operation: "preflight".to_string(),
        source_path: None,
        items: Vec::new(),
        metadata: None,
        preflight: Some(report),
        error: None,
    }
}
