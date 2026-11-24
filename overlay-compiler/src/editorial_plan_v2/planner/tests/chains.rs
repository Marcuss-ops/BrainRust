#[test]
fn chain_reserves_its_window_from_standalone_overlays() {
    let chain = chain_item("chain", 1_647_000, 8_300_000, &[(1_647_000, 2_500_000)]);
    let overlapped = overlay_item("photo", "image", "IMAGE_OVERLAY", 5_000_000, 5_000_000);
    let after = overlay_item("later", "image", "IMAGE_OVERLAY", 10_000_000, 2_000_000);
    let resolved = resolve_chain_overlaps(vec![chain, overlapped, after]);
    let ids: Vec<&str> = resolved.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(vec!["chain", "later"], ids);
}

#[test]
fn chain_keeps_its_background_layer() {
    let chain = chain_item(
        "chain",
        0,
        5_000_000,
        &[(0, 2_500_000), (2_900_000, 2_100_000)],
    );
    let background = overlay_item("bg", "image", "background", 0, 5_000_000);
    let resolved = resolve_chain_overlaps(vec![chain, background]);
    let ids: Vec<&str> = resolved.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(vec!["chain", "bg"], ids);
}

#[test]
fn chain_drops_a_tail_below_the_readable_minimum() {
    let request = request();
    let chain = chain_item(
        "chain",
        0,
        3_000_000,
        &[
            (0, 1_000_000),
            (1_000_000, 1_000_000),
            (2_000_000, 1_000_000),
        ],
    );
    let clipped = clip_items_to_scene_bounds(vec![chain], &request, &mut Vec::new());
    assert_eq!(1, clipped.len());
    let item = &clipped[0];
    assert_eq!((0, 2_000_000), item.window_us());
    let params = item.params.as_ref().expect("params");
    assert_eq!(2, params["chain_count"].as_i64().unwrap());
    assert_eq!(2, params["chain_segments"].as_array().unwrap().len());
    assert_eq!(2, params["chain_motions"].as_array().unwrap().len());
}

#[test]
fn chain_without_two_readable_segments_is_dropped() {
    let request = request();
    let chain = chain_item(
        "chain",
        0,
        2_000_000,
        &[(0, 1_500_000), (1_500_000, 500_000)],
    );
    let clipped = clip_items_to_scene_bounds(vec![chain], &request, &mut Vec::new());
    assert!(clipped.is_empty());
}

#[test]
fn general_chain_is_opt_in_and_chain_disabled_wins() {
    let items = vec![
        chain_phrase("a", "alpha", 100_000, 500_000, "motion_a"),
        chain_phrase("b", "beta", 700_000, 500_000, "motion_b"),
    ];
    let ends = chain_ends(10_000_000);
    let plain = super::coalesce_chained_overlays(items.clone(), &chain_request(&[]), &ends);
    assert_eq!(2, plain.len());
    assert!(plain.iter().all(|item| item.kind == "text_phrase"));

    let disabled = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1), ("chain_disabled", 1)]),
        &ends,
    );
    assert_eq!(2, disabled.len());
    assert!(disabled.iter().all(|item| item.kind == "text_phrase"));
}

