/// One 60 s scene with a configurable annotated phrase list. Word timing is
/// present unless the test explicitly asks for a scene without it, which is the
/// real "unanchored in transcript" condition the gap pass must refuse.
fn gap_request(
    duration_us: i64,
    phrases: serde_json::Value,
    budgets: serde_json::Value,
    canvas: serde_json::Value,
    with_word_timing: bool,
) -> EditorialPlanV2Request {
    let mut scene = json!({
        "id": "scene-1",
        "text": "Una scena di prova con frasi ancorate al parlato.",
        "duration_ms": duration_us / 1_000,
        "execution_mode": "body",
        "timeline_start_us": 0,
        "duration_us": duration_us,
        "annotations": { "phrases": phrases, "end_us": duration_us }
    });
    if with_word_timing {
        scene["voiceover"] = json!({
            "timing": {
                "duration_us": 1_000_000,
                "words": [
                    {"index": 0, "text": "Una", "start_us": 0, "end_us": 200_000},
                    {"index": 1, "text": "scena", "start_us": 200_000, "end_us": 600_000},
                    {"index": 2, "text": "di", "start_us": 600_000, "end_us": 1_000_000}
                ]
            }
        });
    }
    serde_json::from_value(json!({
        "operation": "compile_editorial_plan",
        "schema_version": "editorial_plan.v2",
        "plan": {
            "plan_id": "plan-gap", "video_id": "video-gap", "project_id": "project-gap",
            "motion_seed": "seed-gap",
            "width": 1920, "height": 1080, "fps_num": 24, "fps_den": 1
        },
        "language": "it",
        "seal": { "audio_duration_us": duration_us, "output_profile_id": "test" },
        "canvas": canvas,
        "scenes": [scene],
        "editorial": {
            "schema_version": "editorial.v1",
            "source_language": "it",
            "profile_version": "highlights.v1",
            "certified": true,
            "fingerprint": "a".repeat(64),
            "scenes": [{
                "scene_id": "scene-1",
                "title_status": "resolved",
                "title_source": "extractive_keyphrase",
                "globally_indexed": true,
                "bullets": []
            }]
        },
        "media": {},
        "entity_timeline": { "version": 1, "scenes": [] },
        "budgets": budgets,
        "scene_windows": { "scene-1": [0, duration_us] }
    }))
    .expect("gap fixture")
}

fn budgets(phrases: i64) -> serde_json::Value {
    json!({
        "phrase_overlays": phrases,
        "image_overlays": 0,
        "map_overlays": 0,
        "max_per_scene": 10,
        "bullet": 0
    })
}

fn phrase(text: &str, start_us: i64, duration_us: i64, score: f64) -> serde_json::Value {
    json!({
        "text": text,
        "start_us": start_us,
        "duration_us": duration_us,
        "score": score
    })
}

fn phrase_windows(items: &[super::PlanItemV2]) -> Vec<(i64, i64)> {
    items
        .iter()
        .filter(|item| item.kind == "text_phrase")
        .map(|item| item.window_us())
        .collect()
}

/// The primary admission pass is score-ordered, so the second phrase loses the
/// single budget slot even though it is the only copy in the second half of the
/// scene. The gap pass must promote it: it is word-anchored and fits the desert
/// with room for the cooldown on both sides.
#[test]
fn gap_pass_promotes_a_word_anchored_phrase_into_a_desert() {
    let request = gap_request(
        40_000_000,
        json!([
            phrase("la prima frase", 1_000_000, 4_000_000, 0.9),
            phrase("frase nel deserto", 20_000_000, 5_000_000, 0.8)
        ]),
        budgets(1),
        json!({}),
        true,
    );
    let compiled = compile_plan(&request).expect("plan");
    let windows = phrase_windows(&compiled.plan.items);
    assert_eq!(
        vec![(1_000_000, 5_000_000), (20_000_000, 25_000_000)],
        windows,
        "the desert phrase must be promoted, not dropped"
    );
    assert_eq!(1, compiled.plan.intake.gap_promoted);
    let coverage = compiled.plan.coverage.expect("coverage");
    assert_eq!(1, coverage.gap_promotions);
    assert!(
        coverage.max_gap_us < 25_000_000,
        "coverage after pacing: {coverage:?}"
    );
    assert_eq!(0, coverage.gaps_over_threshold);
}

/// A fixed ceiling of 50 on a nine-minute video is unreachable and produces a
/// permanent shortfall warning. `phrase_per_minute` derives the reachable
/// target instead, and an explicit smaller ceiling still caps it.
#[test]
fn phrase_per_minute_sets_a_reachable_target() {
    let request = gap_request(
        540_000_000,
        json!([]),
        json!({
            "phrase_overlays": 50,
            "image_overlays": 0,
            "map_overlays": 0,
            "max_per_scene": 10,
            "bullet": 0
        }),
        json!({ "phrase_per_minute": 4.0 }),
        true,
    );
    let compiled = compile_plan(&request).expect("plan");
    let coverage = compiled.plan.coverage.expect("coverage");
    assert_eq!(36, coverage.phrase_target, "4 phrases x 9 minutes");
    assert_eq!(36, compiled.plan.intake.budget.phrase_overlays);
    let shortfall = compiled.plan.phrase_shortfall.expect("shortfall");
    assert_eq!(36, shortfall.requested);
    assert_eq!(0, shortfall.materialized);

    let capped = gap_request(
        540_000_000,
        json!([]),
        json!({
            "phrase_overlays": 12,
            "image_overlays": 0,
            "map_overlays": 0,
            "max_per_scene": 10,
            "bullet": 0
        }),
        json!({ "phrase_per_minute": 4.0 }),
        true,
    );
    let compiled = compile_plan(&capped).expect("plan");
    assert_eq!(
        12,
        compiled.plan.coverage.expect("coverage").phrase_target,
        "an explicit caller ceiling still wins when it is smaller"
    );
}

