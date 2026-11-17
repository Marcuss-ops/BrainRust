//! The v2 single-call planner. It consumes the complete request and returns a
//! sealed `FinalPlanV2`: scene candidates, editorial title/bullet selection,
//! verified entity cards, admission, entrance cascade and seal. The module
//! deliberately reuses the existing Rust ranking/cascade primitives.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    cascade_clustered_entrances, rank_entity_overlays, CascadeEntranceItem,
    CascadeEntrancesRequest, EntityRankingInput, EntityRankingScene, RankEntityOverlaysRequest,
    TimingItem,
};

use super::editorial::project_editorial_scenes;
use super::error::EditorialPlanError;
use super::request::{
    AssetRefV2, EditorialPlanV2Request, EntityOccurrenceV2, ImageCandidateV2, MapCandidateV2,
    MediaV2, SceneV2, TimedAnnotationV2,
};
use super::response::{EditorialPlanV2Compiled, FinalPlanV2, IntakeBudgetV2, IntakeV2, PlanItemV2};
use super::EDITORIAL_PLAN_V2_SCHEMA_VERSION;

mod admission;
mod candidates;
mod coalesce;
mod media;
mod presentation;

use admission::*;
use candidates::*;
use coalesce::*;
use media::*;
use presentation::*;

/// Validates the complete request, runs every phase in order and seals the
/// final plan. There is no empty-plan fallback: a missing input is an error.
pub fn compile_plan(
    request: &EditorialPlanV2Request,
) -> Result<EditorialPlanV2Compiled, EditorialPlanError> {
    // WP4: raw surfaces are projected here, before any planner phase. The
    // producer cannot pre-select, drop or time a phrase/entity/number.
    let mut projected_request = request.clone();
    super::projection::project_request(&mut projected_request);
    let request = &projected_request;
    request.validate_compilable()?;
    let mut editorial = project_editorial_scenes(request)?;
    if request.chapters.is_some() {
        // Chapter bullets own the bullet slot on the scenes they cover. They
        // are projected before the editorial bullets are admitted, exactly as
        // the Go planner orders the two phases; a scene keeps at most one
        // bullet, so the editorial bullet is dropped on covered scenes.
        let chapters = crate::project_chapter_bullets(request);
        for (scene_id, phrases) in &chapters.scene_phrases {
            editorial
                .scene_phrases
                .entry(scene_id.clone())
                .or_default()
                .extend(phrases.iter().cloned());
        }
        let covered: BTreeSet<String> = chapters.covered_scenes.iter().cloned().collect();
        for (scene_id, phrases) in editorial.scene_phrases.iter_mut() {
            if covered.contains(scene_id) {
                phrases.retain(|phrase| phrase.presentation_role != "scene_bullet");
            }
        }
        editorial.covered_scenes = chapters.covered_scenes;
    }
    let mut candidates = build_base_candidates(request, &editorial)?;
    trim_maps(request, &mut candidates);
    let recap = build_recap_candidates(request)?;
    candidates.extend(recap.items);
    // The first admission pass runs on the editorial/annotation candidates.
    // Entity cards are merged after it and then pass the same final admission,
    // exactly as the Go planner resolved cards after BuildPlan and before the
    // final budget. This keeps base items from losing slots to a later card.
    let intake = new_intake(request, &candidates);
    let (mut items, mut intake) = admit(request, candidates, intake);
    let cards = build_entity_card_candidates(request)?;
    intake.candidates += cards.len() as i64;
    for card in &cards {
        *intake
            .candidates_by_kind
            .entry(card.kind.clone())
            .or_default() += 1;
    }
    items.extend(cards);
    let (mut items, mut intake) = admit(request, items, intake);
    append_verified_intent_images(request, &mut items)?;
    assign_entity_media(request, &mut items)?;
    cascade(&mut items, request);
    // Overlap resolution is the last layout phase: it must see the final
    // admitted and cascaded set, never the pre-budget candidates. A map can
    // shorten an earlier map or shift/drop a colliding card, so items and the
    // intake accounting are re-sealed together afterwards.
    let before_overlap = items.len();
    items = resolve_overlaps(request, items);
    record_drops(&mut intake, "overlap", before_overlap, items.len());
    let scene_ends = request
        .scenes
        .iter()
        .map(|scene| {
            (
                scene.id.clone(),
                scene.timeline_start_us.saturating_add(scene.duration_us),
            )
        })
        .collect::<BTreeMap<String, i64>>();
    items = coalesce_chained_overlays(items, request, &scene_ends);
    items = fuse_captioned_entity_image_chains(items, &scene_ends);
    let before_leaks = items.len();
    items = append_light_leak_items(request, items);
    // Leaks are derived render items, not budget candidates, but the sealed
    // intake contract still requires every planned item to be accounted for.
    let added_leaks = items.len().saturating_sub(before_leaks) as i64;
    if added_leaks > 0 {
        intake.candidates += added_leaks;
        *intake
            .candidates_by_kind
            .entry("light_leak".to_string())
            .or_default() += added_leaks;
    }
    let before_presentation = items.len();
    items = apply_presentation(items, &request.plan.plan_id);
    // The long-copy split turns one admitted phrase into multiple render
    // items. They are derived items, not new candidates, but the sealed
    // intake contract still requires every planned item to be accounted for.
    let added_by_presentation = items.len().saturating_sub(before_presentation) as i64;
    if added_by_presentation > 0 {
        intake.candidates += added_by_presentation;
        *intake
            .candidates_by_kind
            .entry("text_phrase".to_string())
            .or_default() += added_by_presentation;
    }
    name_items_for_review(&mut items);
    finalize_intake(&mut intake, &items);
    let plan = seal(request, items, intake, recap.placements);
    let compiled = EditorialPlanV2Compiled {
        ok: true,
        schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
        plan,
    };
    compiled.validate_for(request)?;
    Ok(compiled)
}