#[test]
fn general_chain_fuses_nearby_phrases_with_sequential_segments() {
    let mut first = chain_phrase("a", "alpha", 100_000, 500_000, "motion_a");
    first.group_id = "group-a".to_string();
    first.subgroup_id = "child-a".to_string();
    first.motion_params = Some(serde_json::json!({"enter_frames": 17, "vendor": "payload-owned"}));
    first.params = Some(serde_json::json!({"font_family": "Inter", "font_size_px": 72}));
    first.style = Some(serde_json::json!({"font_family": "Inter"}));
    let second = chain_phrase("b", "beta", 700_000, 500_000, "motion_b");
    let fused = super::coalesce_chained_overlays(
        vec![first, second],
        &chain_request(&[("chain_gap_ms", 600)]),
        &chain_ends(10_000_000),
    );
    assert_eq!(1, fused.len());
    let chain = &fused[0];
    assert_eq!("phrase_chain", chain.kind);
    assert_eq!("PHRASE_CHAIN", chain.template_id);
    assert!(chain.preset_id.is_empty());
    assert!(chain.motion_id.is_empty());
    assert_eq!((100_000, 1_500_000), chain.window_us());
    let params = chain.params.as_ref().expect("chain params");
    assert_eq!(2, params["chain_count"].as_i64().unwrap());
    assert_eq!("fly_under_fade", params["chain_camera"].as_str().unwrap());
    assert_eq!(400_000, params["chain_gap_us"].as_i64().unwrap());
    assert!(params.get("font_family").is_none());
    let segments = params["chain_segments"].as_array().expect("segments");
    assert_eq!(2, segments.len());
    assert_eq!(100_000, segments[0]["start_us"].as_i64().unwrap());
    assert_eq!(1_000_000, segments[1]["start_us"].as_i64().unwrap());
    assert_eq!("motion_a", segments[0]["motion_id"].as_str().unwrap());
    assert_eq!("motion_b", segments[1]["motion_id"].as_str().unwrap());
    assert_eq!("group-a", segments[0]["group_id"].as_str().unwrap());
    assert_eq!(
        17,
        segments[0]["motion_params"]["enter_frames"]
            .as_i64()
            .unwrap()
    );
    assert_eq!(
        "Inter",
        segments[0]["params"]["font_family"].as_str().unwrap()
    );
    assert_eq!(
        "Inter",
        segments[0]["style"]["font_family"].as_str().unwrap()
    );
}

#[test]
fn general_chain_handoff_can_be_removed() {
    let items = vec![
        chain_phrase("a", "alpha", 100_000, 500_000, "motion_a"),
        chain_phrase("b", "beta", 700_000, 500_000, "motion_b"),
    ];
    let fused = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1), ("chain_handoff_ms", 0)]),
        &chain_ends(10_000_000),
    );
    let chain = &fused[0];
    let params = chain.params.as_ref().expect("chain params");
    assert_eq!(0, params["chain_gap_us"].as_i64().unwrap());
    let segments = params["chain_segments"].as_array().expect("segments");
    assert_eq!(100_000, segments[0]["start_us"].as_i64().unwrap());
    assert_eq!(600_000, segments[1]["start_us"].as_i64().unwrap());
    assert_eq!((100_000, 1_100_000), chain.window_us());
}

#[test]
fn general_chain_handoff_is_configurable() {
    let items = vec![
        chain_phrase("a", "alpha", 100_000, 500_000, "motion_a"),
        chain_phrase("b", "beta", 700_000, 500_000, "motion_b"),
    ];
    let fused = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1), ("chain_handoff_ms", 1_500)]),
        &chain_ends(10_000_000),
    );
    let chain = &fused[0];
    let params = chain.params.as_ref().expect("chain params");
    assert_eq!(1_500_000, params["chain_gap_us"].as_i64().unwrap());
    let segments = params["chain_segments"].as_array().expect("segments");
    assert_eq!(2_100_000, segments[1]["start_us"].as_i64().unwrap());
    assert_eq!((100_000, 2_600_000), chain.window_us());
}

#[test]
fn general_chain_fuses_captioned_entity_images_into_a_sequence() {
    let mut first = chain_image("ada", 100_000, 3_000_000, "image_focus_reveal");
    first.kind = "entity_image".to_string();
    first.template_id = "image_popup".to_string();
    first.entity_id = "ent_ada".to_string();
    first.entity_caption = "Ada Lovelace".to_string();
    first.caption_layout = "below".to_string();
    first.entity_style_id = "premium_below_random_v1".to_string();
    first.params = Some(serde_json::json!({
        "position_x": 0.0, "position_y": 0.0, "width": 820.0, "height": 820.0
    }));
    let mut second = chain_image("grace", 3_400_000, 3_000_000, "image_fade_reveal");
    second.kind = "entity_image".to_string();
    second.template_id = "image_popup".to_string();
    second.entity_id = "ent_grace".to_string();
    second.entity_caption = "Grace Hopper".to_string();
    second.caption_layout = "below".to_string();
    second.entity_style_id = "premium_below_random_v1".to_string();
    second.params = Some(serde_json::json!({
        "position_x": 0.0, "position_y": 0.0, "width": 820.0, "height": 820.0
    }));
    let plain = chain_image("photo", 8_000_000, 2_000_000, "image_scale_reveal");

    let fused = super::coalesce_chained_overlays(
        vec![first, second, plain],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(20_000_000),
    );
    assert_eq!(
        2,
        fused.len(),
        "cards fuse, the plain photo stays standalone"
    );
    let chain = fused
        .iter()
        .find(|item| item.kind == "image_chain")
        .expect("captioned entity cards must form an image_chain");
    let segments = chain.params.as_ref().unwrap()["chain_segments"]
        .as_array()
        .unwrap();
    assert_eq!(2, segments.len());
    assert_eq!(
        "Ada Lovelace",
        segments[0]["entity_caption"].as_str().unwrap()
    );
    assert_eq!(
        "Grace Hopper",
        segments[1]["entity_caption"].as_str().unwrap()
    );
    for segment in segments {
        assert_eq!("below", segment["caption_layout"].as_str().unwrap());
        assert_eq!(
            "premium_below_random_v1",
            segment["entity_style_id"].as_str().unwrap()
        );
        assert_eq!(
            0.0,
            segment["params"]["position_x"].as_f64().unwrap(),
            "a chained card must keep its centered viewport"
        );
    }
    assert!(fused.iter().any(|item| item.kind == "image"));
}

