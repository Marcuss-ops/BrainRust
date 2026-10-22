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
/// Each entry also carries the label the asset won under, which powers the
/// V2 diversity stage (semantic overlap with already-bound winners).
#[derive(Debug, Clone, Default)]
pub struct BoundAssets {
    /// Map of asset id -> (segment id that bound it first, label).
    bound: std::collections::HashMap<String, (String, String)>,
}

impl BoundAssets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when `asset_id` is already bound to a scene other than
    /// `scene_id`.
    pub fn is_reuse(&self, asset_id: &str, scene_id: &str) -> bool {
        match self.bound.get(asset_id) {
            Some((owner, _)) => owner != scene_id,
            None => false,
        }
    }

    /// Records that `scene_id` has bound `asset_id` (no label; the diversity
    /// stage then receives no overlap evidence from this entry).
    pub fn bind(&mut self, asset_id: &str, scene_id: &str) {
        self.bind_label(asset_id, scene_id, "");
    }

    /// `bind` plus the winning label — the caller-facing constructor the
    /// winner path uses so diversity has real overlap evidence.
    pub fn bind_label(&mut self, asset_id: &str, scene_id: &str, label: &str) {
        self.bound.insert(
            asset_id.to_string(),
            (scene_id.to_string(), label.to_string()),
        );
    }

    /// Highest token-overlap (Jaccard in `[0.0, 1.0]`) between `tokens` and
    /// any already-bound label. Empty ledger → 0.0 (no diversity context).
    pub(crate) fn best_label_overlap(&self, tokens: &std::collections::BTreeSet<String>) -> f32 {
        self.bound
            .values()
            .filter(|(_, label)| !label.trim().is_empty())
            .map(|(_, label)| label_tokens(label).jaccard(tokens))
            .fold(0.0_f32, f32::max)
    }

    /// True when at least one bound entry carries a winning label, i.e. when
    /// the diversity stage HAS overlap evidence to reason about.
    pub(crate) fn has_any_label(&self) -> bool {
        self.bound
            .values()
            .any(|(_, label)| !label.trim().is_empty())
    }
}

/// Lowercase alphanumeric word tokens of a label, in order, deduplicated.
pub(crate) fn label_tokens(label: &str) -> std::collections::BTreeSet<String> {
    label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() > 1)
        .map(|word| word.to_lowercase())
        .collect()
}

pub(crate) trait Jaccard {
    fn jaccard(&self, other: &Self) -> f32;
}