fn seal(
    request: &EditorialPlanV2Request,
    mut items: Vec<PlanItemV2>,
    intake: IntakeV2,
    recap_placements: i64,
) -> FinalPlanV2 {
    // Render keys are derived state, never planner input. The seal clears
    // them; Go recomputes a SHA-256 key from the finished document.
    for item in &mut items {
        item.render_key.clear();
    }
    let seal = request.seal.as_ref().expect("validated seal");
    let scene_end_ms = request
        .scenes
        .iter()
        .map(|scene| (scene.timeline_start_us + scene.duration_us) / 1000)
        .max()
        .unwrap_or(0);
    let duration_ms = (seal.audio_duration_us / 1000).max(scene_end_ms);
    let recap_cards = count_marked_items(&items, "recap");
    let chart_cards = count_marked_items(&items, "chart");
    let chart_placements = count_chart_placements(request);
    FinalPlanV2 {
        schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
        plan_id: request.plan.plan_id.clone(),
        video_id: request.plan.video_id.clone(),
        project_id: request.plan.project_id.clone(),
        language: request.language.clone(),
        width: request.plan.width,
        height: request.plan.height,
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        duration_ms,
        background: seal.background.clone(),
        items,
        recap_placements,
        recap_cards,
        chart_placements,
        chart_cards,
        timed_scenes: request
            .scenes
            .iter()
            .filter(|scene| scene.duration_us > 0)
            .count() as i64,
        intake,
        output_profile_id: seal.output_profile_id.clone(),
    }
}