/// Every rejected phrase carries exactly one terminal reason, so a shortfall
/// warning can say WHY a card is missing instead of only how many.
#[test]
fn shortfall_reports_one_reason_per_rejected_phrase() {
    let request = gap_request(
        10_000_000,
        json!([
            phrase("la prima frase", 500_000, 4_000_000, 0.9),
            phrase("la seconda frase", 5_000_000, 4_000_000, 0.8)
        ]),
        budgets(1),
        json!({}),
        true,
    );
    let compiled = compile_plan(&request).expect("plan");
    let shortfall = compiled.plan.phrase_shortfall.expect("shortfall");
    assert_eq!(1, shortfall.dropped, "{shortfall:?}");
    assert_eq!(Some(&1), shortfall.by_reason.get("budget_exhausted"));
    assert_eq!("la seconda frase", shortfall.drops[0].text);
    assert_eq!("budget_exhausted", shortfall.drops[0].reason);
}

/// A scene without certified word timing cannot anchor a phrase. The pass must
/// report it as `unanchored_in_transcript` and leave the desert alone rather
/// than placing copy at a guessed instant.
#[test]
fn unanchored_phrase_is_reported_and_not_promoted() {
    let request = gap_request(
        20_000_000,
        json!([
            phrase("la prima frase", 1_000_000, 4_000_000, 0.9),
            phrase("frase senza timing", 10_000_000, 5_000_000, 0.8)
        ]),
        budgets(1),
        json!({}),
        false,
    );
    let compiled = compile_plan(&request).expect("plan");
    assert_eq!(
        vec![(1_000_000, 5_000_000)],
        phrase_windows(&compiled.plan.items),
        "an unanchored phrase must never be promoted"
    );
    assert_eq!(0, compiled.plan.intake.gap_promoted);
    let shortfall = compiled.plan.phrase_shortfall.expect("shortfall");
    assert_eq!(
        Some(&1),
        shortfall.by_reason.get("unanchored_in_transcript"),
        "{shortfall:?}"
    );
    assert_eq!("unanchored_in_transcript", shortfall.drops[0].reason);
}

/// Pacing inserts cards, it never stacks them: every pair of admitted items
/// must remain disjoint.
#[test]
fn gap_pass_never_creates_overlaps() {
    let request = gap_request(
        60_000_000,
        json!([
            phrase("prima idea qui", 1_000_000, 4_000_000, 0.9),
            phrase("seconda idea qui", 25_000_000, 5_000_000, 0.8),
            phrase("terza idea qui", 40_000_000, 5_000_000, 0.7)
        ]),
        budgets(1),
        json!({}),
        true,
    );
    let compiled = compile_plan(&request).expect("plan");
    let items = &compiled.plan.items;
    assert!(
        items.len() >= 2,
        "at least one desert phrase must be promoted: {items:?}"
    );
    for (index, left) in items.iter().enumerate() {
        let (left_start, left_end) = left.window_us();
        for right in items.iter().skip(index + 1) {
            let (right_start, right_end) = right.window_us();
            assert!(
                left_end <= right_start || right_end <= left_start,
                "{:?} [{left_start},{left_end}) overlaps {:?} [{right_start},{right_end})",
                left.id,
                right.id
            );
        }
    }
}

/// A category the caller disabled stays disabled: the gap pass is a pacing
/// rule, not a bypass for an editorial switch.
#[test]
fn gap_pass_never_promotes_a_disabled_category() {
    let request = gap_request(
        40_000_000,
        json!([
            phrase("la prima frase", 1_000_000, 4_000_000, 0.9),
            phrase("frase nel deserto", 20_000_000, 5_000_000, 0.8)
        ]),
        budgets(1),
        json!({ "overlay_categories": { "phrases": "disabled" } }),
        true,
    );
    let compiled = compile_plan(&request).expect("plan");
    assert!(phrase_windows(&compiled.plan.items).is_empty());
    assert_eq!(0, compiled.plan.intake.gap_promoted);
}

/// The promotion must respect the owning scene end even when the plan continues
/// with another scene: a card can never spill into its neighbour.
#[test]
fn promoted_cards_stay_inside_their_scene() {
    let request = gap_request(
        60_000_000,
        json!([
            phrase("la prima idea", 1_000_000, 4_000_000, 0.9),
            phrase("nel deserto ora", 20_000_000, 5_000_000, 0.8)
        ]),
        budgets(1),
        json!({}),
        true,
    );
    let compiled = compile_plan(&request).expect("plan");
    for item in &compiled.plan.items {
        let (start_us, end_us) = item.window_us();
        assert!(start_us >= 0, "{item:?}");
        assert!(end_us <= 60_000_000, "{item:?}");
    }
}