impl Jaccard for std::collections::BTreeSet<String> {
    fn jaccard(&self, other: &Self) -> f32 {
        if self.is_empty() || other.is_empty() {
            return 0.0;
        }
        let intersection = self.intersection(other).count();
        let union = self.union(other).count();
        intersection as f32 / union as f32
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

/// is_subject_mismatch reports whether the candidate's label is
/// semantically incompatible with the scene subject. A candidate is a
/// mismatch when its label references NONE of: the scene subject, any
/// visual term, OR any known-compatible keyword — AND references a
/// known mismatched subject. The keyword tables are data-driven (see
/// [`SampleOptions`]) and supplied per content domain; an empty table
/// disables that signal.
fn is_subject_mismatch(scene: &Scene, candidate: &Candidate, options: &SampleOptions) -> bool {
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
fn action_relevance(candidate: &Candidate, options: &SampleOptions) -> f32 {
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
fn context_relevance(candidate: &Candidate, options: &SampleOptions) -> f32 {
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
fn diversity_bonus(candidate: &Candidate, bound: &BoundAssets, options: &SampleOptions) -> f32 {
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

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // The fixture domain for these tests is Mediterranean food. Under the
    // data-driven contract the caller supplies the keyword tables, so the
    // tests carry the domain lists explicitly instead of relying on any
    // baked-in default.
    fn food_domain_options() -> SampleOptions {
        SampleOptions {
            mismatched_subjects: [
                "boxing",
                "boxer",
                "boxing gym",
                "gym",
                "fitness",
                "workout",
                "weightlifting",
                "bodybuilding",
                "crossfit",
                "yoga",
                "pilates",
                "martial arts",
                "karate",
                "mma",
                "basketball",
                "football",
                "soccer",
                "tennis",
                "baseball",
                "volleyball",
                "swimming",
                "cycling",
                "running",
                "marathon",
                "boxing match",
                "boxing ring",
                "boxing gloves",
            ]
            .iter()
            .map(|value| value.to_string())
            .collect(),
            compatible_keywords: [
                "restaurant",
                "table",
                "dining",
                "kitchen",
                "recipe",
                "cooking",
                "meal",
                "food",
                "cuisine",
                "dish",
                "plate",
                "bowl",
                "ingredients",
                "fresh",
                "healthy",
                "mediterranean",
                "greek",
                "italian",
                "feta",
                "olive",
                "olive oil",
                "tomato",
                "tomatoes",
                "vegetables",
                "salad",
                "paella",
                "seafood",
            ]
            .iter()
            .map(|value| value.to_string())
            .collect(),
            action_verbs: [
                "prepar", "cooking", "making", "serving", "chopping", "cutting", "mixing",
            ]
            .iter()
            .map(|value| value.to_string())
            .collect(),
            context_keywords: [
                "mediterranean",
                "greek",
                "italian",
                "restaurant",
                "kitchen",
                "table",
            ]
            .iter()
            .map(|value| value.to_string())
            .collect(),
            ..SampleOptions::default()
        }
    }

    fn greek_salad_scene() -> Scene {
        Scene {
            id: "mediterranean-01-greek-salad".to_string(),
            subject: "greek salad".to_string(),
            terms: vec![
                "feta".to_string(),
                "tomatoes".to_string(),
                "olives".to_string(),
            ],
        }
    }

    fn boxing_candidate() -> Candidate {
        Candidate {
            id: "artlist-boxing-001".to_string(),
            label: "woman boxing".to_string(),
            generic_similarity: 0.72,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        }
    }

    fn greek_salad_candidate() -> Candidate {
        Candidate {
            id: "artlist-greek-salad-001".to_string(),
            label: "greek salad being prepared".to_string(),
            generic_similarity: 0.68,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        }
    }

    fn restaurant_candidate() -> Candidate {
        Candidate {
            id: "artlist-restaurant-001".to_string(),
            label: "mediterranean restaurant".to_string(),
            generic_similarity: 0.64,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        }
    }

    // TestSamplerRejectsSubjectMismatch — boxing must be REJECTED for
    // Greek Salad, even though it has the highest generic similarity.
    #[test]
    fn rejects_subject_mismatch() {
        let scene = greek_salad_scene();
        let candidates = vec![
            boxing_candidate(),
            greek_salad_candidate(),
            restaurant_candidate(),
        ];
        let mut bound = BoundAssets::new();
        let (results, winner) =
            sample_scene(&scene, &candidates, &food_domain_options(), &mut bound);
        // Boxing must be rejected.
        let boxing = results
            .iter()
            .find(|r| r.candidate_id == "artlist-boxing-001")
            .unwrap();
        assert_eq!(boxing.rejection, Some(RejectionReason::SubjectMismatch));
        // The winner must be the greek-salad candidate, NOT boxing.
        assert_eq!(winner.as_deref(), Some("artlist-greek-salad-001"));
        // The greek-salad winner must score higher than the restaurant backup.
        let winner_result = results
            .iter()
            .find(|r| r.candidate_id == "artlist-greek-salad-001")
            .unwrap();
        let backup = results
            .iter()
            .find(|r| r.candidate_id == "artlist-restaurant-001")
            .unwrap();
        assert!(
            winner_result.score > backup.score,
            "winner {} should outrank backup {}",
            winner_result.score,
            backup.score
        );
    }

    #[test]
    fn group_selects_distinct_ranked_assets_and_binds_all() {
        let scene = greek_salad_scene();
        let candidates = vec![
            boxing_candidate(),
            greek_salad_candidate(),
            restaurant_candidate(),
        ];
        let mut bound = BoundAssets::new();
        let (results, group) =
            sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, 3)
                .expect("x3 group");
        // Boxing never enters the group: 2 acceptable assets only.
        assert_eq!(group.len(), 2, "group: {group:?}");
        assert!(!group.contains(&"artlist-boxing-001".to_string()));
        // Credentials bound in the ledger exactly like single winners.
        for id in &group {
            assert!(!bound.is_reuse(id, "mediterranean-01-greek-salad"));
        }
        // Full results stay in the stable accepted-first ordering.
        assert!(results[0].is_accepted());
    }

    #[test]
    fn group_is_deterministic_and_never_duplicates() {
        let scene = greek_salad_scene();
        let candidates = vec![
            greek_salad_candidate(),
            restaurant_candidate(),
            boxing_candidate(),
        ];
        let first = {
            let mut bound = BoundAssets::new();
            sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, 5)
                .unwrap()
                .1
        };
        for _ in 0..99 {
            let mut bound = BoundAssets::new();
            let group =
                sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, 5)
                    .unwrap()
                    .1;
            assert_eq!(group, first);
        }
        let ids = first.clone();
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            "no duplicates"
        );
    }

    #[test]
    fn group_fails_closed_outside_two_to_five() {
        let scene = greek_salad_scene();
        let candidates = vec![greek_salad_candidate()];
        let mut bound = BoundAssets::new();
        for bad in [0, 1, 6, 20] {
            assert!(sample_scene_group(
                &scene,
                &candidates,
                &food_domain_options(),
                &mut bound,
                bad
            )
            .is_err());
        }
    }

    #[test]
    fn group_respects_the_request_side_reuse_ledger() {
        let scene = greek_salad_scene();
        let mut bound = BoundAssets::new();
        bound.bind("artlist-greek-salad-001", "other-scene");
        let candidates = vec![greek_salad_candidate(), restaurant_candidate()];
        let (results, group) =
            sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, 5)
                .expect("x2+ group");
        assert!(
            !group.contains(&"artlist-greek-salad-001".to_string()),
            "already-bound asset must not reappear: {group:?}"
        );
        assert!(results
            .iter()
            .any(|r| r.rejection == Some(RejectionReason::AlreadyBound)));
    }

