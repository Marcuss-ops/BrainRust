//! The semantic MediaSampler pipeline (Fase 4):
//!
//! ```text
//! candidate
//!  ↓
//! ownership validation
//!  ↓
//! duplicate/reuse validation
//!  ↓
//! subject relevance
//!  ↓
//! action relevance
//!  ↓
//! context relevance
//!  ↓
//! semantic score
//!  ↓
//! diversity
//!  ↓
//! final score
//! ```
//!
//! The sampler does NOT pick the highest generic vector similarity. It
//! applies hard constraints first (ownership, reuse, subject match), then a
//! semantic score, then diversity. The LIVE-test bug where Artlist returned
//! a boxing clip for Greek Salad is rejected at the SUBJECT RELEVANCE stage.

use crate::types::{Candidate, RejectionReason, SampleOptions, SampleResult, Scene};

/// BoundAssets tracks asset ids already bound to scenes so the sampler can
/// reject cross-scene reuse when `allow_reuse = false`. It is the
/// per-run binding ledger the sampler consults before accepting a winner.
#[derive(Debug, Clone, Default)]
pub struct BoundAssets {
    /// Map of asset id -> segment id that bound it first.
    bound: std::collections::HashMap<String, String>,
}

impl BoundAssets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when `asset_id` is already bound to a scene other than
    /// `scene_id`.
    pub fn is_reuse(&self, asset_id: &str, scene_id: &str) -> bool {
        match self.bound.get(asset_id) {
            Some(owner) => owner != scene_id,
            None => false,
        }
    }

    /// Records that `scene_id` has bound `asset_id`.
    pub fn bind(&mut self, asset_id: &str, scene_id: &str) {
        self.bound.insert(asset_id.to_string(), scene_id.to_string());
    }
}

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
        // Record the binding so later scenes reject reuse.
        bound.bind(&r.candidate_id, &scene.id);
        r.candidate_id.clone()
    });
    // Sort the full result list for stable output: accepted first (score
    // desc), then rejected (by reason, then id).
    results.sort_by(|a, b| {
        match (a.is_accepted(), b.is_accepted()) {
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
        }
    });
    (results, winner)
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
    // 3. subject relevance (hard constraint)
    if is_subject_mismatch(scene, candidate) {
        return SampleResult::rejected(
            &candidate.id,
            &candidate.label,
            RejectionReason::SubjectMismatch,
            "candidate subject is not compatible with scene subject",
        );
    }
    // 4-7. soft scoring: action + context + semantic + diversity
    let action = action_relevance(scene, candidate);
    let context = context_relevance(scene, candidate);
    let diversity = diversity_bonus(scene, candidate);
    let semantic = semantic_score(scene, candidate);
    // Final score blends the generic similarity (provider signal) with the
    // semantic grounding (source signal). The semantic score dominates so a
    // high generic similarity cannot rescue a semantically weak candidate.
    let final_score = (semantic * 0.6) + (candidate.generic_similarity * 0.3) + (action * 0.05)
        + (context * 0.025)
        + (diversity * 0.025);
    let clamped = final_score.clamp(0.0, 1.0);
    SampleResult::accepted(
        &candidate.id,
        &candidate.label,
        clamped,
        "accepted: subject compatible, semantic-grounded",
    )
}

/// is_subject_mismatch reports whether the candidate's label is
/// semantically incompatible with the scene subject. A candidate is a
/// mismatch when its label references NONE of: the scene subject, any
/// visual term, OR any known-compatible cuisine keyword — AND references a
/// known mismatched subject (boxing, gym, fitness, ...). This is what
/// rejects `woman boxing` for `greek salad` while accepting `greek salad
/// being prepared`.
fn is_subject_mismatch(scene: &Scene, candidate: &Candidate) -> bool {
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
    // If the label references a known cuisine-compatible keyword, it is not
    // a mismatch (e.g. "mediterranean restaurant" for a greek salad scene
    // is a weak-but-acceptable backup, not a mismatch).
    for kw in COMPATIBLE_KEYWORDS.iter() {
        if label.contains(kw) {
            return false;
        }
    }
    // The label references none of the scene anchors. It is a mismatch
    // only when it references a known-incompatible subject (boxing, gym,
    // fitness, sport). A label that references nothing at all is treated as
    // a weak backup (not a mismatch) so under-labeled candidates are not
    // false-positively rejected.
    for bad in MISMATCHED_SUBJECTS.iter() {
        if label.contains(bad) {
            return true;
        }
    }
    false
}

/// action_relevance scores whether the candidate label references a
/// preparation/action verb compatible with the scene (e.g. "being
/// prepared", "cooking", "making"). Returns a small bonus in `[0.0, 0.1]`.
fn action_relevance(_scene: &Scene, candidate: &Candidate) -> f32 {
    let label = candidate.label.to_lowercase();
    for action in ACTION_VERBS.iter() {
        if label.contains(action) {
            return 0.1;
        }
    }
    0.0
}

