#[test]
fn compiles_title_bullet_and_verified_entity_image() {
    let request = request();
    let compiled = compile_plan(&request).expect("plan");
    compiled.validate_for(&request).expect("valid plan");
    // The two-word title is filtered by the same three-word rule Go
    // applies to every phrase lane; the bullet and the entity image remain.
    assert_eq!(2, compiled.plan.items.len());
    assert!(compiled
        .plan
        .items
        .iter()
        .any(|item| item.kind == "text_phrase"));
    let image = compiled
        .plan
        .items
        .iter()
        .find(|item| item.kind == "entity_image")
        .expect("entity image");
    assert_eq!("Tim Cook", image.entity_caption);
    assert_eq!(1, image.asset_refs.len());
    let params = image.params.as_ref().expect("entity params");
    assert_eq!(820.0, params["width"].as_f64().unwrap());
    assert_eq!(820.0, params["height"].as_f64().unwrap());
    assert_eq!(0.0, params["position_x"].as_f64().unwrap());
    assert_eq!(0.0, params["position_y"].as_f64().unwrap());
}

#[test]
fn seal_clips_items_and_chain_segments_to_the_scene_end() {
    let request = request();
    let item = PlanItemV2 {
        id: "long-phrase".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "text_phrase".to_string(),
        start_us: 0,
        duration_us: 3_000_000,
        start_ms: 0,
        end_ms: 3_000,
        params: Some(serde_json::json!({
            "chain_segments": [{"start_us": 0, "duration_us": 3_000_000}]
        })),
        ..PlanItemV2::default()
    };
    let clipped = clip_items_to_scene_bounds(vec![item], &request);
    assert_eq!(1, clipped.len());
    assert_eq!(2_000_000, clipped[0].duration_us);
    assert_eq!(2_000, clipped[0].end_ms);
    let segment = &clipped[0].params.as_ref().expect("params")["chain_segments"][0];
    assert_eq!(2_000_000, segment["duration_us"]);
}

#[test]
fn later_overlapping_chain_is_dropped() {
    let first = chain_item(
        "first",
        0,
        4_000_000,
        &[(0, 2_000_000), (2_400_000, 1_600_000)],
    );
    let second = chain_item(
        "second",
        3_000_000,
        4_000_000,
        &[(3_000_000, 2_000_000), (5_400_000, 1_600_000)],
    );
    let resolved = resolve_chain_overlaps(vec![second, first]);
    let ids: Vec<&str> = resolved.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(vec!["first"], ids);
}

#[test]
fn seal_clears_derived_render_keys() {
    let request = request();
    let compiled = compile_plan(&request).expect("plan");
    assert!(
        compiled
            .plan
            .items
            .iter()
            .all(|item| item.render_key.is_empty()),
        "sealing must leave render-key derivation to Go"
    );
}

#[test]
fn zero_image_budget_keeps_the_certified_default() {
    let mut request = request();
    request.budgets.as_mut().expect("budgets").image_overlays = 0;
    let compiled = compile_plan(&request).expect("plan");
    assert!(compiled
        .plan
        .items
        .iter()
        .any(|item| item.kind == "entity_image"));
}

#[test]
fn explicit_image_budget_is_a_run_ceiling() {
    let mut request = request();
    request.scenes[0].annotations = Some(
        serde_json::from_value(serde_json::json!({
            "images": [{
                "asset_id": "asset-scene",
                "url": "https://example.test/scene.jpg",
                "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "media_type": "image/jpeg",
                "start_us": 300000, "duration_us": 500000
            }]
        }))
        .expect("annotations"),
    );
    request.budgets.as_mut().expect("budgets").image_overlays = 1;
    let compiled = compile_plan(&request).expect("plan");
    let images = compiled
        .plan
        .items
        .iter()
        .filter(|item| item.kind == "image" || item.kind == "entity_image")
        .count();
    assert_eq!(1, images);
    assert!(!compiled
        .plan
        .items
        .iter()
        .any(|item| item.kind == "entity_image"));
    assert_eq!(1, compiled.plan.intake.dropped_by_reason["budget"]);
}

#[test]
fn routes_dates_and_metrics_to_certified_cards() {
    let mut request = request();
    request.scenes[0].annotations = Some(
        serde_json::from_value(serde_json::json!({
            "numbers": [
                {"text": "13 agosto", "type": "DATE", "start_us": 0, "duration_us": 500000},
                {"text": "42", "type": "NUMBER", "start_us": 1000000, "duration_us": 500000}
            ]
        }))
        .expect("annotations"),
    );
    request.budgets.as_mut().expect("budgets").max_per_scene = 10;
    let compiled = compile_plan(&request).expect("plan");
    let date = compiled
        .plan
        .items
        .iter()
        .find(|item| item.template_id == "TIMELINE_DATE_CARD")
        .expect("date card");
    assert_eq!(2_000_000, date.duration_us, "date is capped by the scene");
    assert!(super::DATE_MOTIONS.contains(&date.motion_id.as_str()));
    let metric = compiled
        .plan
        .items
        .iter()
        .find(|item| item.template_id == "METRIC_STAT_CARD")
        .expect("metric card");
    assert!(super::METRIC_MOTIONS.contains(&metric.motion_id.as_str()));
}

