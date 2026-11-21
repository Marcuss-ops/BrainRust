#[test]
fn editorial_bullet_pin_controls_font_and_entrance() {
    let mut request = request();
    request.canvas.editorial_bullet_styles = serde_json::from_value(serde_json::json!([{
        "scene_id": "scene-1",
        "font_family": "oswald",
        "font_size_px": 96,
        "motion_id": "air_rise_type_on"
    }]))
    .expect("bullet pin");
    let compiled = compile_plan(&request).expect("plan");
    let bullet = compiled
        .plan
        .items
        .iter()
        .find(|item| {
            item.params
                .as_ref()
                .and_then(|params| params.get("editorial_role"))
                .and_then(serde_json::Value::as_str)
                == Some("scene_bullet")
        })
        .expect("scene bullet");
    assert_eq!("air_rise_type_on", bullet.motion_id);
    assert_eq!(
        Some("oswald"),
        bullet.params.as_ref().expect("params")["font_family"].as_str()
    );
    assert_eq!(
        Some(96.0),
        bullet.params.as_ref().expect("params")["font_size_px"].as_f64()
    );
    assert_eq!(
        Some(96.0),
        bullet.style.as_ref().expect("style")["font_size"].as_f64()
    );
}

#[test]
fn short_phrase_motion_follows_the_word_count_bucket() {
    let mut request = request();
    request.canvas.short_phrase_motions_by_word = std::collections::BTreeMap::from([
        (
            "2".to_string(),
            vec!["typewriter_slow_type_short".to_string()],
        ),
        (
            "4".to_string(),
            vec!["typewriter_word_burst_short".to_string()],
        ),
    ]);
    let mut items = vec![
        phrase_item("two", "Nike corre", None),
        phrase_item("four", "Nike corre al museo", None),
        phrase_item("bullet", "Nike corre", Some("scene_bullet")),
        phrase_item(
            "long",
            "una frase decisamente troppo lunga per la corsia breve",
            None,
        ),
    ];
    assign_short_phrase_motions(&mut items, &request);
    assert_eq!(items[0].motion_id, "typewriter_slow_type_short");
    assert_eq!(items[1].motion_id, "typewriter_word_burst_short");
    assert_eq!(
        items[2].motion_id, "typewriter_slow_type_short",
        "an unpinned editorial bullet rotates the bucket like a plain phrase"
    );
    assert_eq!(
        items[3].motion_id, "",
        "six words and up belong to the long lane"
    );
}

#[test]
fn editorial_bullets_and_titles_rotate_without_a_motion_pin() {
    let mut request = request();
    request.canvas.short_phrase_motions_by_word = std::collections::BTreeMap::from([(
        "3".to_string(),
        vec![
            "typewriter_glyph_drift_short".to_string(),
            "typewriter_rapid_fire_short".to_string(),
            "typewriter_word_lift_short".to_string(),
        ],
    )]);
    let mut items = vec![
        phrase_item("title", "Tre parole qui", Some("scene_title")),
        phrase_item("bullet-a", "Tre parole qui", Some("scene_bullet")),
        phrase_item("bullet-b", "Tre parole qui", Some("chapter_bullet")),
    ];
    assign_short_phrase_motions(&mut items, &request);
    let motions = items
        .iter()
        .map(|item| item.motion_id.as_str())
        .collect::<Vec<_>>();
    let expected = [
        "typewriter_glyph_drift_short",
        "typewriter_rapid_fire_short",
        "typewriter_word_lift_short",
    ];
    let unique: std::collections::BTreeSet<&str> = motions.iter().copied().collect();
    assert_eq!(
        unique.len(),
        expected.len(),
        "editorial titles and bullets must rotate distinct entries, got {motions:?}"
    );
    assert!(
        motions.iter().all(|motion| expected.contains(motion)),
        "editorial titles and bullets must not fall back to the preset default, got {motions:?}"
    );
}

