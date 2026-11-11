//! Deterministic narrative continuity for the semantic manifest.
//!
//! The resolver is deliberately conservative: explicit names and the shared
//! identity registry are authoritative; pronouns and role nominals resolve only
//! when one compatible antecedent remains. Anything else stays `ambiguous` or
//! `unresolved` with its candidate ids and source evidence.

mod references;
mod resolution;
mod threads;

use semantic_core::narrative::{DiscourseCounts, DiscourseProjection, DISCOURSE_SCHEMA};
use semantic_core::{SceneInput, SceneManifest};

/// Builds the additive `discourse.v1` projection for one analysed script.
pub fn build(
    language: &str,
    inputs: &[SceneInput],
    scenes: &[SceneManifest],
) -> DiscourseProjection {
    let references = resolution::resolve_references(language, inputs, scenes);
    let (threads, scene_continuity) = threads::build(&references, scenes);
    let mut counts = DiscourseCounts {
        total: references.len(),
        threads: threads.len(),
        cross_scene_threads: threads
            .iter()
            .filter(|thread| thread.scene_ids.len() > 1)
            .count(),
        ..DiscourseCounts::default()
    };
    for reference in &references {
        match reference.status.as_str() {
            "resolved" => counts.resolved += 1,
            "ambiguous" => counts.ambiguous += 1,
            _ => counts.unresolved += 1,
        }
    }
    DiscourseProjection {
        schema: DISCOURSE_SCHEMA.to_string(),
        references,
        threads,
        scene_continuity,
        counts,
    }
}
