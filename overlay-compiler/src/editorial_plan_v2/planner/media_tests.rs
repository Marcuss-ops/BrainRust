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
        let distinct: std::collections::BTreeSet<&str> = captions.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            captions.len().min(2),
            "the bag must not repeat before the compatible pool is exhausted: {captions:?}"
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
        let mut decisions = Vec::new();
        append_entity_cards(&mut items, &request, &mut decisions).expect("entity cards");
        assert!(
            decisions.iter().any(|decision| {
                decision.reason_code == super::decisions::REASON_DUPLICATE_IDENTITY
                    && decision.scene_id == "scene-1"
            }),
            "the second Augias occurrence in scene-1 must carry a typed decision: {decisions:#?}"
        );
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

#[cfg(test)]
mod image_pool_tests {
    use super::*;

    fn request_with_images(pool: Vec<&str>) -> EditorialPlanV2Request {
        serde_json::from_value(serde_json::json!({
            "operation": "compile_editorial_plan",
            "schema_version": "editorial_plan.v2",
            "plan": {
                "plan_id": "plan-images", "video_id": "video-images",
                "project_id": "project-images", "motion_seed": "seed-images",
                "width": 1920, "height": 1080, "fps_num": 24, "fps_den": 1
            },
            "language": "it",
            "seal": {"audio_duration_us": 8_000_000},
            "canvas": {"image_motions": pool},
            "scenes": [{
                "id": "scene-1", "text": "", "duration_ms": 8000,
                "timeline_start_us": 0, "duration_us": 8_000_000
            }]
        }))
        .expect("request")
    }

    fn standalone_image(id: &str, start_us: i64) -> PlanItemV2 {
        PlanItemV2 {
            id: id.to_string(),
            scene_id: "scene-1".to_string(),
            kind: "image".to_string(),
            start_us,
            duration_us: 2_000_000,
            ..Default::default()
        }
    }

    /// ANIMRAND-04: a missing pool is a configuration error, never a silent
    /// `image_focus_reveal` on every image.
    #[test]
    fn standalone_image_without_a_pool_fails_closed() {
        let request = request_with_images(Vec::new());
        let mut items = vec![standalone_image("img-1", 0)];
        let error = normalize_image_presentation(&request, &mut items)
            .expect_err("an empty image pool must fail closed");
        assert!(
            error.to_string().contains("canvas.image_motions"),
            "error must name the missing pool: {error}"
        );
        assert!(
            items[0].motion_id.is_empty(),
            "a failed normalization must not pin a fallback"
        );
    }

    #[test]
    fn standalone_image_uses_the_certified_pool() {
        let request = request_with_images(vec!["image_a", "image_b"]);
        let mut items = vec![
            standalone_image("img-1", 0),
            standalone_image("img-2", 3_000_000),
        ];
        normalize_image_presentation(&request, &mut items).expect("normalize");
        assert!(
            items
                .iter()
                .all(|item| request.canvas.image_motions.contains(&item.motion_id)),
            "motions must come from the pool: {:?}",
            items.iter().map(|item| &item.motion_id).collect::<Vec<_>>()
        );
        assert_ne!(
            items[0].motion_id, items[1].motion_id,
            "neighbouring images must not share the same entrance"
        );
    }
}

#[cfg(test)]
mod explicit_motion_guard_tests {
    use super::*;

    fn request() -> EditorialPlanV2Request {
        serde_json::from_value(serde_json::json!({
            "operation": "compile_editorial_plan",
            "schema_version": "editorial_plan.v2",
            "plan": {
                "plan_id": "plan-explicit", "video_id": "video-explicit",
                "project_id": "project-explicit", "motion_seed": "seed-explicit",
                "width": 1920, "height": 1080, "fps_num": 24, "fps_den": 1
            },
            "language": "it",
            "seal": {"audio_duration_us": 4_000_000},
            "canvas": {
                "image_motions": ["image_a", "image_b"],
                "entity_image_motions": ["entity_a", "entity_b"],
                "entity_caption_motions": ["caption_a", "caption_b"],
                "date_motions": ["date_a", "date_b"],
                "metric_motions": ["metric_a", "metric_b"]
            },
            "scenes": [{
                "id": "scene-1", "text": "", "duration_ms": 4000,
                "timeline_start_us": 0, "duration_us": 4_000_000
            }]
        }))
        .expect("request")
    }

    /// ANIMRAND-09: an entrance pinned by the producer survives every automatic
    /// lane — numeric, entity image, caption and standalone image.
    #[test]
    fn explicit_motions_are_never_overwritten() {
        let request = request();

        let mut numbers = vec![PlanItemV2 {
            id: "date".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "number".to_string(),
            template_id: "TIMELINE_DATE_CARD".to_string(),
            motion_id: "pinned_date".to_string(),
            ..Default::default()
        }];
        assign_numeric_motions(&mut numbers, &request);
        assert_eq!("pinned_date", numbers[0].motion_id);

        let asset = || AssetRefV2 {
            asset_id: "portrait".to_string(),
            url: "https://cdn.example/portrait.png".to_string(),
            sha256: "a".repeat(64),
            media_type: "image/png".to_string(),
        };
        let mut cards = vec![PlanItemV2 {
            id: "card".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "entity_image".to_string(),
            motion_id: "pinned_entity".to_string(),
            entity_caption: "Ada".to_string(),
            caption_motion_id: "pinned_caption".to_string(),
            asset_refs: vec![asset()],
            ..Default::default()
        }];
        assign_entity_media(&request, &mut cards).expect("assign entity media");
        assert_eq!("pinned_entity", cards[0].motion_id);
        assert_eq!("pinned_caption", cards[0].caption_motion_id);

        let mut images = vec![PlanItemV2 {
            id: "image".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "image".to_string(),
            motion_id: "pinned_image".to_string(),
            start_us: 0,
            duration_us: 2_000_000,
            ..Default::default()
        }];
        normalize_image_presentation(&request, &mut images).expect("normalize");
        assert_eq!("pinned_image", images[0].motion_id);
    }
}
