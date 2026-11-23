use super::*;

#[cfg(test)]
mod entity_window_sequencing_tests {
    use super::*;

    fn entity_image(id: &str, start_us: i64, duration_us: i64) -> PlanItemV2 {
        PlanItemV2 {
            id: id.to_string(),
            scene_id: "scene-3".to_string(),
            kind: "entity_image".to_string(),
            start_us,
            duration_us,
            ..Default::default()
        }
    }

    fn scene_windows() -> std::collections::BTreeMap<String, [i64; 2]> {
        std::collections::BTreeMap::from([("scene-3".to_string(), [50_000_000, 80_000_000])])
    }

    fn scene() -> SceneV2 {
        SceneV2 {
            id: "scene-3".to_string(),
            text: String::new(),
            duration_ms: 30_000,
            execution_mode: String::new(),
            short_phrase_overlay: false,
            overlay_categories: std::collections::BTreeMap::new(),
            index: 2,
            voiceover: None,
            timeline_start_us: 50_000_000,
            duration_us: 30_000_000,
            annotations: None,
            raw: None,
        }
    }

    fn occurrence(audio_start_us: i64, audio_end_us: i64) -> EntityOccurrenceV2 {
        EntityOccurrenceV2 {
            entity_id: "ent_ada".to_string(),
            name: "Ada Lovelace".to_string(),
            entity_type: "PERSON".to_string(),
            scene_id: "scene-3".to_string(),
            scene_index: 2,
            text_start: 0,
            text_end: 0,
            word_start: 0,
            word_end: 0,
            local_start_us: audio_start_us - 50_000_000,
            local_end_us: audio_end_us - 50_000_000,
            timeline_start_us: 50_000_000,
            audio_start_us,
            audio_end_us,
            confidence: 1.0,
            role: String::new(),
            canonical_entity_id: String::new(),
        }
    }

    /// The spoken window is the authority: a mention six seconds into scene 3
    /// must not start at the scene anchor (that produced 10-20 second stills).
    #[test]
    fn entity_window_prefers_the_spoken_start_over_the_scene_anchor() {
        let (start, duration) =
            entity_window(&scene(), &occurrence(56_000_000, 57_200_000)).expect("window");
        assert_eq!(56_000_000, start);
        assert_eq!(
            ENTITY_IMAGE_MIN_HOLD_US, duration,
            "a 1.2s mention still gets the readable 2.5s hold"
        );
    }

    /// A scene-relative `audio_start_us` next to an absolute `audio_end_us`
    /// (the mixed shape seen on scene 2 and later) must still start at the
    /// mention, never at the scene anchor.
    #[test]
    fn entity_window_anchors_a_scene_relative_audio_start_on_the_scene() {
        let mut mixed = occurrence(0, 0);
        mixed.local_start_us = 19_956_195;
        mixed.local_end_us = 21_136_195;
        mixed.audio_start_us = 19_956_195;
        mixed.audio_end_us = 47_732_000;
        let (start, duration) = entity_window(&scene(), &mixed).expect("window");
        assert_eq!(69_956_195, start);
        assert_eq!(ENTITY_IMAGE_MIN_HOLD_US, duration);
    }

    /// Without a spoken window the occurrence falls back to the scene anchor,
    /// clamped to the readable floor and never past the scene end.
    #[test]
    fn entity_window_falls_back_to_the_scene_anchor_without_audio_timing() {
        let (start, duration) = entity_window(&scene(), &occurrence(0, 0)).expect("window");
        assert_eq!(50_000_000, start);
        assert_eq!(ENTITY_IMAGE_MIN_HOLD_US, duration);
    }

    /// Three entities named in the same scene all fall back to the scene start.
    /// They must take turns instead of stacking on top of each other.
    #[test]
    fn cards_that_share_a_window_are_sequenced() {
        let items = vec![
            entity_image("a", 50_000_000, 1_000_000),
            entity_image("b", 50_000_000, 1_000_000),
            entity_image("c", 50_000_000, 1_000_000),
        ];
        let items = sequence_scene_image_windows(&scene_windows(), items);
        assert_eq!(items[0].start_us, 50_000_000);
        assert_eq!(items[0].duration_us, ENTITY_IMAGE_MIN_HOLD_US);
        assert_eq!(items[1].start_us, 52_900_000);
        assert_eq!(items[1].duration_us, ENTITY_IMAGE_MIN_HOLD_US);
        assert_eq!(items[2].start_us, 55_800_000);
        for pair in items.windows(2) {
            assert!(
                pair[1].start_us >= pair[0].start_us + pair[0].duration_us,
                "cards must not overlap: {:?} then {:?}",
                pair[0].id,
                pair[1].id
            );
        }
    }

    /// A card never outlives its scene, and a late card still fits by shrinking.
    #[test]
    fn sequencing_stays_inside_the_scene_window() {
        let items = vec![
            entity_image("a", 50_000_000, 2_000_000),
            entity_image("b", 76_000_000, 2_000_000),
        ];
        let items = sequence_scene_image_windows(&scene_windows(), items);
        for item in &items {
            assert!(item.start_us >= 50_000_000);
            assert!(item.start_us + item.duration_us <= 80_000_000);
        }
    }