#[test]
fn general_chain_fuses_pending_entity_cards_with_verified_entity_images() {
    let verified = chain_entity_image("ada", "Ada Lovelace", 0, 2_000_000);
    let mut pending = chain_entity_image("grace", "Grace Hopper", 2_400_000, 2_000_000);
    pending.kind = "entity_card".to_string();
    pending.template_id = "person_default".to_string();
    pending.asset_refs.clear();
    let second_verified = chain_entity_image("alan", "Alan Turing", 4_800_000, 2_000_000);
    let fused = super::coalesce_chained_overlays(
        vec![verified, pending, second_verified],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(20_000_000),
    );
    assert_eq!(1, fused.len(), "identity cards and images form one run");
    let segments = fused[0].params.as_ref().unwrap()["chain_segments"]
        .as_array()
        .unwrap();
    assert_eq!(3, segments.len());
    assert_eq!(
        vec!["entity_image", "entity_card", "entity_image"],
        segments
            .iter()
            .map(|segment| segment["kind"].as_str().unwrap())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        "mixed_chain", fused[0].kind,
        "Go promotes the pending card and re-labells the container as an image_chain"
    );
}

#[test]
fn general_chain_keeps_image_runs_separate_from_text() {
    let items = vec![
        chain_image("img", 100_000, 500_000, "image_focus_reveal"),
        chain_phrase("a", "alpha", 700_000, 500_000, "motion_a"),
        chain_phrase("b", "beta", 1_300_000, 500_000, "motion_b"),
    ];
    let fused = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(10_000_000),
    );
    assert_eq!(2, fused.len());
    assert_eq!("image", fused[0].kind);
    assert_eq!("phrase_chain", fused[1].kind);
    let segments = fused[1].params.as_ref().unwrap()["chain_segments"]
        .as_array()
        .unwrap();
    assert_eq!("alpha", segments[0]["text"].as_str().unwrap());
    assert!(fused.iter().all(|item| item.kind != "image_chain"));
}