#[test]
fn bullet_pin_motion_wins_but_font_only_pin_still_rotates() {
    let mut request = request();
    request.canvas.short_phrase_motions_by_word = std::collections::BTreeMap::from([(
        "3".to_string(),
        vec!["typewriter_staccato_short".to_string()],
    )]);
    request.canvas.editorial_bullet_styles = serde_json::from_value(serde_json::json!([
        {"scene_id": "scene-1", "font_family": "oswald", "motion_id": "air_rise_type_on"},
        {"scene_id": "scene-2", "font_family": "abril_fatface"}
    ]))
    .expect("bullet pins");
    let mut pinned = phrase_item("pinned", "Tre parole qui", Some("scene_bullet"));
    pinned.scene_id = "scene-1".to_string();
    let mut font_only = phrase_item("font-only", "Tre parole qui", Some("scene_bullet"));
    font_only.scene_id = "scene-2".to_string();
    let mut items = vec![pinned, font_only];
    assign_short_phrase_motions(&mut items, &request);
    assert_eq!(
        items[0].motion_id, "",
        "a motion pin is written later by apply_editorial_bullet_pins"
    );
    assert_eq!(
        items[1].motion_id, "typewriter_staccato_short",
        "a font-only pin does not select an entrance"
    );
}

#[test]
fn short_phrase_pin_wins_over_the_bucket() {
    let mut request = request();
    request.canvas.short_phrase_motion = "typewriter_glyph_square_short".to_string();
    let mut items = vec![
        phrase_item("two", "Nike corre", None),
        phrase_item("four", "Nike corre al museo", None),
    ];
    assign_short_phrase_motions(&mut items, &request);
    assert_eq!(items[0].motion_id, "typewriter_glyph_square_short");
    assert_eq!(items[1].motion_id, "typewriter_glyph_square_short");
}

#[test]
fn short_phrase_rotation_covers_every_typewriter_gesture_without_repeats() {
    let mut request = request();
    let typewriters = [
        "typewriter_glyph_drift_short",
        "typewriter_glyph_square_short",
        "typewriter_impact_push_short",
        "typewriter_rapid_fire_short",
        "typewriter_slow_type_short",
        "typewriter_soft_glide_short",
        "typewriter_staccato_short",
        "typewriter_word_burst_short",
        "typewriter_word_lift_short",
        "typewriter_word_step_short",
    ];
    let mut bucket = vec!["legacy_first".to_string(), "legacy_second".to_string()];
    bucket.extend(typewriters.iter().map(|id| id.to_string()));
    request.canvas.short_phrase_motions_by_word =
        std::collections::BTreeMap::from([("3".to_string(), bucket)]);
    let build = || {
        let mut items: Vec<PlanItemV2> = (0..12)
            .map(|index| phrase_item(&format!("p{index}"), "tre parole qui", None))
            .collect();
        assign_short_phrase_motions(&mut items, &request);
        items
            .into_iter()
            .map(|item| item.motion_id)
            .collect::<Vec<_>>()
    };
    let motions = build();
    assert_eq!(
        motions,
        build(),
        "replaying the same plan must pick the same rotation"
    );
    for motion in &motions[..10] {
        assert!(
            typewriters.contains(&motion.as_str()),
            "the first ten phrases must show the typewriter family, got {motion:?}"
        );
    }
    let unique: std::collections::BTreeSet<&str> = motions.iter().map(String::as_str).collect();
    assert_eq!(
        unique.len(),
        motions.len(),
        "a twelve-phrase run must not repeat a gesture: {motions:?}"
    );
    assert!(
        motions[10..].iter().any(|id| id.starts_with("legacy_")),
        "legacy short gestures must still be reachable after the typewriter block"
    );
}