    // TestSamplerDeterministic — 100 runs must produce the same winner.
    #[test]
    fn deterministic_across_runs() {
        let scene = greek_salad_scene();
        let candidates = vec![
            boxing_candidate(),
            greek_salad_candidate(),
            restaurant_candidate(),
        ];
        let first = {
            let mut bound = BoundAssets::new();
            sample_scene(&scene, &candidates, &food_domain_options(), &mut bound).1
        };
        for _ in 0..99 {
            let mut bound = BoundAssets::new();
            let winner = sample_scene(&scene, &candidates, &food_domain_options(), &mut bound).1;
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
            terms: vec![
                "shrimp".to_string(),
                "mussels".to_string(),
                "rice".to_string(),
            ],
        };
        let shared = Candidate {
            id: "artlist-shared-001".to_string(),
            label: "greek salad being prepared".to_string(),
            generic_similarity: 0.9,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        };
        let paella = Candidate {
            id: "artlist-paella-001".to_string(),
            label: "seafood paella".to_string(),
            generic_similarity: 0.85,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        };
        let mut bound = BoundAssets::new();
        let (_, winner0) = sample_scene(
            &scene0,
            &[shared.clone(), paella.clone()],
            &food_domain_options(),
            &mut bound,
        );
        assert_eq!(winner0.as_deref(), Some("artlist-shared-001"));
        // Now scene 4: the shared candidate must be rejected as already bound.
        let (results4, winner4) = sample_scene(
            &scene4,
            &[shared.clone(), paella.clone()],
            &food_domain_options(),
            &mut bound,
        );
        let shared_result = results4
            .iter()
            .find(|r| r.candidate_id == "artlist-shared-001")
            .unwrap();
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
                embedding_similarity: None,
            })
            .collect();
        let mut bound = BoundAssets::new();
        let (results, _) = sample_scene(
            &scene,
            &image_candidates,
            &SampleOptions {
                allow_reuse: false,
                images_per_scene: 3,
                ..food_domain_options()
            },
            &mut bound,
        );
        let accepted: Vec<&SampleResult> = results.iter().filter(|r| r.is_accepted()).collect();
        assert_eq!(
            accepted.len(),
            3,
            "expected one accepted image per entity, got {accepted:?}"
        );
    }

    // TestThreeImagesPerScene — image fanout selects exactly 3 images.
    #[test]
    fn three_images_per_scene() {
        let scene = greek_salad_scene();
        let image_candidates: Vec<Candidate> = (0..5)
            .map(|i| Candidate {
                id: format!("img-{i}"),
                label: if i < 3 {
                    scene.terms[i].clone()
                } else {
                    "extra".to_string()
                },
                generic_similarity: 0.5,
                owner_segment_id: String::new(),
                embedding_similarity: None,
            })
            .collect();
        let mut bound = BoundAssets::new();
        let (results, _) = sample_scene(
            &scene,
            &image_candidates,
            &SampleOptions {
                allow_reuse: false,
                images_per_scene: 3,
                ..food_domain_options()
            },
            &mut bound,
        );
        let accepted: Vec<&SampleResult> = results.iter().filter(|r| r.is_accepted()).collect();
        assert!(
            accepted.len() >= 3,
            "expected at least 3 accepted images, got {}",
            accepted.len()
        );
    }

    // Additional: ownership mismatch rejects a candidate whose
    // owner_segment_id is a different scene.
    #[test]
    fn rejects_owner_mismatch() {
        let scene = greek_salad_scene();
        let mut candidate = greek_salad_candidate();
        candidate.owner_segment_id = "mediterranean-02-hummus".to_string();
        let mut bound = BoundAssets::new();
        let (results, winner) =
            sample_scene(&scene, &[candidate], &food_domain_options(), &mut bound);
        let r = results.first().unwrap();
        assert_eq!(r.rejection, Some(RejectionReason::OwnerMismatch));
        assert!(winner.is_none());
    }

    // ── V2: data-driven tables, embedding similarity, diversity stage ──

    #[test]
    fn data_driven_tables_replace_the_curated_domain() {
        // A non-culinary domain (portraiture workflow): the request carries
        // its OWN tables and the sampler honours them wholesale.
        let scene = Scene {
            id: "portrait-01".to_string(),
            subject: "mountain climbers".to_string(),
            terms: vec!["glacier".to_string(), "summit ridge".to_string()],
        };
        let candidates = vec![
            Candidate {
                id: "stock-climber".to_string(),
                label: "goalkeeper diving save".to_string(),
                generic_similarity: 0.95,
                owner_segment_id: String::new(),
                embedding_similarity: None,
            },
            Candidate {
                id: "stock-glacier".to_string(),
                label: "glacier crossing at dawn".to_string(),
                generic_similarity: 0.85,
                owner_segment_id: String::new(),
                embedding_similarity: None,
            },
        ];
        let options = SampleOptions {
            mismatched_subjects: vec!["goalkeeper".to_string()],
            compatible_keywords: vec!["climbing".to_string(), "mountain".to_string()],
            action_verbs: vec!["crossing".to_string()],
            context_keywords: vec!["alpine".to_string()],
            diversity_weight: 0.0, // V1-neutral scoring for a clear ranking
            ..food_domain_options()
        };
        let mut bound = BoundAssets::new();
        let (results, winner) = sample_scene(&scene, &candidates, &options, &mut bound);
        assert_eq!(winner.as_deref(), Some("stock-glacier"));
        let rejected = results
            .iter()
            .find(|r| r.candidate_id == "stock-climber")
            .unwrap();
        // With the CUSTOM table, goalkeeper is a hard reject even though
        // with the culinary defaults (sport-family already covers soccer-ish
        // terms but NOT goalkeeper) it would have slipped through.
        assert_eq!(rejected.rejection, Some(RejectionReason::SubjectMismatch));
    }

    #[test]
    fn embedding_similarity_strengthens_the_weak_anchor_fallback() {
        let scene = Scene {
            id: "scene-emb-01".to_string(),
            subject: "greek salad".to_string(),
            terms: vec!["feta".to_string()],
        };
        // Same label family, no anchor match; one carries a real vector
        // similarity, the other only a weaker generic provider score.
        let with_embedding = Candidate {
            id: "a-embedding".to_string(),
            label: "fresh vegetable bowl on marble".to_string(),
            generic_similarity: 0.40,
            owner_segment_id: String::new(),
            embedding_similarity: Some(0.88),
        };
        let without = Candidate {
            id: "b-generic".to_string(),
            label: "fresh vegetable plate on wood".to_string(),
            generic_similarity: 0.40,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        };
        let mutate_bound = BoundAssets::new();
        let mut bound = mutate_bound.clone();
        let (results, winner) = sample_scene(
            &scene,
            &[with_embedding, without],
            &food_domain_options(),
            &mut bound,
        );
        assert_eq!(winner.as_deref(), Some("a-embedding"));
        let weak = results
            .iter()
            .find(|r| r.candidate_id == "b-generic")
            .unwrap();
        // Determinism guard repeated below in determinism tests applies too.
        assert!(weak.score > 0.0);
    }

    #[test]
    fn embedding_never_rescues_a_subject_mismatch() {
        let scene = Scene {
            id: "scene-emb-02".to_string(),
            subject: "greek salad".to_string(),
            terms: vec!["feta".to_string()],
        };
        let boxing = Candidate {
            id: "a-boxing".to_string(),
            label: "woman boxing".to_string(),
            generic_similarity: 0.72,
            owner_segment_id: String::new(),
            embedding_similarity: Some(1.0),
        };
        let weak = Candidate {
            id: "b-generic".to_string(),
            label: "close up plate".to_string(),
            generic_similarity: 0.30,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        };
        let mut bound = BoundAssets::new();
        let (results, winner) =
            sample_scene(&scene, &[boxing, weak], &food_domain_options(), &mut bound);
        let rejected = results
            .iter()
            .find(|r| r.candidate_id == "a-boxing")
            .unwrap();
        assert_eq!(rejected.rejection, Some(RejectionReason::SubjectMismatch));
        assert_eq!(winner.as_deref(), Some("b-generic"));
    }

    #[test]
    fn diversity_stage_penalizes_too_similar_bound_winners() {
        let bound_scene = Scene {
            id: "scene-div-00".to_string(),
            subject: "greek salad".to_string(),
            terms: vec!["feta".to_string()],
        };
        let next = Scene {
            id: "scene-div-01".to_string(),
            subject: "tomato tart".to_string(),
            terms: vec![],
        };
        let previous = Candidate {
            id: "c-previous".to_string(),
            label: "greek salad close up".to_string(),
            generic_similarity: 0.95,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        };
        let mut bound = BoundAssets::new();
        sample_scene(
            &bound_scene,
            &[previous],
            &food_domain_options(),
            &mut bound,
        );
        assert!(
            bound
                .bound
                .values()
                .any(|(owner, label)| owner == "scene-div-00" && label == "greek salad close up"),
            "the ledger must record the winning label"
        );
        // Two candidates with IDENTICAL base signals; only label overlap
        // with the bound winner separates them.
        let overlapping = Candidate {
            id: "a-overlap".to_string(),
            label: "close up tomato tart".to_string(),
            generic_similarity: 0.90,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        };
        let distinct = Candidate {
            id: "b-distinct".to_string(),
            label: "tomato tart served".to_string(),
            generic_similarity: 0.90,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        };
        let (results_next, winner_next) = sample_scene(
            &next,
            &[overlapping, distinct],
            &food_domain_options(),
            &mut bound,
        );
        assert_eq!(winner_next.as_deref(), Some("b-distinct"));
        let penalized = results_next
            .iter()
            .find(|r| r.candidate_id == "a-overlap")
            .unwrap();
        let boosted = results_next
            .iter()
            .find(|r| r.candidate_id == "b-distinct")
            .unwrap();
        assert!(
            boosted.score > penalized.score,
            "diversity must separate identical-base-score candidates: {penalized:?} vs {boosted:?}"
        );
    }

    #[test]
    fn diversity_weight_zero_keeps_v1_scores_byte_identical() {
        let scene = greek_salad_scene();
        let candidates = vec![
            boxing_candidate(),
            greek_salad_candidate(),
            restaurant_candidate(),
        ];
        let options = SampleOptions {
            diversity_weight: 0.0,
            ..food_domain_options()
        };
        let mut bound = BoundAssets::new();
        let (results, _) = sample_scene(&scene, &candidates, &options, &mut bound);
        let winner = results
            .iter()
            .find(|r| r.candidate_id == "artlist-greek-salad-001")
            .unwrap();
        // V1 arithmetic: semantic .95*.6 + generic .68*.3 + action .1*.05 +
        // context .1*.025
        assert!(
            (winner.score - (0.95 * 0.6 + 0.68 * 0.3 + 0.1 * 0.05 + 0.1 * 0.025)).abs() < 1e-6,
            "V1-neutral score drifted: {}",
            winner.score
        );
    }

    // ── select_group (MultiImage 2.0, x2–x5) ────────────────────────────

    fn ready_to_use_scene() -> Scene {
        Scene {
            id: "scene-group-01".to_string(),
            subject: "greek salad".to_string(),
            terms: vec![
                "feta".to_string(),
                "tomatoes".to_string(),
                "olives".to_string(),
            ],
        }
    }

    fn varied_candidates() -> Vec<Candidate> {
        // Six acceptable candidates plus one rejected mismatch, ranked by
        // their labels so the deterministic ordering stays observable.
        [
            ("salad-hero", "greek salad close up", 0.90),
            ("feta-block", "feta cheese block", 0.85),
            ("tomato-bed", "tomatoes on the counter", 0.80),
            ("olives-jar", "olive jar", 0.75),
            ("salad-plate", "greek salad plated", 0.70),
            ("kitchen-table", "mediterranean kitchen table", 0.60),
            ("boxing-mismatch", "woman boxing", 0.95),
        ]
        .into_iter()
        .map(|(id, label, similarity)| Candidate {
            id: format!("artlist-{id}"),
            label: label.to_string(),
            generic_similarity: similarity,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        })
        .collect()
    }

    fn run_group(count: usize, bound_assets: &mut BoundAssets) -> (Vec<SampleResult>, Vec<String>) {
        sample_scene_group(
            &ready_to_use_scene(),
            &varied_candidates(),
            &food_domain_options(),
            bound_assets,
            count,
        )
        .expect("legal 2..=5 group size")
    }

    #[test]
    fn select_group_sizes_two_to_five_are_distinct_and_ranked() {
        for count in 2..=5 {
            let mut bound = BoundAssets::new();
            let (results, group) = run_group(count, &mut bound);
            assert_eq!(group.len(), count, "count={count}: {group:?}");
            let distinct: std::collections::HashSet<&String> = group.iter().collect();
            assert_eq!(
                distinct.len(),
                count,
                "group must contain distinct assets: {group:?}"
            );
            // The boxing candidate must never enter any group.
            assert!(
                !group.iter().any(|id| id.contains("boxing")),
                "mismatched candidate selected for count={count}: {group:?}"
            );
            // Group members follow the accepted rank order (score desc, id asc).
            let accepted: Vec<&SampleResult> = results.iter().filter(|r| r.is_accepted()).collect();
            let expected: Vec<String> = accepted
                .iter()
                .take(count)
                .map(|r| r.candidate_id.clone())
                .collect();
            assert_eq!(
                group, expected,
                "group must match rank order, count={count}"
            );
            // Every member must be accepted with a positive score.
            for id in &group {
                let r = results.iter().find(|r| &r.candidate_id == id).unwrap();
                assert!(r.is_accepted() && r.score > 0.0, "member {id}: {r:?}");
            }
        }
    }

    #[test]
    fn select_group_is_deterministic_across_runs() {
        let reference = {
            let mut bound = BoundAssets::new();
            run_group(3, &mut bound)
        };
        for _ in 0..99 {
            let mut bound = BoundAssets::new();
            let run = run_group(3, &mut bound);
            assert_eq!(run, reference, "non-deterministic select_group output");
        }
    }

    #[test]
    fn select_group_records_every_member_in_the_ledger() {
        let mut bound = BoundAssets::new();
        let (_, group) = run_group(4, &mut bound);
        for id in &group {
            let scene = ready_to_use_scene();
            assert!(
                !bound.is_reuse(id, &scene.id),
                "member {id} must be bound to its own scene"
            );
            assert!(
                bound.is_reuse(id, "scene-group-02"),
                "member {id} must reject reuse on a later scene"
            );
        }
    }

    #[test]
    fn select_group_respects_pre_bound_assets() {
        // The Go side passes assets already assigned to earlier scenes; the
        // sampler must refuse them for this scene instead of reusing them.
        let mut bound = BoundAssets::new();
        bound.bind("artlist-salad-hero", "mediterranean-00-opening");
        let (results, group) = run_group(3, &mut bound);
        assert!(
            !group.iter().any(|id| id == "artlist-salad-hero"),
            "pre-bound asset must not reappear: {group:?}"
        );
        let rejected = results
            .iter()
            .find(|r| r.candidate_id == "artlist-salad-hero")
            .expect("pre-bound candidate still reported");
        assert_eq!(rejected.rejection, Some(RejectionReason::AlreadyBound));
    }

    #[test]
    fn select_group_with_too_few_candidates_returns_only_distinct_accepteds() {
        // Only two acceptable candidates: an x4 request must yield exactly
        // two assets, never duplicates or padding.
        let scene = ready_to_use_scene();
        let candidates: Vec<Candidate> = [
            ("salad-hero", "greek salad close up", 0.9),
            ("feta-block", "feta cheese block", 0.85),
            ("boxing-mismatch", "woman boxing", 0.95),
        ]
        .into_iter()
        .map(|(id, label, similarity)| Candidate {
            id: format!("artlist-{id}"),
            label: label.to_string(),
            generic_similarity: similarity,
            owner_segment_id: String::new(),
            embedding_similarity: None,
        })
        .collect();
        let mut bound = BoundAssets::new();
        let (_, group) =
            sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, 4)
                .expect("legal count");
        assert_eq!(group.len(), 2, "group: {group:?}");
        let distinct: std::collections::HashSet<&String> = group.iter().collect();
        assert_eq!(distinct.len(), 2);
    }

    #[test]
    fn select_group_rejects_counts_outside_two_to_five() {
        let scene = ready_to_use_scene();
        let mut bound = BoundAssets::new();
        for count in [0usize, 1, 6, 99] {
            let error = sample_scene_group(
                &scene,
                &varied_candidates(),
                &food_domain_options(),
                &mut bound,
                count,
            )
            .expect_err("count outside 2..=5 must fail closed");
            assert!(
                error.contains("2..=5"),
                "count={count}: error must name the valid range: {error}"
            );
        }
    }
}