#[test]
fn general_chain_keeps_plain_image_runs_standalone() {
    let items = (0..4)
        .map(|index| {
            chain_image(
                &format!("img-{index}"),
                index * 5_000_000,
                5_000_000,
                "image_motion",
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(4, items.len());
    assert!(items.iter().all(|item| item.scene_id == "scene-1"));
    assert_eq!(
        21_200_000,
        super::projected_chain_end_us(&items, super::CHAIN_HANDOFF_US)
    );
    let room = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(40_000_000),
    );
    // Plain photos are never fused, even when a chain would fit: the
    // renderer draws a chain's generic image segments as a simultaneous
    // deck instead of a sequence. Only captioned entity cards chain.
    assert_eq!(4, room.len());
    assert!(room.iter().all(|item| item.kind == "image"));
    assert_eq!(
        room.iter().map(|item| item.start_us).collect::<Vec<_>>(),
        vec![0, 5_000_000, 10_000_000, 15_000_000]
    );
}

#[test]
fn chain_camera_knob_selects_a_camera_pan_for_image_chains_only() {
    let image_items = vec![
        chain_entity_image("ada", "Ada Lovelace", 0, 2_000_000),
        chain_entity_image("grace", "Grace Hopper", 2_400_000, 2_000_000),
    ];
    for (value, want) in [
        (1, "image_group_camera_pan_right"),
        (2, "image_group_camera_pan_left"),
        (3, "image_group_camera_pan_down"),
        (4, "image_group_camera_pan_up"),
    ] {
        let fused = super::coalesce_chained_overlays(
            image_items.clone(),
            &chain_request(&[("chain_enabled", 1), ("chain_camera", value)]),
            &chain_ends(20_000_000),
        );
        let chain = fused
            .iter()
            .find(|item| item.kind == "image_chain")
            .expect("captioned images must chain");
        assert_eq!(
            want,
            chain.params.as_ref().unwrap()["chain_camera"]
                .as_str()
                .unwrap(),
            "chain_camera={value} must select {want}"
        );
    }

    let text_items = vec![
        chain_phrase("a", "alpha", 100_000, 500_000, "motion_a"),
        chain_phrase("b", "beta", 700_000, 500_000, "motion_b"),
    ];
    let fused_text = super::coalesce_chained_overlays(
        text_items,
        &chain_request(&[("chain_enabled", 1), ("chain_camera", 1)]),
        &chain_ends(20_000_000),
    );
    assert_eq!(
        "fly_under_fade",
        fused_text[0].params.as_ref().unwrap()["chain_camera"]
            .as_str()
            .unwrap(),
        "a text chain keeps the legacy handoff"
    );
}

#[test]
fn general_chain_trims_a_captioned_run_at_the_scene_end() {
    let items = vec![
        chain_entity_image("ada", "Ada Lovelace", 0, 5_000_000),
        chain_entity_image("grace", "Grace Hopper", 5_400_000, 5_000_000),
        chain_entity_image("alan", "Alan Turing", 10_800_000, 5_000_000),
    ];
    let fused = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(10_000_000),
    );
    let chains = fused
        .iter()
        .filter(|item| item.kind == "image_chain")
        .collect::<Vec<_>>();
    assert_eq!(
        1,
        chains.len(),
        "the run must chain instead of being dropped"
    );
    let segments = chains[0].params.as_ref().unwrap()["chain_segments"]
        .as_array()
        .unwrap();
    assert_eq!(
        2,
        segments.len(),
        "only the segments that start inside the scene are fused"
    );
    assert!(
        fused
            .iter()
            .any(|item| item.kind == "entity_image" && item.entity_caption == "Alan Turing"),
        "the card that cannot start inside the scene stays standalone"
    );
}

#[test]
fn general_chain_drops_a_tail_without_a_readable_window() {
    // grace ends at 8.8s, so alan would start at 9.2s: 800ms before the
    // scene end. That used to be fused and clamped to a sub-second flash.
    let items = vec![
        chain_entity_image("ada", "Ada Lovelace", 0, 4_000_000),
        chain_entity_image("grace", "Grace Hopper", 4_400_000, 4_400_000),
        chain_entity_image("alan", "Alan Turing", 9_200_000, 5_000_000),
    ];
    let fused = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(10_000_000),
    );
    let chain = fused
        .iter()
        .find(|item| item.kind == "image_chain")
        .expect("readable run must chain");
    let segments = chain.params.as_ref().unwrap()["chain_segments"]
        .as_array()
        .unwrap();
    assert_eq!(
        2,
        segments.len(),
        "a sub-second tail must not become a segment"
    );
    assert!(
        fused
            .iter()
            .any(|item| item.kind == "entity_image" && item.entity_caption == "Alan Turing"),
        "the unreadable tail stays standalone"
    );
}

#[test]
fn general_chain_caps_a_run_at_ten_segments() {
    let items = || {
        (0..12)
            .map(|index| chain_phrase("phrase", "text", index * 600_000, 500_000, "motion_a"))
            .collect::<Vec<_>>()
    };
    let fused = super::coalesce_chained_overlays(
        items(),
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(60_000_000),
    );
    let chains = fused
        .iter()
        .filter(|item| item.kind == "phrase_chain")
        .collect::<Vec<_>>();
    assert_eq!(2, chains.len(), "twelve items are two chains, not one");
    let first = chains[0].params.as_ref().unwrap()["chain_segments"]
        .as_array()
        .unwrap();
    assert_eq!(super::MAX_CHAIN_SEGMENTS, first.len());

    // A configured value above the ceiling cannot raise it.
    let capped = super::coalesce_chained_overlays(
        items(),
        &chain_request(&[("chain_enabled", 1), ("chain_max_segments", 50)]),
        &chain_ends(60_000_000),
    );
    let count = capped
        .iter()
        .filter(|item| item.kind == "phrase_chain")
        .map(|item| {
            item.params.as_ref().unwrap()["chain_segments"]
                .as_array()
                .unwrap()
                .len()
        })
        .max()
        .unwrap_or(0);
    assert_eq!(super::MAX_CHAIN_SEGMENTS, count);
}