#[test]
fn date_motion_family_filters_the_date_pool() {
    for (selector, expected_prefix) in [
        ("classic", None),
        ("didone", Some("date_didone_")),
        ("typewriter", Some("date_typewriter_")),
    ] {
        let mut request = request();
        request.canvas.date_motion_family = selector.to_string();
        request.scenes[0].annotations = Some(
            serde_json::from_value(serde_json::json!({
                "numbers": [
                    {"text": "13 agosto", "type": "DATE", "start_us": 0, "duration_us": 500000}
                ]
            }))
            .expect("annotations"),
        );
        request.budgets.as_mut().expect("budgets").max_per_scene = 10;
        let compiled = compile_plan(&request).expect("plan");
        let date = compiled
            .plan
            .items
            .iter()
            .find(|item| item.template_id == "TIMELINE_DATE_CARD")
            .expect("date card");
        match expected_prefix {
            Some(prefix) => assert!(
                date.motion_id.starts_with(prefix),
                "{selector} picked {}",
                date.motion_id
            ),
            None => assert!(
                !date.motion_id.starts_with("date_didone_")
                    && !date.motion_id.starts_with("date_typewriter_"),
                "classic picked {}",
                date.motion_id
            ),
        }
    }
}

#[test]
fn repeated_dates_and_metrics_rotate_their_motion_pools() {
    let mut request = request();
    request.canvas.date_motion_family = "typewriter".to_string();
    // A date card needs at least two readable seconds and stays on screen
    // for five, so the fixture's 2 s scene can only hold one. Stretch the
    // scene so three dates and three metrics can share it.
    let scene_duration_us = 20_000_000_i64;
    request.seal.as_mut().expect("seal").audio_duration_us = scene_duration_us;
    request.scenes[0].duration_us = scene_duration_us;
    request.scenes[0].duration_ms = scene_duration_us / 1000;
    request
        .scene_windows
        .insert("scene-1".to_string(), [0, scene_duration_us]);
    request.scenes[0].annotations = Some(
        serde_json::from_value(serde_json::json!({
            "numbers": [
                {"text": "13 agosto", "type": "DATE", "start_us": 0, "duration_us": 400000},
                {"text": "4 luglio", "type": "DATE", "start_us": 7000000, "duration_us": 400000},
                {"text": "2 giugno", "type": "DATE", "start_us": 14000000, "duration_us": 400000},
                {"text": "42", "type": "NUMBER", "start_us": 2000000, "duration_us": 300000},
                {"text": "67", "type": "NUMBER", "start_us": 4000000, "duration_us": 300000},
                {"text": "91", "type": "NUMBER", "start_us": 9000000, "duration_us": 300000}
            ]
        }))
        .expect("annotations"),
    );
    request.budgets.as_mut().expect("budgets").max_per_scene = 20;
    let compiled = compile_plan(&request).expect("plan");

    let date_motions: Vec<&str> = compiled
        .plan
        .items
        .iter()
        .filter(|item| item.template_id == "TIMELINE_DATE_CARD")
        .map(|item| item.motion_id.as_str())
        .collect();
    assert!(!date_motions.is_empty(), "expected date cards");
    assert!(
        date_motions
            .iter()
            .all(|motion| motion.starts_with("date_typewriter_")),
        "date family leaked: {date_motions:?}"
    );
    let distinct_dates: std::collections::BTreeSet<&str> = date_motions.iter().copied().collect();
    assert!(
        distinct_dates.len() > 1,
        "every date reused one motion: {date_motions:?}"
    );

    let metric_motions: Vec<&str> = compiled
        .plan
        .items
        .iter()
        .filter(|item| item.template_id == "METRIC_STAT_CARD")
        .map(|item| item.motion_id.as_str())
        .collect();
    assert!(!metric_motions.is_empty(), "expected metric cards");
    assert!(
        metric_motions
            .iter()
            .all(|motion| super::METRIC_MOTIONS.contains(motion)),
        "metric motion outside the certified pool: {metric_motions:?}"
    );
    let distinct_metrics: std::collections::BTreeSet<&str> =
        metric_motions.iter().copied().collect();
    assert!(
        distinct_metrics.len() > 1,
        "every metric reused one motion: {metric_motions:?}"
    );
}

