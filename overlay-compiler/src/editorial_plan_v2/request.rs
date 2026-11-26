//! The complete `editorial_plan.v2` request: everything the Rust planner needs
//! to make every overlay decision for a run.
//!
//! The envelope is additive. Fields carried by the first landed slice keep
//! their meaning; the sections added for TICKET-14 WP1 are the planner's real
//! inputs (seal extent, scene annotations, entity timeline, media refs, run
//! budgets and canonical scene windows). Nothing here is a precomputed
//! selection: `media` holds only materialized/verified refs and certified plate
//! or image candidates, and `annotations` holds certified semantic surfaces,
//! never the admitted item list.
//!
//! Request JSON is replayable: no pointers, no filesystem paths, no logger
//! state. Fields the planner genuinely treats as opaque render metadata
//! (`background`, `animation_directives`) stay `serde_json::Value`.

use std::collections::BTreeSet;

use super::error::EditorialPlanError;
use super::{
    CHAPTER_MANIFEST_SCHEMA_VERSION, EDITORIAL_MANIFEST_SCHEMA_VERSION, EDITORIAL_PLAN_OPERATION,
    EDITORIAL_PLAN_V2_SCHEMA_VERSION, EDITORIAL_PROFILE_VERSION,
};

const OVERLAY_CATEGORIES: [&str; 11] = [
    "phrases",
    "images",
    "numbers",
    "quotes",
    "brand_text",
    "maps",
    "charts",
    "recaps",
    "titles",
    "bullets",
    "entities",
];

mod types;
pub use types::*;
mod validation;
pub use validation::is_lower_hex_64;

#[cfg(test)]
mod tests {
    use super::{
        BudgetsV2, EditorialBulletV2, EditorialManifestV2, EditorialPlanV2Request,
        EditorialSceneV2, EntityTimelineV2, ImageCandidateV2, PlanIdentityV2, SceneV2, SealV2,
        TimingV2, VoiceoverV2, WordTimingV2, EDITORIAL_MANIFEST_SCHEMA_VERSION,
        EDITORIAL_PLAN_V2_SCHEMA_VERSION, EDITORIAL_PROFILE_VERSION,
    };
    use std::collections::BTreeMap;

    fn plan() -> PlanIdentityV2 {
        PlanIdentityV2 {
            plan_id: "plan-1".to_string(),
            video_id: "video-1".to_string(),
            project_id: "project-1".to_string(),
            motion_seed: "seed-1".to_string(),
            width: 1920,
            height: 1080,
            fps_num: 24,
            fps_den: 1,
        }
    }

    fn timing() -> TimingV2 {
        TimingV2 {
            duration_us: 2_000_000,
            words: vec![
                WordTimingV2 {
                    index: 0,
                    text: "Nike".to_string(),
                    start_us: 100_000,
                    end_us: 400_000,
                },
                WordTimingV2 {
                    index: 1,
                    text: "corre".to_string(),
                    start_us: 400_000,
                    end_us: 800_000,
                },
            ],
        }
    }

    fn scene() -> SceneV2 {
        SceneV2 {
            id: "scene-1".to_string(),
            text: "Nike corre. La statua è al museo.".to_string(),
            duration_ms: 2_000,
            execution_mode: "body".to_string(),
            short_phrase_overlay: false,
            overlay_categories: BTreeMap::new(),
            index: 0,
            voiceover: Some(VoiceoverV2 {
                timing: Some(timing()),
            }),
            timeline_start_us: 0,
            duration_us: 2_000_000,
            annotations: None,
            raw: None,
        }
    }

    fn editorial() -> EditorialManifestV2 {
        EditorialManifestV2 {
            schema_version: EDITORIAL_MANIFEST_SCHEMA_VERSION.to_string(),
            source_language: "it".to_string(),
            profile_version: EDITORIAL_PROFILE_VERSION.to_string(),
            certified: true,
            fingerprint: "a".repeat(64),
            scenes: vec![EditorialSceneV2 {
                scene_id: "scene-1".to_string(),
                title: Some("Nike corre".to_string()),
                title_status: "resolved".to_string(),
                title_source: "sentence".to_string(),
                globally_indexed: false,
                bullets: vec![EditorialBulletV2 {
                    sentence_start: 0,
                    sentence_end: 1,
                    text: "Nike corre.".to_string(),
                }],
            }],
        }
    }

