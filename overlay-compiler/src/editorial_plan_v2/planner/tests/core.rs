use super::super::request::{
    BudgetsV2, EditorialBulletV2, EditorialManifestV2, EditorialSceneV2, EntityTimelineV2, MediaV2,
    SceneEntityTimelineV2, TimingV2, VoiceoverV2, WordTimingV2,
};

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
    let clipped = clip_items_to_scene_bounds(vec![item], &request, &mut Vec::new());
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
    // The ceiling is a count, not a kind filter: one image reaches the plan.
    let images = compiled
        .plan
        .items
        .iter()
        .filter(|item| item.kind == "image" || item.kind == "entity_image")
        .count();
    assert_eq!(1, images);
    // Within the ceiling a verified entity portrait outranks an interchangeable
    // scene still, so the portrait is the one that survives and the generic
    // image is the one displaced.
    assert!(compiled
        .plan
        .items
        .iter()
        .any(|item| item.kind == "entity_image"));
    assert!(!compiled.plan.items.iter().any(|item| item.kind == "image"));
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
fn date_and_metric_cards_interleave_all_three_families() {
    let build = |seed: &str, plan_id: &str| {
        let mut request = request();
        request.plan.motion_seed = seed.to_string();
        request.plan.plan_id = plan_id.to_string();
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
                    {"text": "13 agosto 2007", "type": "DATE", "start_us": 0, "duration_us": 400000},
                    {"text": "4 luglio 2010", "type": "DATE", "start_us": 7000000, "duration_us": 400000},
                    {"text": "2 giugno 2020", "type": "DATE", "start_us": 14000000, "duration_us": 400000},
                    {"text": "42", "type": "NUMBER", "start_us": 2000000, "duration_us": 300000},
                    {"text": "67", "type": "NUMBER", "start_us": 4000000, "duration_us": 300000},
                    {"text": "91", "type": "NUMBER", "start_us": 9000000, "duration_us": 300000}
                ]
            }))
            .expect("annotations"),
        );
        request.budgets.as_mut().expect("budgets").max_per_scene = 20;
        let compiled = compile_plan(&request).expect("plan");
        let motions = |template: &str| -> Vec<String> {
            compiled
                .plan
                .items
                .iter()
                .filter(|item| item.template_id == template)
                .map(|item| item.motion_id.clone())
                .collect()
        };
        (motions("TIMELINE_DATE_CARD"), motions("METRIC_STAT_CARD"))
    };

    let (dates, metrics) = build("seed-1", "plan-1");
    assert!(dates.len() >= 3, "expected date cards, got {dates:?}");
    assert!(metrics.len() >= 3, "expected metric cards, got {metrics:?}");
    assert!(
        dates.iter().all(|motion| super::DATE_MOTIONS.contains(&motion.as_str())),
        "date motion outside the certified pool: {dates:?}"
    );
    assert!(
        metrics
            .iter()
            .all(|motion| super::METRIC_MOTIONS.contains(&motion.as_str())),
        "metric motion outside the certified pool: {metrics:?}"
    );
    assert!(
        dates
            .iter()
            .any(|motion| motion.starts_with("date_typewriter_")),
        "a three-date run must reach the typewriter family: {dates:?}"
    );
    assert!(
        metrics
            .iter()
            .any(|motion| motion.starts_with("metric_typewriter_")),
        "a three-metric run must reach the typewriter family: {metrics:?}"
    );
    assert_eq!(
        dates.iter().collect::<std::collections::BTreeSet<_>>().len(),
        dates.len(),
        "date cards must not repeat a gesture: {dates:?}"
    );
    assert_eq!(
        metrics
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        metrics.len(),
        "metric cards must not repeat a gesture: {metrics:?}"
    );

    let (other_dates, other_metrics) = build("seed-2", "plan-1");
    assert_ne!(
        (dates.clone(), metrics.clone()),
        (other_dates, other_metrics),
        "a different run seed must rotate the numeric entrances"
    );
    let (third_dates, third_metrics) = build("seed-1", "plan-3");
    assert_ne!(
        (dates, metrics),
        (third_dates, third_metrics),
        "a shared motion_seed must not freeze the selection across plans"
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

#[test]
fn text_entity_card_wins_the_per_scene_ceiling_over_a_number() {
    // r18: scene-01 carried 12 admitted number/phrase items, so the three
    // text entity cards appended afterwards all failed the per-scene budget
    // and the named person disappeared from the plan. Admission ranks the
    // named card above a date, so the card takes the slot and the date loses,
    // in the same single pass every other lane goes through.
    let mut request = request();
    request.budgets.as_mut().expect("budgets").max_per_scene = 2;
    request
        .media
        .as_mut()
        .expect("media")
        .verified_entity_assets
        .clear();
    let validated = super::ValidatedRequest::new(&request).expect("validated");
    let mut candidates = vec![
        overlay_item("number-1", "number", "DATE", 100_000, 400_000),
        overlay_item("number-2", "number", "DATE", 600_000, 400_000),
    ];
    let cards = super::build_entity_card_candidates(&request, &mut Vec::new()).expect("cards");
    assert_eq!(1, cards.len());
    candidates.extend(cards);
    let intake = super::new_intake(&validated, &candidates);
    let (items, intake) = super::admit(&validated, candidates, intake, &mut Vec::new());

    let kinds: Vec<&str> = items.iter().map(|item| item.kind.as_str()).collect();
    assert_eq!(2, items.len(), "{kinds:?}");
    assert!(kinds.contains(&"entity_card"), "{kinds:?}");
    assert_eq!(1, kinds.iter().filter(|kind| **kind == "number").count());
    assert_eq!(Some(&1), intake.dropped_by_reason.get("per_scene"));
}

/// An `images: disabled` scene keeps the named identity visible as a text card.
/// Before OVL-01 the post-plan promotion checked the toggle; with the portrait
/// now attached in `append_entity_cards`, the same rule must run there or the
/// card would become an `entity_image` that admission drops, losing the person
/// entirely.
#[test]
fn images_disabled_keeps_the_entity_card_textual() {
    let mut request = request();
    request.scenes[0]
        .overlay_categories
        .insert("images".to_string(), "disabled".to_string());
    let compiled = compile_plan(&request).expect("plan");
    assert!(
        compiled
            .plan
            .items
            .iter()
            .all(|item| item.kind != "entity_image" && item.kind != "image_chain"),
        "no image survives an images:disabled scene: {:?}",
        compiled
            .plan
            .items
            .iter()
            .map(|item| item.kind.clone())
            .collect::<Vec<_>>()
    );
    let card = compiled
        .plan
        .items
        .iter()
        .find(|item| item.entity_id == "person-tim")
        .expect("the person card stays in the plan");
    assert_eq!("entity_card", card.kind);
    assert!(card.asset_refs.is_empty(), "{card:?}");
    assert_eq!("Tim Cook", card.text);
}

const DENSE_SCENE_COUNT: usize = 6;
const DENSE_BULLETS_PER_SCENE: usize = 8;
const DENSE_SCENE_US: i64 = 20_000_000;

/// Six timed scenes with eight grounded editorial bullets each. It is the
/// smallest request that exercises the full candidate → admission → chain
/// pipeline at the Augias density target, without shipping a live payload in
/// the unit suite.
fn dense_request() -> EditorialPlanV2Request {
    let mut request = request();
    let mut scenes = Vec::new();
    let mut editorial_scenes = Vec::new();
    let mut entity_scenes = Vec::new();
    let mut windows = std::collections::BTreeMap::new();

    for scene_index in 0..DENSE_SCENE_COUNT {
        let scene_id = format!("scene-{}", scene_index + 1);
        let start_us = scene_index as i64 * DENSE_SCENE_US;
        let mut words = Vec::new();
        let mut bullets = Vec::new();
        let mut clauses = Vec::new();
        let mut word_index = 0_i64;

        for bullet_index in 0..DENSE_BULLETS_PER_SCENE {
            let clause = format!("clausola numero {} del testo", bullet_index + 1);
            let mut word_start = 200_000 + bullet_index as i64 * 1_800_000;
            for token in clause.split_whitespace() {
                words.push(WordTimingV2 {
                    index: word_index,
                    text: token.to_string(),
                    start_us: word_start,
                    end_us: word_start + 250_000,
                });
                word_index += 1;
                word_start += 250_000;
            }
            bullets.push(EditorialBulletV2 {
                sentence_start: bullet_index as i64,
                sentence_end: bullet_index as i64 + 1,
                text: clause.clone(),
            });
            clauses.push(clause);
        }

        let mut scene = request.scenes[0].clone();
        scene.id = scene_id.clone();
        scene.text = format!("{}. ", clauses.join(". ")).trim_end().to_string();
        scene.duration_ms = DENSE_SCENE_US / 1_000;
        scene.duration_us = DENSE_SCENE_US;
        scene.timeline_start_us = start_us;
        scene.index = scene_index as i64;
        scene.voiceover = Some(VoiceoverV2 {
            timing: Some(TimingV2 {
                duration_us: DENSE_SCENE_US,
                words,
            }),
        });
        scene.annotations = None;
        scene.raw = None;
        scenes.push(scene);

        editorial_scenes.push(EditorialSceneV2 {
            scene_id: scene_id.clone(),
            title: None,
            title_status: "missing".to_string(),
            title_source: String::new(),
            globally_indexed: true,
            bullets,
        });
        entity_scenes.push(SceneEntityTimelineV2 {
            scene_id: scene_id.clone(),
            scene_index: scene_index as i64,
            timeline_start_us: start_us,
            voiceover_asset_id: String::new(),
            entities: Vec::new(),
        });
        windows.insert(scene_id, [start_us, start_us + DENSE_SCENE_US]);
    }

    request.scenes = scenes;
    request.editorial = Some(EditorialManifestV2 {
        schema_version: "editorial.v1".to_string(),
        source_language: "it".to_string(),
        profile_version: "highlights.v1".to_string(),
        scenes: editorial_scenes,
        certified: true,
        fingerprint: "a".repeat(64),
    });
    request.entity_timeline = Some(EntityTimelineV2 {
        version: 1,
        project_id: "project-1".to_string(),
        language: "it".to_string(),
        duration_us: DENSE_SCENE_COUNT as i64 * DENSE_SCENE_US,
        scenes: entity_scenes,
    });
    request.media = Some(MediaV2::default());
    request.budgets = Some(BudgetsV2 {
        phrase_overlays: 60,
        image_overlays: 0,
        map_overlays: 0,
        max_per_scene: DENSE_BULLETS_PER_SCENE as i64,
        short_phrase: 0,
        long_phrase: 0,
        bullet: 60,
    });
    request.scene_windows = windows;
    if let Some(seal) = request.seal.as_mut() {
        seal.audio_duration_us = DENSE_SCENE_COUNT as i64 * DENSE_SCENE_US;
    }
    request.canvas.animation_counts = std::collections::BTreeMap::from([
        ("chain_enabled".to_string(), 1),
        ("chain_gap_ms".to_string(), 600),
    ]);
    request
}

/// A chain is one render container but several visual moments. Count each
/// segment, and count a standalone overlay as one; backgrounds are not
/// editorial moments.
fn effective_visual_moments(items: &[PlanItemV2]) -> usize {
    items
        .iter()
        .map(|item| {
            if matches!(
                item.kind.as_str(),
                "phrase_chain" | "image_chain" | "mixed_chain" | "map_chain"
            ) {
                item.params
                    .as_ref()
                    .and_then(|params| params.get("chain_segments"))
                    .and_then(|segments| segments.as_array())
                    .map(Vec::len)
                    .unwrap_or(0)
            } else if item.template_id.eq_ignore_ascii_case("background") {
                0
            } else {
                1
            }
        })
        .sum()
}

#[test]
fn dense_six_scene_plan_reaches_forty_effective_moments() {
    let request = dense_request();
    let compiled = compile_plan(&request).expect("dense plan");
    assert_eq!(DENSE_SCENE_COUNT as i64, compiled.plan.timed_scenes);

    let moments = effective_visual_moments(&compiled.plan.items);
    assert!(
        moments >= 40,
        "expected >= 40 effective visual moments, got {moments} from {} render items: {:?}",
        compiled.plan.items.len(),
        compiled.plan.intake
    );
    assert_eq!(
        48, moments,
        "six scenes with eight bullets each must yield 48 effective moments"
    );
    assert!(
        compiled.plan.intake.candidates >= 40,
        "the dense candidate set must survive into intake: {:?}",
        compiled.plan.intake
    );
    assert!(
        compiled
            .plan
            .items
            .iter()
            .any(|item| item.kind == "phrase_chain"),
        "the density proof must exercise effective moments inside a chain"
    );

    for item in &compiled.plan.items {
        if let Some(segments) = item
            .params
            .as_ref()
            .and_then(|params| params.get("chain_segments"))
            .and_then(|segments| segments.as_array())
        {
            for segment in segments {
                if let Some(text) = segment.get("text").and_then(|text| text.as_str()) {
                    assert!(
                        text.chars().count() <= super::MAX_TEXT_RUNES,
                        "over-budget chain segment: {text:?}"
                    );
                }
            }
        } else if item.kind == "text_phrase" {
            assert!(
                item.text.chars().count() <= super::MAX_TEXT_RUNES,
                "over-budget standalone phrase: {:?}",
                item.text
            );
        }
    }
}

/// An outlet is an organization for identity (`identity.rs` prefixes its
/// canonical id with `org`), so the planner must route it to the same card and
/// image lanes. Before the single fold, `MEDIA_OUTLET` was unknown to
/// `entity_card_kind` and to the recovered-image gate: the entity was
/// extracted, linked and counted as a named subject, yet never planned.
fn media_outlet_request() -> EditorialPlanV2Request {
    let mut request = request();
    {
        let entity = &mut request.entity_timeline.as_mut().expect("timeline").scenes[0].entities[0];
        entity.entity_type = "MEDIA_OUTLET".to_string();
        entity.name = "la Repubblica".to_string();
        entity.entity_id = "outlet-repubblica".to_string();
        entity.canonical_entity_id = "org:la-repubblica".to_string();
    }
    {
        let verified = &mut request
            .media
            .as_mut()
            .expect("media")
            .verified_entity_assets[0];
        verified.entity_id = "outlet-repubblica".to_string();
        verified.canonical_entity_id = "org:la-repubblica".to_string();
    }
    request
}

#[test]
fn a_media_outlet_is_carded_as_an_organization() {
    let mut request = media_outlet_request();
    request
        .media
        .as_mut()
        .expect("media")
        .verified_entity_assets
        .clear();
    let compiled = compile_plan(&request).expect("plan");
    let card = compiled
        .plan
        .items
        .iter()
        .find(|item| item.kind == "organization")
        .expect("organization card");
    assert_eq!("org_default", card.template_id, "{card:?}");
    // A text card carries its label in the body, not in the image caption.
    assert_eq!("la Repubblica", card.text);
    assert!(card.entity_caption.is_empty(), "{card:?}");
    assert_eq!(
        Some("MEDIA_OUTLET"),
        card.params
            .as_ref()
            .and_then(|params| params["entity_type"].as_str())
    );
}

#[test]
fn a_media_outlet_binds_its_verified_asset() {
    let request = media_outlet_request();
    let compiled = compile_plan(&request).expect("plan");
    let image = compiled
        .plan
        .items
        .iter()
        .find(|item| item.kind == "entity_image" && item.entity_caption == "la Repubblica")
        .expect("outlet image");
    assert_eq!("image_popup", image.template_id);
    assert_eq!(1, image.asset_refs.len());
}

#[test]
fn a_validated_request_hands_out_the_required_sections_without_optionals() {
    let request = request();
    let validated = super::ValidatedRequest::new(&request).expect("validated");
    assert_eq!(
        request.budgets.as_ref().expect("budgets").phrase_overlays,
        validated.budgets().phrase_overlays
    );
    assert_eq!(
        request.seal.as_ref().expect("seal").audio_duration_us,
        validated.seal().audio_duration_us
    );
    // Every other field stays reachable through the deref.
    assert_eq!(request.scenes.len(), validated.scenes.len());
}

#[test]
fn a_request_without_budgets_is_rejected_not_panicked() {
    let mut request = request();
    request.budgets = None;
    assert!(matches!(
        super::ValidatedRequest::new(&request),
        Err(super::EditorialPlanError::MissingSection("budgets"))
    ));
}

#[test]
fn a_request_without_seal_is_rejected_not_panicked() {
    let mut request = request();
    request.seal = None;
    assert!(matches!(
        super::ValidatedRequest::new(&request),
        Err(super::EditorialPlanError::MissingSection("seal"))
    ));
}