#[test]
fn seeded_number_motion_matches_the_go_sampler() {
    assert_eq!(
        "s1-number-42-per-cento",
        super::go_item_id("s1", "number", "42 per cento")
    );
    // The Go sampler hashes the seed to lowercase hex and reads the first
    // eight hex characters as a big-endian integer. This is the exact
    // fixture decision the Go oracle produces for this semantic id.
    assert_eq!(
        Some("metric_debt_flip".to_string()),
        super::seeded_preset(
            "charts-fixture",
            "s1",
            "s1-number-42-per-cento",
            "metric_presentation",
            super::METRIC_MOTIONS,
        )
    );
}

#[test]
fn clip_start_light_leaks_skip_freeze_and_respect_the_seal() {
    let mut request = request();
    request.canvas.light_leak_on_clip_start = true;
    request.clips = vec![
        super::super::request::ClipV2 {
            scene_id: "scene-1".to_string(),
            asset_id: "asset-real".to_string(),
            index: 0,
            start_us: 0,
            duration_us: 3_000_000,
            freeze: false,
        },
        super::super::request::ClipV2 {
            scene_id: "scene-1".to_string(),
            asset_id: "asset-freeze".to_string(),
            index: 1,
            start_us: 500_000,
            duration_us: 1_000_000,
            freeze: true,
        },
        super::super::request::ClipV2 {
            scene_id: "scene-1".to_string(),
            asset_id: "asset-past-seal".to_string(),
            index: 2,
            start_us: 2_500_000,
            duration_us: 1_000_000,
            freeze: false,
        },
    ];
    let compiled = compile_plan(&request).expect("plan");
    let leaks = compiled
        .plan
        .items
        .iter()
        .filter(|item| item.kind == "light_leak")
        .collect::<Vec<_>>();
    assert_eq!(1, leaks.len(), "only the real in-seal clip gets a leak");
    assert_eq!(1_200_000, leaks[0].duration_us);
    assert!(leaks[0].motion_id.starts_with("lightleak_"));
}

#[test]
fn recap_window_fills_three_cards_when_it_has_room() {
    let mut request = request();
    request.scenes[0].duration_ms = 12_000;
    request.scenes[0].duration_us = 12_000_000;
    request.recap_briefs = (0..3)
        .map(|index| super::super::request::RecapBriefV2 {
            id: format!("brief-{index}"),
            title: format!("Capitolo {index}"),
            score: 1.0 - index as f64 * 0.1,
            start_us: index as i64 * 1_000_000,
            end_us: (index as i64 + 1) * 1_000_000,
        })
        .chain(std::iter::once(super::super::request::RecapBriefV2 {
            id: "scene-1".to_string(),
            title: "Placement".to_string(),
            score: 0.5,
            start_us: 0,
            end_us: 12_000_000,
        }))
        .collect();
    let recap = super::build_recap_candidates(&request).expect("recap candidates");
    assert_eq!(3, recap.items.len());
    assert_eq!(1, recap.placements);
    assert!(recap
        .items
        .iter()
        .all(|item| item.params.as_ref().unwrap()["recap"] == true));
}

#[test]
fn names_items_with_motion_subject_and_category() {
    let mut items = vec![
        super::PlanItemV2 {
            motion_id: "date_fade_rise".to_string(),
            text: "13 agosto".to_string(),
            kind: "number".to_string(),
            ..Default::default()
        },
        super::PlanItemV2 {
            motion_id: "date_fade_rise".to_string(),
            text: "13 agosto".to_string(),
            kind: "number".to_string(),
            ..Default::default()
        },
    ];
    super::name_items_for_review(&mut items);
    assert_eq!("date-fade-rise__13-agosto__date_number__01", items[0].id);
    assert_eq!("date-fade-rise__13-agosto__date_number__02", items[1].id);
}

#[test]
fn maps_only_drops_every_non_map_candidate() {
    let mut request = request();
    request.canvas.maps_only = true;
    let compiled = compile_plan(&request).expect("plan");
    assert!(compiled.plan.items.is_empty());
    assert_eq!(2, compiled.plan.intake.dropped_by_reason["maps_only"]);
}

#[test]
fn chart_placements_count_strict_comparable_values() {
    let mut request = request();
    request.scenes[0].annotations = Some(
        serde_json::from_value(serde_json::json!({
            "numbers": [
                {"text": "42", "type": "NUMBER", "start_us": 0, "duration_us": 1000000},
                {"text": "18%", "type": "NUMBER", "start_us": 1000000, "duration_us": 1000000}
            ]
        }))
        .expect("annotations"),
    );
    assert_eq!(1, super::count_chart_placements(&request));
}

#[test]
fn chart_placements_reject_word_values() {
    let mut request = request();
    request.scenes[0].annotations = Some(
            serde_json::from_value(serde_json::json!({
                "numbers": [
                    {"text": "42 per cento", "type": "NUMBER", "start_us": 0, "duration_us": 1000000},
                    {"text": "18 per cento", "type": "NUMBER", "start_us": 1000000, "duration_us": 1000000}
                ]
            }))
            .expect("annotations"),
        );
    assert_eq!(0, super::count_chart_placements(&request));
}