/// Strict verbatim numeric label parser, mirroring `overlays.ParseChartValue`:
/// digits with at most one decimal separator and one optional trailing
/// percentage/currency sign. Anything else (number words, embedded digits,
/// "42 per cento") is rejected instead of coerced.
fn parse_chart_value(text: &str) -> Option<f64> {
    let mut value = text.trim();
    if value.is_empty() {
        return None;
    }
    value = value.trim_matches(|ch| matches!(ch, '%' | '€' | '$' | '£' | '¥'));
    let compact = value.trim().replace(' ', "");
    if compact.is_empty() {
        return None;
    }
    let numeric = if compact.matches(',').count() == 1 && !compact.contains('.') {
        compact.replace(',', ".")
    } else {
        compact.replace(',', "")
    };
    if numeric.is_empty() || numeric == "-" || numeric == "." || numeric == "-." {
        return None;
    }
    let mut digits = 0usize;
    let mut separator = false;
    for (index, ch) in numeric.char_indices() {
        match ch {
            '0'..='9' => digits += 1,
            '.' if !separator && index > 0 => separator = true,
            '-' if index == 0 => {}
            _ => return None,
        }
    }
    if digits == 0 {
        return None;
    }
    let parsed: f64 = numeric.parse().ok()?;
    (!parsed.is_nan() && !parsed.is_infinite()).then_some(parsed)
}

/// Counts the grounded numeric comparisons the chart lane can render, with the
/// same rules as `ChartPlacementsForScenes`: 2..6 strict values in one scene
/// and a window of at least two seconds. The items themselves are generated
/// by the chart lane; this is the placement count the sealed plan reports.
fn count_chart_placements(request: &EditorialPlanV2Request) -> i64 {
    let mut placements = 0_i64;
    for scene in &request.scenes {
        let Some(annotations) = scene.annotations.as_ref() else {
            continue;
        };
        let numbers = if annotations.chart_numbers.is_empty() {
            &annotations.numbers
        } else {
            &annotations.chart_numbers
        };
        let mut windows: Vec<(i64, i64)> = Vec::new();
        for number in numbers {
            if number.text.trim().is_empty() || number.start_us < 0 || number.duration_us <= 0 {
                continue;
            }
            if parse_chart_value(&number.text).is_none() {
                continue;
            }
            windows.push((
                number.start_us,
                number.start_us.saturating_add(number.duration_us),
            ));
            if windows.len() >= 6 {
                break;
            }
        }
        if windows.len() < 2 {
            continue;
        }
        let start = windows.iter().map(|window| window.0).min().unwrap_or(0);
        let end = windows.iter().map(|window| window.1).max().unwrap_or(0);
        if end > start && end - start >= 2_000_000 {
            placements += 1;
        }
    }
    placements
}

#[cfg(test)]
mod tests {
    use super::{compile_plan, EditorialPlanV2Request};