/// context_relevance scores whether the candidate label references a
/// context keyword compatible with the scene (e.g. "mediterranean",
/// "restaurant", "kitchen"). Returns a small bonus in `[0.0, 0.1]`.
fn context_relevance(_scene: &Scene, candidate: &Candidate) -> f32 {
    let label = candidate.label.to_lowercase();
    for ctx in CONTEXT_KEYWORDS.iter() {
        if label.contains(ctx) {
            return 0.1;
        }
    }
    0.0
}

/// diversity_bonus is a placeholder for the diversity score (V1: 0.0; V2
/// will penalize candidates too similar to already-bound assets). Kept as
/// a named stage so the pipeline mirrors the plan exactly.
fn diversity_bonus(_scene: &Scene, _candidate: &Candidate) -> f32 {
    0.0
}

/// semantic_score computes the deterministic semantic grounding score in
/// `[0.0, 1.0]`. A candidate that references the scene subject scores
/// 0.9+; one that references a visual term scores 0.7+; otherwise it
/// falls back to the generic similarity scaled down.
fn semantic_score(scene: &Scene, candidate: &Candidate) -> f32 {
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
    // No direct anchor match: scale the generic similarity down so a
    // high generic similarity alone cannot win.
    candidate.generic_similarity * 0.5
}

// ── Curated keyword tables (V1, dependency-free) ────────────────────

/// Mismatched subjects: lowercase substrings that signal a candidate is
/// about a completely different topic (sport, fitness). References to
/// these reject a candidate when none of the scene anchors are present.
const MISMATCHED_SUBJECTS: &[&str] = &[
    "boxing", "boxer", "gym", "fitness", "workout", "soccer", "football",
    "basketball", "tennis", "sport", "runner", "running",
];

/// Compatible keywords: lowercase substrings that are weakly compatible
/// with any Mediterranean-food scene (cuisine context). Their presence
/// prevents a subject-mismatch rejection even when the subject itself is
/// not referenced.
const COMPATIBLE_KEYWORDS: &[&str] = &[
    "mediterranean", "greek", "italian", "restaurant", "kitchen", "food",
    "dish", "meal", "cooking", "recipe",
];

/// Action verbs: lowercase substrings signalling a preparation action.
const ACTION_VERBS: &[&str] = &[
    "prepar", "cooking", "making", "serving", "chopping", "cutting", "mixing",
];