    fn request() -> EditorialPlanV2Request {
        EditorialPlanV2Request {
            operation: "compile_editorial_plan".to_string(),
            schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
            plan: plan(),
            language: "it".to_string(),
            source_language: "it".to_string(),
            seal: Some(SealV2 {
                audio_duration_us: 2_000_000,
                output_profile_id: "assemblyready.alpha.v1".to_string(),
                background: None,
            }),
            canvas: Default::default(),
            editorial: Some(editorial()),
            chapters: None,
            heavy_sentences: Vec::new(),
            numeric_lexicon: None,
            image_plan: None,
            clips: Vec::new(),
            recap_briefs: Vec::new(),
            scenes: vec![scene()],
            entity_timeline: Some(EntityTimelineV2 {
                version: 1,
                project_id: "project-1".to_string(),
                language: "it".to_string(),
                duration_us: 2_000_000,
                scenes: Vec::new(),
            }),
            media: Some(Default::default()),
            budgets: Some(BudgetsV2 {
                phrase_overlays: 8,
                image_overlays: 5,
                map_overlays: 3,
                max_per_scene: 3,
                short_phrase: 3,
                long_phrase: 5,
                bullet: 2,
            }),
            scene_windows: BTreeMap::from([("scene-1".to_string(), [0, 2_000_000])]),
        }
    }

