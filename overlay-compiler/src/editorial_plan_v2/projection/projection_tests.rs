use serde_json::json;

use super::{dedupe_grounded_metric_numbers, project_request, timed};
use crate::editorial_plan_v2::request::{
    BudgetsV2, EditorialManifestV2, EditorialPlanV2Request, ImageCandidateV2, ImagePlanV2, MediaV2,
    SceneImageCandidateSetV2, SealV2,
};

fn request(overlay_categories: serde_json::Value) -> EditorialPlanV2Request {
    serde_json::from_value(json!({
        "operation": "compile_editorial_plan",
        "schema_version": "editorial_plan.v2",
        "plan": {
            "plan_id": "raw-plan", "video_id": "raw-video", "project_id": "raw-project",
            "motion_seed": "seed",
            "width": 1920, "height": 1080, "fps_num": 24, "fps_den": 1
        },
        "language": "it",
        "source_language": "it",
        "heavy_sentences": [
            {"text": "Roma ha 18%", "importance": 0.95}
        ],
        "scenes": [{
            "id": "s1",
            "text": "Roma ha 18% il 13 agosto 2007",
            "duration_ms": 4000,
            "timeline_start_us": 0,
            "duration_us": 4000000,
            "overlay_categories": overlay_categories,
            "voiceover": {"timing": {"duration_us": 4000000, "words": [
                {"index": 0, "text": "Roma", "start_us": 0, "end_us": 300000},
                {"index": 1, "text": "ha", "start_us": 300000, "end_us": 500000},
                {"index": 2, "text": "18%", "start_us": 500000, "end_us": 800000},
                {"index": 3, "text": "il", "start_us": 800000, "end_us": 900000},
                {"index": 4, "text": "13", "start_us": 900000, "end_us": 1000000},
                {"index": 5, "text": "agosto", "start_us": 1000000, "end_us": 1400000},
                {"index": 6, "text": "2007", "start_us": 1400000, "end_us": 1800000}
            ]}},
            "raw": {
                "important_phrases": [{"text": "Roma ha", "score": 0.2}],
                "primary_entities": [
                    {
                        "stable_entity_id": "ent_roma", "type": "GPE", "canonical_name": "Roma",
                        "confidence": 0.93, "geo": {"latitude": 41.9, "longitude": 12.5, "scope": "city"},
                        "plate_covered": true
                    },
                    {
                        "stable_entity_id": "ent_18", "type": "PERCENT", "canonical_name": "18%",
                        "confidence": 0.8
                    }
                ]
            }
        }],
        "entity_timeline": {
            "version": 1, "project_id": "raw-project", "language": "it", "duration_us": 4000000,
            "scenes": [{
                "scene_id": "s1", "scene_index": 0, "timeline_start_us": 0,
                "entities": [
                    {"entity_id": "ent_roma", "name": "Roma", "type": "GPE", "scene_id": "s1",
                     "audio_start_us": 0, "audio_end_us": 500000},
                    {"entity_id": "ent_18", "name": "18%", "type": "PERCENT", "scene_id": "s1",
                     "audio_start_us": 500000, "audio_end_us": 800000}
                ]
            }]
        }
    }))
    .expect("valid raw request")
}

#[test]
fn projects_phrases_numbers_dates_maps_and_heavy_boost() {
    let mut request = request(json!({}));
    project_request(&mut request);
    let annotations = request.scenes[0]
        .annotations
        .as_ref()
        .expect("projected annotations");

    assert_eq!(annotations.phrases.len(), 1);
    assert_eq!(annotations.phrases[0].text, "Roma ha");
    assert_eq!(annotations.phrases[0].start_us, 0);
    assert_eq!(annotations.phrases[0].duration_us, 500_000);
    assert_eq!(annotations.phrases[0].score, 0.95);

    assert_eq!(annotations.maps.len(), 1);
    assert_eq!(annotations.maps[0].label, "Roma");
    assert_eq!(annotations.maps[0].start_us, 0);

    assert!(annotations
        .numbers
        .iter()
        .any(|number| number.text == "18%"));
    assert!(annotations
        .numbers
        .iter()
        .any(|number| number.text == "13 agosto 2007"));
    assert_eq!(annotations.end_us, 4_000_000);
}

