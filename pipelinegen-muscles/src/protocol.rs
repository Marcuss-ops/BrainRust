//! `mediaexec.v2` wire protocol.
//!
//! The module is split by contract area: requests and validation, overlay
//! manifests, responses and copy certification. Every type is re-exported here
//! so callers keep using `crate::protocol::*` regardless of the internal file
//! layout.

pub const PROTOCOL_VERSION: &str = "mediaexec.v2";

/// Assembly-ready output contract identity, V1 (frozen).
///
/// This is the ONE declaration of the string in this crate. It is the same
/// identity the Go assembly contract carries
/// (`internal/kernel/media.AssemblyMediaContractID`) and that the RenderingGen
/// media profile registry exposes; the certification and copy-assembly paths
/// must reference this constant instead of restating the literal, so a rename
/// cannot land in one language and not the others.
pub const VELOX_ASSEMBLY_READY_V1: &str = "VELOX_ASSEMBLY_READY_V1";

/// Assembly-ready output contract identity, V2: the canonical contract for
/// newly produced assembly-ready clips, and the id that gates the stricter
/// extradata/level checks.
pub const VELOX_ASSEMBLY_READY_V2: &str = "VELOX_ASSEMBLY_READY_V2";

pub const OVERLAY_MANIFEST_SCHEMA_V1: &str = "pipelinegen.overlay-manifest.v1";

mod certification;
mod overlay;
mod request;
mod response;

pub use certification::CopyCertification;
pub use overlay::{OverlayManifestEntry, OverlayManifestWire};
// Test-only modules exercise the overlay wire models through the protocol
// facade; production code only reaches them via the overlay submodule itself.
#[cfg(test)]
pub use overlay::{
    OverlayManifest, OverlayManifestAsset, OverlayManifestPlacement, OverlayManifestSet,
};
pub use request::{AudioAsset, CutJob, Operation, Request};
#[cfg(test)]
pub use request::{RenderEffectPath, RenderTransition};
pub use response::{AudioEditMetadata, CutItem, MediaMetadata, Response};

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