    #[test]
    fn v2_request_round_trips_through_json() {
        let request = request();
        request.validate().expect("valid request");
        let encoded = serde_json::to_string(&request).expect("encode request");
        let decoded: EditorialPlanV2Request =
            serde_json::from_str(&encoded).expect("decode request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn v2_rejects_unknown_schema_version() {
        let mut request = request();
        request.schema_version = "editorial_plan.v1".to_string();
        assert!(matches!(
            request.validate().expect_err("unknown schema"),
            super::super::EditorialPlanError::UnsupportedSchemaVersion { .. }
        ));
    }

    #[test]
    fn v2_rejects_duplicate_scene_ids() {
        let mut request = request();
        request.scenes.push(request.scenes[0].clone());
        assert!(request
            .validate()
            .expect_err("duplicate scene")
            .to_string()
            .contains("duplicate scene id"));
    }

    #[test]
    fn v2_rejects_non_monotonic_word_timing() {
        let mut request = request();
        let timing = request.scenes[0]
            .voiceover
            .as_mut()
            .expect("voiceover")
            .timing
            .as_mut()
            .expect("timing");
        timing.words[1].start_us = 50_000;
        assert!(request
            .validate()
            .expect_err("non-monotonic")
            .to_string()
            .contains("not monotonic"));
    }

    #[test]
    fn v2_rejects_malformed_editorial_fingerprint() {
        let mut request = request();
        request.editorial.as_mut().expect("editorial").fingerprint = "A".repeat(64);
        assert!(request
            .validate()
            .expect_err("fingerprint")
            .to_string()
            .contains("64 lowercase hex"));
    }

    #[test]
    fn v2_rejects_invalid_overlay_category() {
        let mut request = request();
        request
            .canvas
            .overlay_categories
            .insert("unknown".to_string(), "enabled".to_string());
        assert!(request
            .validate()
            .expect_err("category")
            .to_string()
            .contains("unknown overlay category"));
    }

    #[test]
    fn v2_date_motion_family_accepts_aliases_and_rejects_unknown() {
        let mut request = request();
        for selector in [
            "auto",
            "classic",
            "date_v1",
            "didone",
            "date_didone_v1",
            "typewriter",
            "date_typewriter_v1",
        ] {
            request.canvas.date_motion_family = selector.to_string();
            request
                .validate()
                .unwrap_or_else(|error| panic!("{selector}: {error}"));
        }
        request.canvas.date_motion_family = "didone_v2".to_string();
        assert!(request
            .validate()
            .expect_err("unknown family")
            .to_string()
            .contains("date_motion_family"));
    }

    #[test]
    fn v2_compilable_requires_every_input_section() {
        for missing in ["seal", "media", "budgets", "entity_timeline"] {
            let mut request = request();
            match missing {
                "seal" => request.seal = None,
                "media" => request.media = None,
                "budgets" => request.budgets = None,
                "entity_timeline" => request.entity_timeline = None,
                _ => unreachable!(),
            }
            assert!(
                request
                    .validate_compilable()
                    .expect_err("missing section")
                    .to_string()
                    .contains(missing),
                "{missing} must be required"
            );
        }
    }

    #[test]
    fn v2_rejects_unmaterialized_semantic_asset() {
        let mut request = request();
        request.media = Some(super::MediaV2 {
            materialized_assets: vec![super::AssetRefV2 {
                asset_id: "asset-1".to_string(),
                url: "semantic/hero.png".to_string(),
                sha256: "b".repeat(64),
                media_type: "image/png".to_string(),
            }],
            ..Default::default()
        });
        assert!(request
            .validate()
            .expect_err("placeholder")
            .to_string()
            .contains("unmaterialized semantic placeholder"));
    }

    #[test]
    fn v2_rejects_scene_window_mismatch() {
        let mut request = request();
        request
            .scene_windows
            .insert("scene-1".to_string(), [0, 1_000_000]);
        assert!(request
            .validate_compilable()
            .expect_err("window")
            .to_string()
            .contains("does not match the scene duration"));
    }

    #[test]
    fn v2_rejects_entity_occurrence_with_a_foreign_timeline_offset() {
        let mut request = request();
        let occurrence_scene = super::SceneEntityTimelineV2 {
            scene_id: "scene-1".to_string(),
            scene_index: 0,
            timeline_start_us: 0,
            voiceover_asset_id: String::new(),
            entities: vec![super::EntityOccurrenceV2 {
                entity_id: "person:nike".to_string(),
                name: "Nike".to_string(),
                entity_type: "ORG".to_string(),
                scene_id: "scene-1".to_string(),
                scene_index: 0,
                text_start: 0,
                text_end: 4,
                word_start: 0,
                word_end: 0,
                local_start_us: 100_000,
                local_end_us: 400_000,
                timeline_start_us: 0,
                audio_start_us: 100_000,
                audio_end_us: 400_000,
                confidence: 0.9,
                role: "named_entity".to_string(),
                canonical_entity_id: "org:nike".to_string(),
            }],
        };
        request
            .entity_timeline
            .as_mut()
            .expect("timeline")
            .scenes
            .push(occurrence_scene);
        request.validate().expect("canonical mapping is valid");
        request.entity_timeline.as_mut().expect("timeline").scenes[0].entities[0]
            .timeline_start_us = 500_000;
        assert!(request
            .validate()
            .expect_err("offset drift")
            .to_string()
            .contains("different timeline offset"));
    }

    #[test]
    fn v2_rejects_out_of_range_map_coordinates() {
        let mut request = request();
        request.scenes[0].annotations = Some(super::SceneAnnotationsV2 {
            maps: vec![super::MapCandidateV2 {
                entity_id: "place:paris".to_string(),
                label: "Paris".to_string(),
                scope: "city".to_string(),
                latitude: 91.0,
                longitude: 2.35,
                start_ms: 0,
                end_ms: 1_000,
                start_us: 0,
                duration_us: 1_000_000,
                score: 0.9,
            }],
            ..Default::default()
        });
        assert!(request
            .validate()
            .expect_err("latitude")
            .to_string()
            .contains("WGS84"));
    }

    #[test]
    fn v2_rejects_a_negative_image_window() {
        let mut request = request();
        request.scenes[0].annotations = Some(super::SceneAnnotationsV2 {
            images: vec![ImageCandidateV2 {
                asset_id: "asset-1".to_string(),
                url: "https://cdn.example/hero.png".to_string(),
                sha256: "c".repeat(64),
                media_type: "image/png".to_string(),
                start_ms: 0,
                end_ms: 1_000,
                start_us: -1,
                duration_us: 1_000_000,
                score: 0.8,
            }],
            ..Default::default()
        });
        assert!(request
            .validate()
            .expect_err("negative window")
            .to_string()
            .contains("negative image window"));
    }

    #[test]
    fn entity_badge_color_is_optional_and_requires_rgb_hex() {
        let mut request = request();
        request.canvas.entity_badge_color = "#336699".to_string();
        request.validate().expect("valid payload color");
        request.canvas.entity_badge_color = "blue".to_string();
        assert!(request
            .validate()
            .expect_err("non-hex color must fail closed")
            .to_string()
            .contains("canvas.entity_badge_color"));
    }
}
