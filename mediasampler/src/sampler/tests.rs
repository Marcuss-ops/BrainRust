//! Unit tests for the semantic sampler pipeline.

use super::*;
use crate::types::{Candidate, RejectionReason, SampleOptions, SampleResult, Scene};

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
    let (results, winner) = sample_scene(&scene, &candidates, &food_domain_options(), &mut bound);
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
        let group = sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, 5)
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
        assert!(
            sample_scene_group(&scene, &candidates, &food_domain_options(), &mut bound, bad)
                .is_err()
        );
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
    let (results, winner) = sample_scene(&scene, &[candidate], &food_domain_options(), &mut bound);
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