#[test]
fn short_phrase_motion_is_stable_and_length_specific() {
    let mut request = request();
    request.canvas.short_phrase_motions_by_word = std::collections::BTreeMap::from([
        (
            "2".to_string(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
        ),
        ("4".to_string(), vec!["d".to_string(), "e".to_string()]),
    ]);
    let build = || {
        let mut items = vec![
            phrase_item("one", "due parole", None),
            phrase_item("two", "quattro parole qui ora", None),
            phrase_item("three", "due parole", None),
        ];
        assign_short_phrase_motions(&mut items, &request);
        items
            .into_iter()
            .map(|item| item.motion_id)
            .collect::<Vec<_>>()
    };
    let first = build();
    assert_eq!(
        first,
        build(),
        "replaying the same plan must pick the same motions"
    );
    assert!(["a", "b", "c"].contains(&first[0].as_str()));
    assert!(["d", "e"].contains(&first[1].as_str()));
}

#[test]
fn long_phrase_motion_rotates_the_long_pool() {
    let mut request = request();
    request.canvas.long_phrase_motions = vec![
        "typewriter_glyph_wrap_long".to_string(),
        "doc_01_yellow_marker_sweep".to_string(),
    ];
    let mut items = vec![
        phrase_item("long", "una frase di sei parole esatte qui adesso", None),
        phrase_item("short", "Nike corre al museo", None),
    ];
    assign_long_phrase_motions(&mut items, &request);
    assert!(
        ["typewriter_glyph_wrap_long", "doc_01_yellow_marker_sweep"]
            .contains(&items[0].motion_id.as_str()),
        "long lane picked {:?}",
        items[0].motion_id
    );
    assert_eq!(
        items[1].motion_id, "",
        "the short lane belongs to the short selector"
    );
}

#[test]
fn long_phrase_without_a_pool_keeps_the_presentation_default() {
    let mut request = request();
    request.canvas.long_phrase_motions = Vec::new();
    let mut items = vec![phrase_item(
        "long",
        "una frase di sei parole esatte qui adesso",
        None,
    )];
    assign_long_phrase_motions(&mut items, &request);
    assert_eq!(items[0].motion_id, "");
}

#[test]
fn long_text_is_split_into_candidates_before_chain_coalescing() {
    let long = "parole di prova ".repeat(40); // 640 runes
    let mut first = phrase_item("long-a", &long, None);
    first.start_us = 0;
    first.duration_us = 8_000_000;
    first.start_ms = 0;
    first.end_ms = 8_000;
    let mut second = phrase_item("long-b", &long, None);
    second.start_us = 8_000_000;
    second.duration_us = 8_000_000;
    second.start_ms = 8_000;
    second.end_ms = 16_000;

    let budgeted = super::split_long_text_candidates(vec![first, second]);
    assert!(
        budgeted.len() > 2,
        "long candidates must be split before they enter a chain"
    );
    assert!(
        budgeted
            .iter()
            .filter(|item| item.kind == "text_phrase")
            .all(|item| item.text.chars().count() <= super::MAX_TEXT_RUNES),
        "every candidate must respect the copy budget"
    );

    let fused = super::coalesce_chained_overlays(
        budgeted,
        &chain_request(&[("chain_enabled", 1)]),
        &chain_ends(20_000_000),
    );
    let chain = fused
        .iter()
        .find(|item| item.kind == "phrase_chain")
        .expect("the budgeted phrases must still fuse into a chain");
    let segments = chain.params.as_ref().expect("chain params")["chain_segments"]
        .as_array()
        .expect("chain segments");
    assert!(segments.len() > 2);
    assert!(
        segments.iter().all(|segment| {
            segment["text"].as_str().unwrap().chars().count() <= super::MAX_TEXT_RUNES
        }),
        "a fused chain must never contain an over-budget segment"
    );
}

#[test]
fn long_phrase_split_is_counted_in_intake() {
    let mut request = request();
    let editorial = request.editorial.as_mut().expect("editorial");
    editorial.scenes[0].title = None;
    editorial.scenes[0].bullets.clear();
    request.scenes[0].annotations = Some(
            serde_json::from_value(serde_json::json!({
                "phrases": [{
                    "text": "This is a deliberately long certified phrase that must be split into multiple render items without lying about the sealed intake accounting ".repeat(2),
                    "start_us": 0,
                    "duration_us": 2_000_000,
                    "score": 1.0
                }],
                "end_us": 2_000_000
            }))
            .expect("annotations"),
        );
    let compiled = compile_plan(&request).expect("plan with split phrase");
    let phrases = compiled
        .plan
        .items
        .iter()
        .filter(|item| item.kind == "text_phrase")
        .count();
    assert!(phrases >= 2, "long phrase must produce split items");
    assert!(
        compiled.plan.intake.candidates >= compiled.plan.intake.planned,
        "sealed intake must account for the split items"
    );
    assert!(
        compiled
            .plan
            .intake
            .candidates_by_kind
            .get("text_phrase")
            .copied()
            .unwrap_or(0)
            >= 2,
        "split items must be accounted for by kind"
    );
}