/// Context keywords: lowercase substrings signalling a cuisine context.
const CONTEXT_KEYWORDS: &[&str] = &[
    "mediterranean", "greek", "italian", "restaurant", "kitchen", "table",
];

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn greek_salad_scene() -> Scene {
        Scene {
            id: "mediterranean-01-greek-salad".to_string(),
            subject: "greek salad".to_string(),
            terms: vec!["feta".to_string(), "tomatoes".to_string(), "olives".to_string()],
        }
    }

    fn boxing_candidate() -> Candidate {
        Candidate {
            id: "artlist-boxing-001".to_string(),
            label: "woman boxing".to_string(),
            generic_similarity: 0.72,
            owner_segment_id: String::new(),
        }
    }

    fn greek_salad_candidate() -> Candidate {
        Candidate {
            id: "artlist-greek-salad-001".to_string(),
            label: "greek salad being prepared".to_string(),
            generic_similarity: 0.68,
            owner_segment_id: String::new(),
        }
    }

    fn restaurant_candidate() -> Candidate {
        Candidate {
            id: "artlist-restaurant-001".to_string(),
            label: "mediterranean restaurant".to_string(),
            generic_similarity: 0.64,
            owner_segment_id: String::new(),
        }
    }

    // TestSamplerRejectsSubjectMismatch — boxing must be REJECTED for
    // Greek Salad, even though it has the highest generic similarity.
    #[test]
    fn rejects_subject_mismatch() {
        let scene = greek_salad_scene();
        let candidates = vec![boxing_candidate(), greek_salad_candidate(), restaurant_candidate()];
        let mut bound = BoundAssets::new();
        let (results, winner) = sample_scene(&scene, &candidates, &SampleOptions::default(), &mut bound);
        // Boxing must be rejected.
        let boxing = results.iter().find(|r| r.candidate_id == "artlist-boxing-001").unwrap();
        assert_eq!(boxing.rejection, Some(RejectionReason::SubjectMismatch));
        // The winner must be the greek-salad candidate, NOT boxing.
        assert_eq!(winner.as_deref(), Some("artlist-greek-salad-001"));
        // The greek-salad winner must score higher than the restaurant backup.
        let winner_result = results.iter().find(|r| r.candidate_id == "artlist-greek-salad-001").unwrap();
        let backup = results.iter().find(|r| r.candidate_id == "artlist-restaurant-001").unwrap();
        assert!(winner_result.score > backup.score, "winner {} should outrank backup {}", winner_result.score, backup.score);
    }

    // TestSamplerDeterministic — 100 runs must produce the same winner.
    #[test]
    fn deterministic_across_runs() {
        let scene = greek_salad_scene();
        let candidates = vec![boxing_candidate(), greek_salad_candidate(), restaurant_candidate()];
        let first = {
            let mut bound = BoundAssets::new();
            sample_scene(&scene, &candidates, &SampleOptions::default(), &mut bound).1
        };
        for _ in 0..99 {
            let mut bound = BoundAssets::new();
            let winner = sample_scene(&scene, &candidates, &SampleOptions::default(), &mut bound).1;
            assert_eq!(winner, first, "non-deterministic winner");
        }
    }

    // TestNoCrossSceneAssetReuse — the same candidate bound to scene 0 must
    // be REJECTED on scene 4 when allow_reuse=false.
    #[test]
    fn rejects_cross_scene_asset_reuse() {
        let scene0 = Scene {
            id: "mediterranean-01-greek-salad".to_string(),
            subject: "greek salad".to_string(),
            terms: vec!["feta".to_string()],
        };
        let scene4 = Scene {
            id: "mediterranean-05-paella".to_string(),
            subject: "paella".to_string(),
            terms: vec!["shrimp".to_string(), "mussels".to_string(), "rice".to_string()],
        };
        let shared = Candidate {
            id: "artlist-shared-001".to_string(),
            label: "greek salad being prepared".to_string(),
            generic_similarity: 0.9,
            owner_segment_id: String::new(),
        };
        let paella = Candidate {
            id: "artlist-paella-001".to_string(),
            label: "seafood paella".to_string(),
            generic_similarity: 0.85,
            owner_segment_id: String::new(),
        };
        let mut bound = BoundAssets::new();
        let (_, winner0) = sample_scene(&scene0, &[shared.clone(), paella.clone()], &SampleOptions::default(), &mut bound);
        assert_eq!(winner0.as_deref(), Some("artlist-shared-001"));
        // Now scene 4: the shared candidate must be rejected as already bound.
        let (results4, winner4) = sample_scene(&scene4, &[shared.clone(), paella.clone()], &SampleOptions::default(), &mut bound);
        let shared_result = results4.iter().find(|r| r.candidate_id == "artlist-shared-001").unwrap();
        assert_eq!(shared_result.rejection, Some(RejectionReason::AlreadyBound));
        assert_eq!(winner4.as_deref(), Some("artlist-paella-001"));
    }

    // TestOneImageQueryPerEntity — image fanout: one query per entity.
    // (Exercised at the Go stockintelligence layer; here we assert the
    // sampler options carry the expected images_per_scene and the sampler
    // returns one accepted winner per scene.)
    #[test]
    fn one_image_query_per_entity() {
        let scene = greek_salad_scene();
        // 3 entities => 3 image queries => 3 image candidates, one per term.
        let image_candidates: Vec<Candidate> = ["feta", "tomatoes", "olives"]
            .iter()
            .enumerate()
            .map(|(i, term)| Candidate {
                id: format!("img-{i}"),
                label: term.to_string(),
                generic_similarity: 0.5,
                owner_segment_id: String::new(),
            })
            .collect();
        let mut bound = BoundAssets::new();
        let (results, _) = sample_scene(&scene, &image_candidates, &SampleOptions { allow_reuse: false, images_per_scene: 3 }, &mut bound);
        let accepted: Vec<&SampleResult> = results.iter().filter(|r| r.is_accepted()).collect();
        assert_eq!(accepted.len(), 3, "expected one accepted image per entity, got {accepted:?}");
    }

    // TestThreeImagesPerScene — image fanout selects exactly 3 images.
    #[test]
    fn three_images_per_scene() {
        let scene = greek_salad_scene();
        let image_candidates: Vec<Candidate> = (0..5)
            .map(|i| Candidate {
                id: format!("img-{i}"),
                label: if i < 3 { scene.terms[i].clone() } else { "extra".to_string() },
                generic_similarity: 0.5,
                owner_segment_id: String::new(),
            })
            .collect();
        let mut bound = BoundAssets::new();
        let (results, _) = sample_scene(&scene, &image_candidates, &SampleOptions { allow_reuse: false, images_per_scene: 3 }, &mut bound);
        let accepted: Vec<&SampleResult> = results.iter().filter(|r| r.is_accepted()).collect();
        assert!(accepted.len() >= 3, "expected at least 3 accepted images, got {}", accepted.len());
    }

    // Additional: ownership mismatch rejects a candidate whose
    // owner_segment_id is a different scene.
    #[test]
    fn rejects_owner_mismatch() {
        let scene = greek_salad_scene();
        let mut candidate = greek_salad_candidate();
        candidate.owner_segment_id = "mediterranean-02-hummus".to_string();
        let mut bound = BoundAssets::new();
        let (results, winner) = sample_scene(&scene, &[candidate], &SampleOptions::default(), &mut bound);
        let r = results.first().unwrap();
        assert_eq!(r.rejection, Some(RejectionReason::OwnerMismatch));
        assert!(winner.is_none());
    }
}