#[test]
fn general_chain_honors_max_segments_and_clamps_the_gap() {
    let items = (0..4)
        .map(|index| chain_phrase("phrase", "text", index * 600_000, 500_000, "motion_a"))
        .collect::<Vec<_>>();
    let fused = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_gap_ms", 1_000), ("chain_max_segments", 2)]),
        &chain_ends(10_000_000),
    );
    assert_eq!(2, fused.len());
    assert!(fused.iter().all(|item| item.kind == "phrase_chain"));

    let far = vec![
        chain_phrase("a", "first", 0, 1_000_000, "motion_a"),
        chain_phrase("b", "second", 6_000_001, 1_000_000, "motion_b"),
    ];
    assert_eq!(
        2,
        super::coalesce_chained_overlays(
            far.clone(),
            &chain_request(&[("chain_gap_ms", 10_000)]),
            &chain_ends(20_000_000),
        )
        .len(),
        "the configured gap is clamped to five seconds"
    );
    assert_eq!(
        2,
        super::coalesce_chained_overlays(
            far,
            &chain_request(&[("chain_gap_ms", i64::MAX)]),
            &chain_ends(20_000_000),
        )
        .len(),
        "an oversized gap must clamp without overflowing"
    );
}

#[test]
fn general_chain_keeps_date_cards_standalone_but_metric_numbers_chain() {
    let mut date_a = chain_phrase(
        "date-0",
        "3 giugno 2007",
        1_600_000,
        3_200_000,
        "date_fade_rise",
    );
    date_a.kind = "number".to_string();
    date_a.template_id = "TIMELINE_DATE_CARD".to_string();
    let mut date_b = chain_phrase(
        "date-1",
        "14 ottobre 2010",
        4_900_000,
        3_200_000,
        "date_timeline_tick",
    );
    date_b.kind = "number".to_string();
    date_b.template_id = "TIMELINE_DATE_CARD".to_string();
    let dates = super::coalesce_chained_overlays(
        vec![date_a, date_b],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(20_000_000),
    );
    assert_eq!(2, dates.len());
    assert!(dates.iter().all(|item| item.kind == "number"));

    let mut metric_a = chain_phrase("metric-0", "42", 0, 2_000_000, "metric_counter_rise");
    metric_a.kind = "number".to_string();
    metric_a.template_id = "METRIC_STAT_CARD".to_string();
    let mut metric_b = chain_phrase("metric-1", "18", 2_500_000, 2_000_000, "metric_slide_left");
    metric_b.kind = "number".to_string();
    metric_b.template_id = "METRIC_STAT_CARD".to_string();
    let metrics = super::coalesce_chained_overlays(
        vec![metric_a, metric_b],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(20_000_000),
    );
    assert_eq!(1, metrics.len());
    assert_eq!("phrase_chain", metrics[0].kind);
}

#[test]
fn general_chain_keeps_maps_and_chart_groups_standalone() {
    let mut map_a = chain_phrase("map-a", "", 0, 3_000_000, "map_motion");
    map_a.kind = "map".to_string();
    map_a.template_id = "MAP".to_string();
    map_a.map = Some(serde_json::json!({"provider": "local"}));
    let mut map_b = chain_phrase("map-b", "", 1_000_000, 3_000_000, "map_motion");
    map_b.kind = "map".to_string();
    map_b.template_id = "MAP".to_string();
    map_b.map = Some(serde_json::json!({"provider": "local"}));
    let maps = super::coalesce_chained_overlays(
        vec![map_a, map_b],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(20_000_000),
    );
    assert_eq!(2, maps.len());
    assert!(maps.iter().all(|item| item.kind == "map"));

    let alpha = chain_phrase("a", "alpha", 0, 2_000_000, "motion_a");
    let mut chart = chain_phrase("chart", "42%", 2_000_000, 2_000_000, "motion_chart");
    chart.params = Some(serde_json::json!({"chart_card": true, "chart_group_id": "g1"}));
    let beta = chain_phrase("b", "beta", 4_000_000, 2_000_000, "motion_b");
    let charted = super::coalesce_chained_overlays(
        vec![alpha, chart, beta],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(20_000_000),
    );
    assert_eq!(3, charted.len());
    assert!(charted.iter().all(|item| item.kind == "text_phrase"));
    assert!(charted.iter().any(|item| item
        .params
        .as_ref()
        .is_some_and(|params| params["chart_card"] == true)));
}