    /// Plain images share the lane: two images admitted at the same instant
    /// must also take turns.
    #[test]
    fn plain_images_are_sequenced_with_the_cards() {
        let items = vec![
            PlanItemV2 {
                id: "card".to_string(),
                scene_id: "scene-3".to_string(),
                kind: "entity_image".to_string(),
                start_us: 50_000_000,
                duration_us: 3_000_000,
                ..Default::default()
            },
            PlanItemV2 {
                id: "photo".to_string(),
                scene_id: "scene-3".to_string(),
                kind: "image".to_string(),
                start_us: 50_000_000,
                duration_us: 3_000_000,
                ..Default::default()
            },
        ];
        let items = sequence_scene_image_windows(&scene_windows(), items);
        assert_eq!(items[1].start_us, 53_400_000);
    }

    /// Other kinds keep their admitted window untouched.
    #[test]
    fn text_and_plain_images_are_not_resequenced() {
        let items = vec![PlanItemV2 {
            id: "phrase".to_string(),
            scene_id: "scene-3".to_string(),
            kind: "text_phrase".to_string(),
            start_us: 50_000_000,
            duration_us: 3_000_000,
            ..Default::default()
        }];
        let items = sequence_scene_image_windows(&scene_windows(), items);
        assert_eq!(items[0].start_us, 50_000_000);
        assert_eq!(items[0].duration_us, 3_000_000);
    }

    #[test]
    fn caption_rotation_never_assigns_the_side_slide_motion() {
        let request: EditorialPlanV2Request = serde_json::from_value(serde_json::json!({
            "operation": "compile_editorial_plan",
            "schema_version": "editorial_plan.v2",
            "plan": {
                "plan_id": "plan-captions", "video_id": "video-captions",
                "project_id": "project-captions", "motion_seed": "seed-captions",
                "width": 1920, "height": 1080, "fps_num": 24, "fps_den": 1
            },
            "language": "it",
            "seal": {"audio_duration_us": 8_000_000},
            "canvas": {
                "entity_image_motions": ["entity_one"],
                "entity_caption_motions": [
                    "entity_split_side", "entity_caption_rise", "entity_name_pill"
                ]
            },
            "scenes": [{
                "id": "scene-1", "text": "", "duration_ms": 8000,
                "timeline_start_us": 0, "duration_us": 8_000_000
            }]
        }))
        .expect("request");
        let mut items = vec![
            entity_image("a", 0, 2_000_000),
            entity_image("b", 2_500_000, 2_000_000),
            entity_image("c", 5_000_000, 2_000_000),
        ];
        for item in &mut items {
            item.entity_caption = "Ada".to_string();
            item.asset_refs.push(AssetRefV2 {
                asset_id: "portrait".to_string(),
                url: "https://cdn.example/portrait.png".to_string(),
                sha256: "a".repeat(64),
                media_type: "image/png".to_string(),
            });
        }
        assign_entity_media(&request, &mut items).expect("assign captions");
        let captions: Vec<&str> = items
            .iter()
            .map(|item| item.caption_motion_id.as_str())
            .collect();
        assert!(
            captions.iter().all(|id| !id.is_empty()),
            "every caption must keep a real entrance: {captions:?}"
        );
        assert!(
            captions.iter().all(|id| !id.contains("split_side")),
            "the below-caption lane must never select the side slide: {captions:?}"
        );
        assert_eq!(
            captions,
            vec![
                "entity_caption_rise",
                "entity_name_pill",
                "entity_caption_rise"
            ],
            "the compatible pool must rotate in order"
        );
    }

    #[test]
    fn payload_badge_color_reaches_captioned_entity_style_only() {
        let mut card = entity_image("card", 50_000_000, 3_000_000);
        card.entity_caption = "Ada Lovelace".to_string();
        card.asset_refs.push(AssetRefV2 {
            asset_id: "portrait".to_string(),
            url: "https://cdn.example/portrait.png".to_string(),
            sha256: "a".repeat(64),
            media_type: "image/png".to_string(),
        });
        let mut items = vec![
            card,
            PlanItemV2 {
                id: "plain".to_string(),
                kind: "text_phrase".to_string(),
                ..Default::default()
            },
        ];
        apply_entity_badge_color(&mut items, "#336699");
        assert_eq!(
            items[0].style.as_ref().unwrap()["style"]["background"]["color"],
            "#336699"
        );
        assert!(
            items[1].style.is_none(),
            "badge plate leaked onto a text item"
        );
    }

    /// TICKET-49 local regression, carried by the reduced R30 fixture: two
    /// occurrences of Corrado Augias inside one scene must produce one card,
    /// while the same identity in a second scene still gets its own card.
    /// Occurrence ids differ, so the old per-overlay admission emitted both
    /// portraits in scene 1.
    #[test]
    fn r30_entity_cards_are_deduplicated_by_canonical_identity_per_scene() {
        let request: EditorialPlanV2Request = serde_json::from_str(include_str!(
            "../../../tests/fixtures/r30-entity-admission.json"
        ))
        .expect("R30 fixture request");
        let mut items = Vec::new();
        append_entity_cards(&mut items, &request).expect("entity cards");
        assert_eq!(2, items.len(), "one card per scene, not per occurrence");
        assert_eq!("scene-1", items[0].scene_id);
        assert_eq!("ent_augias_s1a", items[0].entity_id);
        assert_eq!("scene-2", items[1].scene_id);
        assert_eq!("ent_augias_s2", items[1].entity_id);
        for item in &items {
            assert_eq!(
                "person:corrado-augias",
                item.entity_ref.as_ref().unwrap().canonical_entity_id
            );
        }
    }
}