#[test]
fn raw_date_entity_does_not_add_a_bare_numeric_duplicate() {
    let mut request = request(json!({}));
    request.scenes[0]
        .raw
        .as_mut()
        .expect("raw scene")
        .secondary_entities
        .push(
            serde_json::from_value(json!({
                "stable_entity_id": "ent_date",
                "type": "DATE",
                "canonical_name": "13 agosto 2007",
                "confidence": 0.8,
                "mentions": [{"text": "13 agosto 2007", "start_rune": 17, "end_rune": 31}]
            }))
            .expect("raw date entity"),
        );
    request
        .entity_timeline
        .as_mut()
        .expect("entity timeline")
        .scenes[0]
        .entities
        .push(
            serde_json::from_value(json!({
                "entity_id": "ent_date",
                "name": "13 agosto 2007",
                "type": "DATE",
                "scene_id": "s1",
                "audio_start_us": 900000,
                "audio_end_us": 1800000
            }))
            .expect("date occurrence"),
        );

    project_request(&mut request);
    let dates: Vec<_> = request.scenes[0]
        .annotations
        .as_ref()
        .expect("projected annotations")
        .numbers
        .iter()
        .filter(|number| number.annotation_type == "DATE")
        .map(|number| number.text.as_str())
        .collect();
    assert_eq!(dates, ["13 agosto 2007"]);
    let date = request.scenes[0]
        .annotations
        .as_ref()
        .unwrap()
        .numbers
        .iter()
        .find(|number| number.annotation_type == "DATE")
        .unwrap();
    assert_eq!((900_000, 900_000), (date.start_us, date.duration_us));
}

#[test]
fn grounded_metric_entities_project_digit_surfaces_once() {
    let mut request = request(json!({}));
    let scene = &mut request.scenes[0];
    scene.text =
        "Il montaggio accosta 3 estratti, 4 programmi d’intervista e 2 testate per la cronologia."
            .to_string();
    let raw = scene.raw.as_mut().expect("raw scene");
    for (id, text, start, end) in [
        ("ent_3_items", "3 estratti", 21, 31),
        ("ent_3", "3", 21, 22),
        ("ent_4_items", "4 programmi", 33, 44),
        ("ent_4", "4", 33, 34),
        ("ent_2_items", "2 testate", 60, 69),
        ("ent_2", "2", 60, 61),
    ] {
        raw.secondary_entities.push(
            serde_json::from_value(json!({
                "stable_entity_id": id,
                "type": "NUMBER",
                "canonical_name": text,
                "confidence": 0.5,
                "mentions": [{"text": text, "start_rune": start, "end_rune": end}]
            }))
            .expect("raw metric entity"),
        );
        request
            .entity_timeline
            .as_mut()
            .expect("entity timeline")
            .scenes[0]
            .entities
            .push(
                serde_json::from_value(json!({
                    "entity_id": id,
                    "name": text,
                    "type": "NUMBER",
                    "scene_id": "s1",
                    "audio_start_us": start * 100000,
                    "audio_end_us": end * 100000
                }))
                .expect("metric occurrence"),
            );
    }

    project_request(&mut request);
    let numbers: Vec<_> = request.scenes[0]
        .annotations
        .as_ref()
        .expect("projected annotations")
        .numbers
        .iter()
        .filter(|number| number.annotation_type == "NUMBER")
        .map(|number| number.text.as_str())
        .collect();
    assert_eq!(numbers.iter().filter(|text| **text == "3").count(), 1);
    assert_eq!(numbers.iter().filter(|text| **text == "4").count(), 1);
    assert_eq!(numbers.iter().filter(|text| **text == "2").count(), 1);
}

