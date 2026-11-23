//! `select_group` contract tests for MultiImage 2.0.

use super::*;

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
    let (_, group) = sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, 4)
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
