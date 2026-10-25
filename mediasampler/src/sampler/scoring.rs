//! Hard subject constraints and soft relevance/diversity scoring.

use crate::types::{Candidate, SampleOptions, Scene};

use super::bound::BoundAssets;
use super::tokens::label_tokens;

/// is_subject_mismatch reports whether the candidate's label is
/// semantically incompatible with the scene subject. A candidate is a
/// mismatch when its label references NONE of: the scene subject, any
/// visual term, OR any known-compatible keyword — AND references a
/// known mismatched subject. The keyword tables are data-driven (see
/// [`SampleOptions`]) and supplied per content domain; an empty table
/// disables that signal.
pub(crate) fn is_subject_mismatch(
    scene: &Scene,
    candidate: &Candidate,
    options: &SampleOptions,
) -> bool {
    let label = candidate.label.to_lowercase();
    let subject = scene.subject.to_lowercase();
    if subject.is_empty() {
        return false;
    }
    // If the label references the subject or any visual term, it is not a
    // mismatch.
    if label.contains(&subject) {
        return false;
    }
    for term in &scene.terms {
        let t = term.to_lowercase();
        if !t.is_empty() && (label.contains(&t) || t.contains(&label)) {
            return false;
        }
    }
    // If the label references a known-compatible keyword, it is not a
    // mismatch (a weak-but-acceptable backup for the scene's topic).
    for kw in options.compatible_keywords.iter() {
        if !kw.is_empty() && label.contains(&kw.to_lowercase()) {
            return false;
        }
    }
    // The label references none of the scene anchors. It is a mismatch
    // only when it references a known-incompatible subject. A label that
    // references nothing at all is treated as a weak backup (not a
    // mismatch) so under-labeled candidates are not false-positively
    // rejected.
    for bad in options.mismatched_subjects.iter() {
        if !bad.is_empty() && label.contains(&bad.to_lowercase()) {
            return true;
        }
    }
    false
}

/// action_relevance scores whether the candidate label references an
/// action verb compatible with the scene. Returns a small bonus in
/// `[0.0, 0.1]`. The verb table is data-driven (caller-supplied per domain).
pub(crate) fn action_relevance(candidate: &Candidate, options: &SampleOptions) -> f32 {
    let label = candidate.label.to_lowercase();
    for action in options.action_verbs.iter() {
        if !action.is_empty() && label.contains(&action.to_lowercase()) {
            return 0.1;
        }
    }
    0.0
}

/// context_relevance scores whether the candidate label references a
/// context keyword compatible with the scene. Returns a small bonus in
/// `[0.0, 0.1]`. The table is data-driven (caller-supplied per domain).
pub(crate) fn context_relevance(candidate: &Candidate, options: &SampleOptions) -> f32 {
    let label = candidate.label.to_lowercase();
    for ctx in options.context_keywords.iter() {
        if !ctx.is_empty() && label.contains(&ctx.to_lowercase()) {
            return 0.1;
        }
    }
    0.0
}

/// diversity_bonus is the V2 diversity stage: the penalty a candidate
/// incurs for being too similar to an asset already bound by an earlier
/// scene. With an empty ledger (first scene of a run), or when the caller
/// passes `diversity_weight = 0.0`, it returns 0.0 so the score is
/// byte-identical to V1. The overlap is Jaccard over lowercase word tokens
/// of the candidate label vs. the label each bound asset won under.
pub(crate) fn diversity_bonus(
    candidate: &Candidate,
    bound: &BoundAssets,
    options: &SampleOptions,
) -> f32 {
    if options.diversity_weight <= 0.0 {
        return 0.0;
    }
    let tokens = label_tokens(&candidate.label);
    if tokens.is_empty() {
        return 0.0;
    }
    if !bound.has_any_label() {
        // No binding ever carried a label: there is no overlap evidence, so
        // the stage stays V1-neutral instead of guessing similarity from ids.
        return 0.0;
    }
    let best = bound.best_label_overlap(&tokens);
    // Diversity value is INVERSE similarity: identical to a bound winner →
    // 0.0 (full penalty), no token in common → 1.0 (fully diverse).
    (1.0 - best).clamp(0.0, 1.0)
}

/// semantic_score computes the deterministic semantic grounding score in
/// `[0.0, 1.0]`. A candidate that references the scene subject scores
/// 0.9+; one that references a visual term scores 0.7+; otherwise it falls
/// back to the caller-supplied embedding similarity (when present) and to
/// the generic similarity. Source-derived anchors always dominate soft
/// signals, so a keyword-matched label can never be outranked by a raw
/// vector score.
pub(crate) fn semantic_score(scene: &Scene, candidate: &Candidate) -> f32 {
    let label = candidate.label.to_lowercase();
    let subject = scene.subject.to_lowercase();
    if !subject.is_empty() && label.contains(&subject) {
        return 0.95;
    }
    for term in &scene.terms {
        let t = term.to_lowercase();
        if !t.is_empty() && (label.contains(&t) || t.contains(&label)) {
            return 0.75;
        }
    }
    // No direct anchor match. The embedding similarity (a real vector
    // signal, when the caller supplies one) is the stronger grounding:
    // generic similarity alone is scaled down so a high provider score
    // cannot win on its own.
    let generic = candidate.generic_similarity * 0.5;
    match candidate.embedding_similarity {
        Some(value) => generic.max(value.clamp(0.0, 1.0) * 0.8),
        None => generic,
    }
}