#[test]
fn duplicate_metric_surfaces_collapse_but_dates_and_later_values_survive() {
    let numbers = vec![
        timed("18 unità", "NUMBER", 500_000, 2_000_000, 0.5),
        timed("18", "NUMBER", 500_000, 2_000_000, 0.3),
        timed("18", "NUMBER", 4_000_000, 5_000_000, 0.3),
        timed("13 agosto 2007", "DATE", 0, 1_000_000, 0.8),
    ];
    let out = dedupe_grounded_metric_numbers(numbers);
    assert_eq!(out.len(), 3);
    assert_eq!(
        out.iter()
            .filter(|number| number.text.contains("18"))
            .count(),
        2
    );
    assert!(out.iter().any(|number| number.text == "13 agosto 2007"));
}

#[test]
fn a_disabled_phrase_category_clears_the_projected_phrase() {
    let mut request = request(json!({"phrases": "disabled"}));
    project_request(&mut request);
    let annotations = request.scenes[0]
        .annotations
        .as_ref()
        .expect("projected annotations");
    assert!(annotations.phrases.is_empty());
}

fn candidate(sha: char) -> ImageCandidateV2 {
    ImageCandidateV2 {
        asset_id: format!("asset-{sha}"),
        sha256: sha.to_string().repeat(64),
        url: String::new(),
        media_type: "image/jpeg".to_string(),
        start_ms: 0,
        end_ms: 0,
        start_us: 0,
        duration_us: 0,
        score: 0.0,
    }
}

fn multi_scene_request() -> EditorialPlanV2Request {
    let mut request = request(json!({}));
    let mut scenes = Vec::new();
    for index in 0..5 {
        let mut scene = request.scenes[0].clone();
        scene.id = format!("s{}", index + 1);
        scene.index = index as i64;
        scene.text = format!("Scena {} con Roma ha 18%", index + 1);
        scene.timeline_start_us = index as i64 * 4_000_000;
        scene.duration_us = 4_000_000;
        scenes.push(scene);
        request.scene_windows.insert(
            format!("s{}", index + 1),
            [index as i64 * 4_000_000, (index + 1) as i64 * 4_000_000],
        );
    }
    request.scenes = scenes;
    if let Some(timeline) = request.entity_timeline.as_mut() {
        for scene in &mut timeline.scenes {
            scene.entities.clear();
        }
    }
    request.seal = Some(SealV2 {
        audio_duration_us: 20_000_000,
        output_profile_id: String::new(),
        background: None,
    });
    request.budgets = Some(BudgetsV2::default());
    request
}

#[test]
fn allocates_scene_images_round_robin_and_projects_recap_briefs() {
    let mut request = multi_scene_request();
    request.media = Some(MediaV2 {
        scene_image_candidates: vec![
            SceneImageCandidateSetV2 {
                scene_id: "s1".to_string(),
                candidates: vec![candidate('a'), candidate('b')],
            },
            SceneImageCandidateSetV2 {
                scene_id: "s2".to_string(),
                candidates: vec![candidate('a'), candidate('c')],
            },
            SceneImageCandidateSetV2 {
                scene_id: "s3".to_string(),
                candidates: vec![candidate('d')],
            },
        ],
        ..MediaV2::default()
    });
    request.image_plan = Some(ImagePlanV2 {
        enabled: true,
        ceiling: 3,
        excluded_sha256: vec!["b".repeat(64)],
    });

    project_request(&mut request);
    assert_eq!(
        request.scenes[0].annotations.as_ref().unwrap().images.len(),
        1
    );
    assert_eq!(
        request.scenes[1].annotations.as_ref().unwrap().images.len(),
        1
    );
    assert_eq!(
        request.scenes[2].annotations.as_ref().unwrap().images.len(),
        1
    );
    assert_eq!(
        request.scenes[3].annotations.as_ref().unwrap().images.len(),
        0
    );
    assert_ne!(
        request.scenes[0].annotations.as_ref().unwrap().images[0].sha256,
        request.scenes[1].annotations.as_ref().unwrap().images[0].sha256,
        "the duplicate hash must not be allocated twice"
    );
    assert_eq!(request.recap_briefs.len(), 5);
}