    fn request() -> EditorialPlanV2Request {
        serde_json::from_value(serde_json::json!({
            "operation": "compile_editorial_plan",
            "schema_version": "editorial_plan.v2",
            "plan": {
                "plan_id": "plan-1", "video_id": "video-1", "project_id": "project-1",
                "motion_seed": "seed-1", "coherence_seed": "",
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

    fn captioned_card(id: &str, start_us: i64, duration_us: i64) -> super::PlanItemV2 {
        super::PlanItemV2 {
            id: id.to_string(),
            scene_id: "s1".to_string(),
            kind: "entity_card".to_string(),
            entity_caption: "Tim Cook".to_string(),
            start_us,
            duration_us,
            start_ms: start_us / 1000,
            end_ms: (start_us + duration_us + 999) / 1000,
            asset_refs: vec![super::AssetRefV2 {
                asset_id: format!("asset-{id}"),
                url: format!("https://example.test/{id}.jpg"),
                sha256: "b".repeat(64),
                media_type: "image/jpeg".to_string(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn captioned_entity_cards_fuse_into_a_mixed_chain() {
        let ends = std::collections::BTreeMap::from([("s1".to_string(), 10_000_000_i64)]);
        let fused = super::fuse_captioned_entity_image_chains(
            vec![
                captioned_card("a", 0, 1_000_000),
                captioned_card("b", 1_500_000, 1_000_000),
            ],
            &ends,
        );
        assert_eq!(1, fused.len());
        assert_eq!("mixed_chain", fused[0].kind);
        assert_eq!("MIXED_CHAIN", fused[0].template_id);
        assert_eq!(2, fused[0].asset_refs.len());
        assert_eq!("asset-a", fused[0].asset_refs[0].asset_id);
        let segments = fused[0]
            .params
            .as_ref()
            .and_then(|params| params.get("chain_segments"))
            .and_then(serde_json::Value::as_array)
            .expect("chain segments");
        assert_eq!(2, segments.len());
        assert_eq!(0, segments[0]["start_us"].as_i64().unwrap());
        assert_eq!(2_500_000, segments[0]["duration_us"].as_i64().unwrap());
        assert_eq!(2_900_000, segments[1]["start_us"].as_i64().unwrap());
        assert_eq!("Tim Cook", segments[1]["entity_caption"].as_str().unwrap());
    }

    #[test]
    fn captioned_entity_cards_outside_the_gap_stay_standalone() {
        let ends = std::collections::BTreeMap::from([("s1".to_string(), 20_000_000_i64)]);
        let fused = super::fuse_captioned_entity_image_chains(
            vec![
                captioned_card("a", 0, 1_000_000),
                captioned_card("b", 10_000_000, 1_000_000),
            ],
            &ends,
        );
        assert_eq!(2, fused.len());
        assert!(fused.iter().all(|item| item.kind == "entity_card"));
    }

    #[test]
    fn captioned_entity_chain_never_crosses_the_scene_end() {
        let ends = std::collections::BTreeMap::from([("s1".to_string(), 4_000_000_i64)]);
        let fused = super::fuse_captioned_entity_image_chains(
            vec![
                captioned_card("a", 0, 1_000_000),
                captioned_card("b", 1_500_000, 1_000_000),
            ],
            &ends,
        );
        assert_eq!(
            2,
            fused.len(),
            "a chain that would cross the scene stays standalone"
        );
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
        first.motion_params =
            Some(serde_json::json!({"enter_frames": 17, "vendor": "payload-owned"}));
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
    fn general_chain_keeps_overflowing_runs_standalone() {
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
        assert_eq!(21_200_000, super::projected_chain_end_us(&items));
        assert!(super::chain_exceeds_scene_end(&items, Some(20_000_000)));
        let overflow = super::coalesce_chained_overlays(
            items.clone(),
            &chain_request(&[("chain_enabled", 1)]),
            &chain_ends(20_000_000),
        );
        assert_eq!(4, overflow.len());
        assert!(overflow.iter().all(|item| item.kind == "image"));

        let room = super::coalesce_chained_overlays(
            items,
            &chain_request(&[("chain_enabled", 1)]),
            &chain_ends(40_000_000),
        );
        assert_eq!(1, room.len());
        assert_eq!("image_chain", room[0].kind);
        assert!(room[0].window_us().1 <= 40_000_000);
        assert_eq!(4, room[0].asset_refs.len());
        let segments = room[0].params.as_ref().unwrap()["chain_segments"]
            .as_array()
            .unwrap();
        assert_eq!(5_000_000, segments[0]["duration_us"].as_i64().unwrap());
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
        let mut metric_b =
            chain_phrase("metric-1", "18", 2_500_000, 2_000_000, "metric_slide_left");
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
    fn general_chain_attaches_resolved_entity_portraits_before_the_gate() {
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
        assert_eq!(1, card_out.asset_refs.len());
        assert_eq!("portrait-ada", card_out.asset_refs[0].asset_id);

        let chained = super::coalesce_chained_overlays(
            vec![image, card, phrase],
            &chain_request(&[("chain_enabled", 1)]),
            &chain_ends(30_000_000),
        );
        assert_eq!(2, chained.len());
        let mixed = chained
            .iter()
            .find(|item| item.kind == "mixed_chain")
            .expect("mixed chain");
        let segments = mixed.params.as_ref().unwrap()["chain_segments"]
            .as_array()
            .unwrap();
        assert_eq!("ent_ada", segments[0]["entity_id"].as_str().unwrap());
        assert_eq!(
            "portrait-ada",
            segments[0]["assets"][0]["asset_id"].as_str().unwrap()
        );
    }
}