#[test]
fn general_chain_keeps_short_phrase_recipes_standalone() {
    let items = vec![
        chain_phrase("a", "alpha", 0, 2_500_000, "short_phrase_burst"),
        chain_phrase(
            "b",
            "beta",
            2_700_000,
            2_500_000,
            "short_phrase_editorial_focus_resolve",
        ),
    ];
    let fused = super::coalesce_chained_overlays(
        items,
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(10_000_000),
    );
    assert_eq!(2, fused.len());
    assert!(fused.iter().all(|item| item.kind == "text_phrase"));
}

#[test]
fn general_chain_treats_important_word_as_text_family() {
    let mut first = chain_phrase("a", "alpha", 0, 2_000_000, "text_word_rise");
    first.kind = "important_word".to_string();
    first.template_id = "IMPORTANT_WORD".to_string();
    let mut second = chain_phrase("b", "beta", 2_100_000, 2_000_000, "text_word_slide");
    second.kind = "important_word".to_string();
    second.template_id = "IMPORTANT_WORD".to_string();
    let fused = super::coalesce_chained_overlays(
        vec![first, second],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(10_000_000),
    );
    assert_eq!(1, fused.len());
    assert_eq!("phrase_chain", fused[0].kind);
}

#[test]
fn general_chain_keeps_a_text_card_textual_and_the_portrait_with_its_owner() {
    let portrait = super::AssetRefV2 {
        asset_id: "portrait-ada".to_string(),
        url: "https://example.test/ada.jpg".to_string(),
        sha256: "a".repeat(64),
        media_type: "image/jpeg".to_string(),
    };
    let mut image = chain_image("img", 8_000_000, 2_500_000, "image_focus_reveal");
    image.kind = "entity_image".to_string();
    image.template_id = "image_popup".to_string();
    image.entity_id = "ent_ada".to_string();
    image.entity_caption = "Ada".to_string();
    image.asset_refs = vec![portrait];
    let mut card = chain_phrase("card", "", 10_500_000, 2_500_000, "card_motion");
    card.kind = "entity_card".to_string();
    card.template_id = "person_default".to_string();
    card.entity_id = "ent_ada".to_string();
    card.entity_caption = "Ada".to_string();
    let phrase = chain_phrase("phrase", "la frase", 12_000_000, 2_000_000, "phrase_motion");

    let disabled = super::coalesce_chained_overlays(
        vec![image.clone(), card.clone(), phrase.clone()],
        &chain_request(&[]),
        &chain_ends(30_000_000),
    );
    assert_eq!(3, disabled.len());
    let card_out = disabled.iter().find(|item| item.id == "card").unwrap();
    assert_eq!(
        "entity_card", card_out.kind,
        "coalescing never promotes a text card"
    );
    assert!(
        card_out.asset_refs.is_empty(),
        "a portrait is bound by the card builder, not copied from a donor"
    );

    let chained = super::coalesce_chained_overlays(
        vec![image, card, phrase],
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(30_000_000),
    );
    assert_eq!(1, chained.len(), "one run, one container");
    let mixed = chained
        .iter()
        .find(|item| item.kind == "mixed_chain")
        .expect("mixed chain");
    let segments = mixed.params.as_ref().unwrap()["chain_segments"]
        .as_array()
        .unwrap();
    assert_eq!(3, segments.len());
    assert_eq!("ent_ada", segments[0]["entity_id"].as_str().unwrap());
    assert_eq!(
        "portrait-ada",
        segments[0]["assets"][0]["asset_id"].as_str().unwrap()
    );
    assert!(
        segments[1]
            .get("assets")
            .is_none_or(|assets| assets.as_array().is_none_or(|assets| assets.is_empty())),
        "the text card stays text inside the chain: {:?}",
        segments[1]
    );
}
