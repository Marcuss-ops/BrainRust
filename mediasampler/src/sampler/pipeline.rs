//! Candidate ranking pipeline: `sample_scene`, `sample_scene_group` and the
//! per-candidate `evaluate` decision.

use crate::types::{Candidate, RejectionReason, SampleOptions, SampleResult, Scene};

use super::bound::BoundAssets;
use super::scoring::{
    action_relevance, context_relevance, diversity_bonus, is_subject_mismatch, semantic_score,
};

/// sample_scene ranks the candidates for one scene and returns a result per
/// candidate, plus the winner id (the highest-scoring accepted candidate).
/// The order of accepted results is score-desc, ties broken by candidate id
/// so the output is deterministic across 100/100 runs.
///
/// `bound` is the per-run ledger; the caller mutates it with the winner so
/// subsequent scenes see the binding (cross-scene reuse detection).
pub fn sample_scene(
    scene: &Scene,
    candidates: &[Candidate],
    options: &SampleOptions,
    bound: &mut BoundAssets,
) -> (Vec<SampleResult>, Option<String>) {
    let mut results = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        results.push(evaluate(scene, candidate, options, bound));
    }
    // Rank accepted candidates by score desc, then candidate id asc.
    let mut accepted: Vec<&SampleResult> = results.iter().filter(|r| r.is_accepted()).collect();
    accepted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.candidate_id.cmp(&b.candidate_id))
    });
    let winner = accepted.first().map(|r| {
        // Record the binding (with the winning label, for the V2 diversity
        // stage) so later scenes reject reuse and penalize overlaps.
        bound.bind_label(&r.candidate_id, &scene.id, &r.label);
        r.candidate_id.clone()
    });
    // Sort the full result list for stable output: accepted first (score
    // desc), then rejected (by reason, then id).
    results.sort_by(|a, b| match (a.is_accepted(), b.is_accepted()) {
        (true, true) => b
            .score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.candidate_id.cmp(&b.candidate_id)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => {
            let ra = format!("{:?}", a.rejection);
            let rb = format!("{:?}", b.rejection);
            ra.cmp(&rb).then(a.candidate_id.cmp(&b.candidate_id))
        }
    });
    (results, winner)
}

/// sample_scene_group is the `select_group` operation for MultiImage 2.0:
/// 2–5 assets for one scene, ranked by the same pipeline, all truly
/// distinct (an asset cannot appear twice in one group) and every selected
/// asset is recorded in the ledger like a single winner would be.
/// Determinism: score-desc, ties by candidate id, exactly like
/// `sample_scene`. The group may contain fewer than `count` assets when
/// fewer candidates are acceptable; the caller (Go planner) then picks a
/// smaller layout (x4 keeps working when the fifth image is missing) and
/// MUST NOT fill the slot with a duplicate or unverified asset.
///
/// `count` out of 2..=5 fails closed: a zero or one-asset group is a plain
/// winner (`select`), and anything beyond five is a contract error.
pub fn sample_scene_group(
    scene: &Scene,
    candidates: &[Candidate],
    options: &SampleOptions,
    bound: &mut BoundAssets,
    count: usize,
) -> Result<(Vec<SampleResult>, Vec<String>), String> {
    if !(2..=5).contains(&count) {
        return Err(format!("select_group requires 2..=5 assets, got {count}"));
    }
    let mut results = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        results.push(evaluate(scene, candidate, options, bound));
    }
    // Rank accepted candidates by score desc, then candidate id asc; dedupe
    // per id (never the same asset twice in one group).
    let mut accepted: Vec<&SampleResult> = results.iter().filter(|r| r.is_accepted()).collect();
    accepted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.candidate_id.cmp(&b.candidate_id))
    });
    let mut group = Vec::with_capacity(count);
    let mut seen = std::collections::HashSet::new();
    for r in accepted {
        if group.len() == count {
            break;
        }
        if !seen.insert(r.candidate_id.clone()) {
            continue;
        }
        group.push(r.candidate_id.clone());
    }
    // Every member of the group is bound, exactly like a single winner is.
    // The label travels with the binding to power the diversity stage.
    for id in &group {
        if let Some(result) = results.iter().find(|r| &r.candidate_id == id) {
            bound.bind_label(id, &scene.id, &result.label);
        } else {
            bound.bind(id, &scene.id);
        }
    }
    // Keep the stable full-result ordering (accepted first, score desc).
    results.sort_by(|a, b| match (a.is_accepted(), b.is_accepted()) {
        (true, true) => b
            .score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.candidate_id.cmp(&b.candidate_id)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => {
            let ra = format!("{:?}", a.rejection);
            let rb = format!("{:?}", b.rejection);
            ra.cmp(&rb).then(a.candidate_id.cmp(&b.candidate_id))
        }
    });
    Ok((results, group))
}

/// evaluate applies the full pipeline to one candidate and returns the
/// accept/reject decision. Hard constraints (ownership, reuse, subject
/// match) reject; soft scoring produces the final score.
fn evaluate(
    scene: &Scene,
    candidate: &Candidate,
    options: &SampleOptions,
    bound: &BoundAssets,
) -> SampleResult {
    // 1. ownership validation
    if !candidate.owner_segment_id.is_empty() && candidate.owner_segment_id != scene.id {
        return SampleResult::rejected(
            &candidate.id,
            &candidate.label,
            RejectionReason::OwnerMismatch,
            "candidate owner_segment_id does not match this scene",
        );
    }
    // 2. duplicate/reuse validation
    if !options.allow_reuse && bound.is_reuse(&candidate.id, &scene.id) {
        return SampleResult::rejected(
            &candidate.id,
            &candidate.label,
            RejectionReason::AlreadyBound,
            "candidate already bound to another scene (reuse forbidden)",
        );
    }
    // 3. subject relevance (hard constraint, data-driven tables)
    if is_subject_mismatch(scene, candidate, options) {
        return SampleResult::rejected(
            &candidate.id,
            &candidate.label,
            RejectionReason::SubjectMismatch,
            "candidate subject is not compatible with scene subject",
        );
    }
    // 4-7. soft scoring: action + context + semantic + diversity
    let action = action_relevance(candidate, options);
    let context = context_relevance(candidate, options);
    let diversity = diversity_bonus(candidate, bound, options);
    let semantic = semantic_score(scene, candidate);
    // Final score blends the generic similarity (provider signal) with the
    // semantic grounding (source signal). The semantic score dominates so a
    // high generic similarity cannot rescue a semantically weak candidate.
    let final_score = (semantic * 0.6)
        + (candidate.generic_similarity * 0.3)
        + (action * 0.05)
        + (context * 0.025)
        + (diversity * options.diversity_weight);
    let clamped = final_score.clamp(0.0, 1.0);
    SampleResult::accepted(
        &candidate.id,
        &candidate.label,
        clamped,
        "accepted: subject compatible, semantic-grounded",
    )
}