#[test]
fn recap_lane_turns_already_narrated_scenes_into_cards() {
    let mut request = multi_scene_request();
    request.media = Some(MediaV2::default());
    request.editorial = Some(EditorialManifestV2 {
        schema_version: "editorial.v1".to_string(),
        source_language: "it".to_string(),
        profile_version: "highlights.v1".to_string(),
        scenes: Vec::new(),
        certified: true,
        fingerprint: "a".repeat(64),
    });
    project_request(&mut request);
    let compiled = super::super::planner::compile_plan(&request).expect("compiled recap plan");
    let recaps = compiled
        .plan
        .items
        .iter()
        .filter(|item| {
            item.params
                .as_ref()
                .and_then(|params| params.get("recap"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    // A 4 s placement window holds exactly one 4 s recap card; the other two
    // candidates stay unplaced rather than being squeezed into the same slot.
    assert_eq!(recaps, 1);
    assert_eq!(compiled.plan.recap_placements, 1);
}

#[test]
fn a_short_scene_without_timing_becomes_one_phrase_card() {
    let mut request = request(json!({}));
    request.scenes[0].voiceover = None;
    request.scenes[0].short_phrase_overlay = true;
    request.scenes[0].raw = Some(crate::editorial_plan_v2::request::RawSceneV2::default());
    project_request(&mut request);
    let annotations = request.scenes[0]
        .annotations
        .as_ref()
        .expect("projected annotations");
    assert_eq!(annotations.phrases.len(), 1);
    assert_eq!(annotations.phrases[0].text, "Roma ha 18% il 13 agosto 2007");
    assert_eq!(annotations.phrases[0].duration_us, 4_000_000);
}

#[test]
fn grounded_visual_recipe_and_payload_are_projected_verbatim() {
    let mut plan_request = request(json!({}));
    let raw = plan_request.scenes[0].raw.as_mut().expect("raw scene");
    raw.visual_recipe = "counter_odometer".to_string();
    raw.visual_payload = Some(json!({
        "recipe": "counter_odometer",
        "value": 18.0,
        "unit": "PCT",
        "subject_label": "Roma"
    }));
    project_request(&mut plan_request);
    let annotations = plan_request.scenes[0]
        .annotations
        .as_ref()
        .expect("projected annotations");
    assert_eq!(annotations.visual_recipe, "counter_odometer");
    assert_eq!(
        annotations.visual_payload.as_ref().expect("payload")["value"],
        18.0
    );
}

#[test]
fn absent_visual_recipe_keeps_the_request_byte_identical() {
    let mut plan_request = request(json!({}));
    project_request(&mut plan_request);
    let serialized = serde_json::to_string(&plan_request).expect("serialize");
    assert!(!serialized.contains("visual_recipe"));
    assert!(!serialized.contains("visual_payload"));
}

#[test]
fn unsupported_visual_recipe_and_non_object_payload_fail_closed() {
    let mut plan_request = request(json!({}));
    project_request(&mut plan_request);
    plan_request.scenes[0]
        .annotations
        .as_mut()
        .expect("annotations")
        .visual_recipe = "hologram".to_string();
    let error = plan_request
        .validate_compilable()
        .expect_err("unknown recipe");
    assert!(error.to_string().contains("visual_recipe"), "{error}");

    let mut plan_request = request(json!({}));
    project_request(&mut plan_request);
    let annotations = plan_request.scenes[0]
        .annotations
        .as_mut()
        .expect("annotations");
    annotations.visual_recipe = "route_map".to_string();
    annotations.visual_payload = Some(json!([1, 2, 3]));
    let error = plan_request
        .validate_compilable()
        .expect_err("non-object payload");
    assert!(error.to_string().contains("visual_payload"), "{error}");
}
