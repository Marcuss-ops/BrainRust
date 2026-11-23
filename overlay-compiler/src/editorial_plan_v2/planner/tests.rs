// Unit tests for the editorial-plan v2 planner.
use super::{
    assign_long_phrase_motions, assign_short_phrase_motions, clip_items_to_scene_bounds,
    compile_plan, resolve_chain_overlaps, EditorialPlanV2Request, PlanItemV2,
};

pub(super) fn request() -> EditorialPlanV2Request {
    serde_json::from_value(serde_json::json!({
            "operation": "compile_editorial_plan",
            "schema_version": "editorial_plan.v2",
            "plan": {
                "plan_id": "plan-1", "video_id": "video-1", "project_id": "project-1",
                "motion_seed": "seed-1",
                "width": 1920, "height": 1080, "fps_num": 24, "fps_den": 1
            },
            "language": "it", "source_language": "it",
            "seal": {"audio_duration_us": 2000000, "output_profile_id": "test"},
            "canvas": {"group_limits": {},
                "entity_image_motions": ["entity_reveal_push", "entity_soft_rise"],
                "entity_caption_motions": ["caption_rise", "caption_fade"],
                "image_motion_offset": 1},
            "scenes": [{
                "id": "scene-1", "text": "Nike corre al museo.",
                "duration_ms": 2000, "execution_mode": "body",
                "timeline_start_us": 0, "duration_us": 2000000,
                "voiceover": {"timing": {"duration_us": 2000000, "words": [
                    {"index": 0, "text": "Nike", "start_us": 100000, "end_us": 400000},
                    {"index": 1, "text": "corre", "start_us": 400000, "end_us": 800000},
                    {"index": 2, "text": "al", "start_us": 800000, "end_us": 1000000},
                    {"index": 3, "text": "museo.", "start_us": 1000000, "end_us": 1300000}
                ]}}
            }],
            "editorial": {
                "schema_version": "editorial.v1", "source_language": "it",
                "profile_version": "highlights.v1", "certified": true,
                "fingerprint": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "scenes": [{
                    "scene_id": "scene-1", "title": "Nike corre",
                    "title_status": "resolved", "title_source": "extractive_keyphrase",
                    "globally_indexed": true,
                    "bullets": [{"sentence_start": 0, "sentence_end": 1, "text": "Nike corre al museo."}]
                }]
            },
            "entity_timeline": {
                "version": 1, "project_id": "project-1", "language": "it", "duration_us": 2000000,
                "scenes": [{"scene_id": "scene-1", "scene_index": 0, "timeline_start_us": 0,
                    "entities": [{
                        "entity_id": "person-tim", "name": "Tim Cook", "type": "PERSON",
                        "scene_id": "scene-1", "scene_index": 0,
                        "text_start": 0, "text_end": 8, "word_start": 0, "word_end": 2,
                        "local_start_us": 200000, "local_end_us": 1200000,
                        "timeline_start_us": 0,
                        "audio_start_us": 200000, "audio_end_us": 1200000,
                        "confidence": 0.9, "role": "primary", "canonical_entity_id": "person:tim-cook"
                    }]}]
            },
            "media": {"verified_entity_assets": [{
                "entity_id": "person-tim", "canonical_entity_id": "person:tim-cook",
                "assets": [{"asset_id": "asset-tim", "url": "https://example.test/tim.jpg",
                    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "media_type": "image/jpeg"}]
            }]},
            "budgets": {"phrase_overlays": 4, "image_overlays": 1, "map_overlays": 0,
                "max_per_scene": 3, "bullet": 1},
            "scene_windows": {"scene-1": [0, 2000000]}
        }))
        .expect("fixture")
}

fn chain_item(id: &str, start_us: i64, duration_us: i64, segments: &[(i64, i64)]) -> PlanItemV2 {
    PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        kind: "image_chain".to_string(),
        template_id: "IMAGE_CHAIN".to_string(),
        start_us,
        duration_us,
        start_ms: start_us / 1_000,
        end_ms: (start_us + duration_us + 999) / 1_000,
        params: Some(serde_json::json!({
            "chain_count": segments.len(),
            "chain_motions": segments.iter().map(|_| "image_fade_reveal").collect::<Vec<_>>(),
            "chain_segments": segments
                .iter()
                .map(|(start, duration)| serde_json::json!({
                    "kind": "entity_image",
                    "start_us": start,
                    "duration_us": duration,
                }))
                .collect::<Vec<_>>(),
        })),
        ..PlanItemV2::default()
    }
}

fn overlay_item(
    id: &str,
    kind: &str,
    template: &str,
    start_us: i64,
    duration_us: i64,
) -> PlanItemV2 {
    PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        kind: kind.to_string(),
        template_id: template.to_string(),
        start_us,
        duration_us,
        start_ms: start_us / 1_000,
        end_ms: (start_us + duration_us + 999) / 1_000,
        ..PlanItemV2::default()
    }
}

fn phrase_item(id: &str, text: &str, role: Option<&str>) -> PlanItemV2 {
    PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        kind: "text_phrase".to_string(),
        template_id: "IMPORTANT_PHRASE".to_string(),
        text: text.to_string(),
        params: role.map(|role| serde_json::json!({ "editorial_role": role })),
        ..Default::default()
    }
}

fn chain_phrase(
    id: &str,
    text: &str,
    start_us: i64,
    duration_us: i64,
    motion: &str,
) -> super::PlanItemV2 {
    super::PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        kind: "text_phrase".to_string(),
        template_id: "IMPORTANT_PHRASE".to_string(),
        preset_id: "phrase_default".to_string(),
        motion_id: motion.to_string(),
        text: text.to_string(),
        start_ms: start_us / 1000,
        end_ms: (start_us + duration_us + 999) / 1000,
        start_us,
        duration_us,
        ..Default::default()
    }
}

fn chain_image(id: &str, start_us: i64, duration_us: i64, motion: &str) -> super::PlanItemV2 {
    super::PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        kind: "image".to_string(),
        template_id: "IMAGE_OVERLAY".to_string(),
        preset_id: "image_scale_in".to_string(),
        motion_id: motion.to_string(),
        start_ms: start_us / 1000,
        end_ms: (start_us + duration_us + 999) / 1000,
        start_us,
        duration_us,
        asset_refs: vec![super::AssetRefV2 {
            asset_id: format!("asset-{id}"),
            url: format!("https://example.test/{id}.jpg"),
            sha256: "a".repeat(64),
            media_type: "image/jpeg".to_string(),
        }],
        ..Default::default()
    }
}

fn chain_request(counts: &[(&str, i64)]) -> EditorialPlanV2Request {
    let mut request = request();
    request.canvas.animation_counts = counts
        .iter()
        .map(|(key, value)| ((*key).to_string(), *value))
        .collect();
    request
}

fn chain_ends(end_us: i64) -> std::collections::BTreeMap<String, i64> {
    std::collections::BTreeMap::from([("scene-1".to_string(), end_us)])
}

fn chain_entity_image(id: &str, caption: &str, start_us: i64, duration_us: i64) -> PlanItemV2 {
    let mut item = chain_image(id, start_us, duration_us, "image_focus_reveal");
    item.kind = "entity_image".to_string();
    item.template_id = "image_popup".to_string();
    item.entity_id = format!("ent_{id}");
    item.entity_caption = caption.to_string();
    item.caption_layout = "below".to_string();
    item.entity_style_id = "premium_below_random_v1".to_string();
    item
}

include!("tests/core.rs");
include!("tests/entity_image_priority.rs");
include!("tests/phrases.rs");
include!("tests/chains.rs");
