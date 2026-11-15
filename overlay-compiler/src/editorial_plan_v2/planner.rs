//! The v2 single-call planner. It consumes the complete request and returns a
//! sealed `FinalPlanV2`: scene candidates, editorial title/bullet selection,
//! verified entity cards, admission, entrance cascade and seal. The module
//! deliberately reuses the existing Rust ranking/cascade primitives.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

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

/// Validates the complete request, runs every phase in order and seals the
/// final plan. There is no empty-plan fallback: a missing input is an error.
pub fn compile_plan(
    request: &EditorialPlanV2Request,
) -> Result<EditorialPlanV2Compiled, EditorialPlanError> {
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
    let mut candidates = build_candidates(request, &editorial)?;
    trim_maps(request, &mut candidates);
    let (mut items, mut intake) = admit(request, candidates);
    cascade(&mut items, request);
    // Overlap resolution is the last layout phase: it must see the final
    // admitted and cascaded set, never the pre-budget candidates. A map can
    // shorten an earlier map or shift/drop a colliding card, so items and the
    // intake accounting are re-sealed together afterwards.
    let before_overlap = items.len();
    items = resolve_overlaps(request, items);
    record_drops(&mut intake, "overlap", before_overlap, items.len());
    finalize_intake(&mut intake, &items);
    let plan = seal(request, items, intake);
    let compiled = EditorialPlanV2Compiled {
        ok: true,
        schema_version: EDITORIAL_PLAN_V2_SCHEMA_VERSION.to_string(),
        plan,
    };
    compiled.validate_for(request)?;
    Ok(compiled)
}

fn build_candidates(
    request: &EditorialPlanV2Request,
    editorial: &super::response::EditorialPlanV2Response,
) -> Result<Vec<PlanItemV2>, EditorialPlanError> {
    let mut items = Vec::new();
    for scene in &request.scenes {
        if scene.duration_us <= 0 {
            continue;
        }
        append_editorial(&mut items, scene, editorial);
        if let Some(annotations) = &scene.annotations {
            append_annotations(&mut items, scene, annotations);
        }
    }
    append_entity_cards(&mut items, request)?;
    Ok(items)
}

fn append_editorial(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    editorial: &super::response::EditorialPlanV2Response,
) {
    let Some(phrases) = editorial.scene_phrases.get(&scene.id) else {
        return;
    };
    let title = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "scene_title");
    let bullet = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "scene_bullet");
    let chapter = phrases
        .iter()
        .find(|phrase| phrase.presentation_role == "chapter_bullet");
    for (ordinal, phrase) in [title, bullet, chapter].into_iter().flatten().enumerate() {
        let role = phrase.presentation_role.as_str();
        let mut item = scene_item(
            format!("{}-editorial-{ordinal}", scene.id),
            scene,
            "text_phrase",
            "IMPORTANT_PHRASE",
            phrase.start_us,
            phrase.duration_us,
            phrase.text.clone(),
        );
        item.params = Some(json!({
            "editorial_role": role,
            "editorial_candidate": phrase.candidate,
            "priority": phrase.score,
        }));
        items.push(item);
    }
}

fn append_annotations(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    annotations: &super::request::SceneAnnotationsV2,
) {
    for (index, phrase) in annotations.phrases.iter().enumerate() {
        if !phrase.presentation_role.is_empty() {
            continue; // editorial projection owns title/bullet candidates
        }
        items.push(text_item(
            scene,
            "phrase",
            index,
            phrase,
            "text_phrase",
            "IMPORTANT_PHRASE",
        ));
    }
    for (index, keyword) in annotations.keywords.iter().enumerate() {
        items.push(text_item(
            scene,
            "word",
            index,
            keyword,
            "important_word",
            "IMPORTANT_WORD",
        ));
    }
    for (index, quote) in annotations.quotes.iter().enumerate() {
        items.push(text_item(scene, "quote", index, quote, "quote", "QUOTE"));
    }
    for (index, brand) in annotations.brand_texts.iter().enumerate() {
        items.push(text_item(
            scene,
            "brand",
            index,
            brand,
            "brand_text",
            "logo_default",
        ));
    }
    for (index, number) in annotations.numbers.iter().enumerate() {
        items.push(text_item(
            scene, "number", index, number, "number", "NUMBER",
        ));
    }
    for (index, number) in annotations.chart_numbers.iter().enumerate() {
        let mut item = text_item(scene, "chart-number", index, number, "number", "NUMBER");
        if let Some(params) = item.params.as_mut().and_then(Value::as_object_mut) {
            params.insert("chart".to_string(), json!(true));
        }
        items.push(item);
    }
    for (index, image) in annotations.images.iter().enumerate() {
        append_image(items, scene, "image", index, image, "IMAGE_OVERLAY");
    }
    for (index, product) in annotations.products.iter().enumerate() {
        append_image(items, scene, "product", index, product, "PRODUCT");
    }
    for (index, logo) in annotations.logos.iter().enumerate() {
        append_image(items, scene, "logo", index, logo, "LOGO");
    }
    for (index, map) in annotations.maps.iter().enumerate() {
        append_map(items, scene, index, map);
    }
}

fn text_item(
    scene: &SceneV2,
    lane: &str,
    index: usize,
    annotation: &TimedAnnotationV2,
    kind: &str,
    template: &str,
) -> PlanItemV2 {
    let mut item = scene_item(
        format!("{}-{lane}-{index}", scene.id),
        scene,
        kind,
        template,
        annotation.start_us,
        annotation.duration_us,
        annotation.text.clone(),
    );
    item.params = Some(json!({"priority": annotation.score}));
    item
}

fn append_image(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    kind: &str,
    index: usize,
    candidate: &ImageCandidateV2,
    template: &str,
) {
    if candidate.asset_id.trim().is_empty() {
        return;
    }
    let mut item = scene_item(
        format!("{}-{kind}-{index}", scene.id),
        scene,
        kind,
        template,
        candidate.start_us,
        candidate.duration_us,
        String::new(),
    );
    item.asset_refs = vec![AssetRefV2 {
        asset_id: candidate.asset_id.clone(),
        url: candidate.url.clone(),
        sha256: candidate.sha256.clone(),
        media_type: candidate.media_type.clone(),
    }];
    item.params = Some(json!({"priority": candidate.score}));
    items.push(item);
}

fn append_map(
    items: &mut Vec<PlanItemV2>,
    scene: &SceneV2,
    index: usize,
    candidate: &MapCandidateV2,
) {
    let mut item = scene_item(
        format!("{}-map-{index}", scene.id),
        scene,
        "map",
        "MAP",
        candidate.start_us,
        candidate.duration_us,
        String::new(),
    );
    item.map = Some(json!({
        "entity_id": candidate.entity_id,
        "label": candidate.label,
        "scope": candidate.scope,
        "latitude": candidate.latitude,
        "longitude": candidate.longitude,
    }));
    item.params = Some(json!({"priority": candidate.score}));
    items.push(item);
}

fn append_entity_cards(
    items: &mut Vec<PlanItemV2>,
    request: &EditorialPlanV2Request,
) -> Result<(), EditorialPlanError> {
    let timeline = request
        .entity_timeline
        .as_ref()
        .ok_or(EditorialPlanError::MissingSection("entity_timeline"))?;
    let media = request
        .media
        .as_ref()
        .ok_or(EditorialPlanError::MissingSection("media"))?;
    let ranking_scenes: Vec<EntityRankingScene> = timeline
        .scenes
        .iter()
        .map(|scene| EntityRankingScene {
            entities: scene
                .entities
                .iter()
                .map(|entity| EntityRankingInput {
                    entity_id: entity.entity_id.clone(),
                    overlay_id: entity_overlay_id(&scene.scene_id, entity),
                    entity_type: entity.entity_type.clone(),
                    confidence: entity.confidence,
                })
                .collect(),
        })
        .collect();
    let selected = rank_entity_overlays(RankEntityOverlaysRequest {
        scenes: ranking_scenes,
    });
    for (scene_index, entity_scene) in timeline.scenes.iter().enumerate() {
        let Some(scene) = request
            .scenes
            .iter()
            .find(|scene| scene.id == entity_scene.scene_id)
        else {
            continue;
        };
        let Some(indices) = selected.get(scene_index) else {
            continue;
        };
        let mut text_cards = 0;
        for &entity_index in indices {
            let Some(entity) = entity_scene.entities.get(entity_index) else {
                continue;
            };
            let Some((kind, template)) = entity_card_kind(&entity.entity_type) else {
                continue;
            };
            if scene_has_map_for(scene, entity) {
                // A grounded place already renders as a map pin. Emitting a
                // location card as well would render the same entity twice.
                continue;
            }
            let assets = verified_entity_assets(media, entity);
            if assets.is_empty() {
                if text_cards >= 2 {
                    continue;
                }
                text_cards += 1;
            }
            let Some((start_us, duration_us)) = entity_window(scene, entity) else {
                continue;
            };
            let name = entity_name(entity);
            let mut item = scene_item(
                entity_overlay_id(&entity_scene.scene_id, entity),
                scene,
                if assets.is_empty() {
                    kind
                } else {
                    "entity_image"
                },
                template,
                start_us,
                duration_us,
                if assets.is_empty() {
                    name.clone()
                } else {
                    String::new()
                },
            );
            if !assets.is_empty() {
                item.entity_caption = name.clone();
                item.asset_refs = assets;
            }
            item.entity_id = entity.entity_id.clone();
            item.params = Some(json!({
                "entity_type": entity.entity_type,
                "canonical_entity_id": entity.canonical_entity_id,
                "priority": entity.confidence,
            }));
            items.push(item);
        }
    }
    Ok(())
}

fn scene_has_map_for(scene: &SceneV2, entity: &EntityOccurrenceV2) -> bool {
    scene
        .annotations
        .as_ref()
        .map(|annotations| {
            annotations.maps.iter().any(|map| {
                map.entity_id == entity.entity_id
                    || (!entity.canonical_entity_id.is_empty()
                        && map.entity_id == entity.canonical_entity_id)
                    || (!map.label.is_empty()
                        && map.label.eq_ignore_ascii_case(&entity.name))
            })
        })
        .unwrap_or(false)
}

fn entity_window(scene: &SceneV2, entity: &EntityOccurrenceV2) -> Option<(i64, i64)> {
    let scene_start = scene.timeline_start_us;
    let scene_end = scene_start.saturating_add(scene.duration_us);
    let start = if entity.timeline_start_us > 0 {
        entity.timeline_start_us
    } else if entity.audio_start_us > 0 {
        entity.audio_start_us
    } else {
        scene_start.saturating_add(entity.local_start_us.max(0))
    }
    .max(scene_start);
    let end = if entity.audio_end_us > entity.audio_start_us {
        entity.audio_end_us
    } else {
        start.saturating_add(2_000_000)
    }
    .min(scene_end);
    (end > start).then_some((start, end - start))
}

fn verified_entity_assets(media: &MediaV2, entity: &EntityOccurrenceV2) -> Vec<AssetRefV2> {
    for verified in &media.verified_entity_assets {
        let id_matches = !verified.entity_id.is_empty() && verified.entity_id == entity.entity_id;
        let canonical_matches = !verified.canonical_entity_id.is_empty()
            && verified.canonical_entity_id == entity.canonical_entity_id;
        if id_matches || canonical_matches {
            return verified.assets.clone();
        }
    }
    Vec::new()
}

fn entity_card_kind(entity_type: &str) -> Option<(&'static str, &'static str)> {
    match entity_type.trim().to_ascii_uppercase().as_str() {
        "PERSON" => Some(("entity_card", "person_default")),
        "ORG" | "ORGANIZATION" => Some(("organization", "org_default")),
        "GPE" | "PLACE" | "LOCATION" => Some(("location", "gpe_default")),
        "CONCEPT" | "EVENT" => Some(("concept", "concept_default")),
        _ => None,
    }
}

fn entity_name(entity: &EntityOccurrenceV2) -> String {
    if !entity.name.trim().is_empty() {
        entity.name.trim().to_string()
    } else {
        entity
            .canonical_entity_id
            .rsplit(':')
            .next()
            .unwrap_or("")
            .replace('-', " ")
    }
}

fn entity_overlay_id(scene_id: &str, entity: &EntityOccurrenceV2) -> String {
    format!("overlay-{}-{}", scene_id.trim(), entity.entity_id.trim())
}

fn scene_item(
    id: String,
    scene: &SceneV2,
    kind: &str,
    template: &str,
    start_us: i64,
    duration_us: i64,
    text: String,
) -> PlanItemV2 {
    let start_us = start_us.max(scene.timeline_start_us);
    let end_us = start_us
        .saturating_add(duration_us.max(0))
        .min(scene.timeline_start_us.saturating_add(scene.duration_us));
    let duration_us = (end_us - start_us).max(0);
    PlanItemV2 {
        id,
        scene_id: scene.id.clone(),
        kind: kind.to_string(),
        template_id: template.to_string(),
        start_us,
        duration_us,
        start_ms: start_us / 1000,
        end_ms: (start_us + duration_us + 999) / 1000,
        text,
        ..Default::default()
    }
}

fn trim_maps(request: &EditorialPlanV2Request, items: &mut [PlanItemV2]) {
    if !items.iter().any(|item| item.map.is_some()) {
        return;
    }
    let input: Vec<TimingItem> = items.iter().map(timing_item).collect();
    let edits = crate::trim_map_windows_before_budget(crate::ResolveMapOverlapsRequest {
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        scene_windows: request.scene_windows.clone(),
        items: input,
    });
    for edit in edits {
        if let Some(item) = items.get_mut(edit.index) {
            item.start_us = edit.start_us;
            item.duration_us = edit.duration_us;
            item.start_ms = edit.start_ms;
            item.end_ms = edit.end_ms;
        }
    }
}

fn timing_item(item: &PlanItemV2) -> TimingItem {
    TimingItem {
        id: item.id.clone(),
        scene_id: item.scene_id.clone(),
        kind: item.kind.clone(),
        template_id: item.template_id.clone(),
        start_ms: item.start_ms,
        end_ms: item.end_ms,
        start_us: item.start_us,
        duration_us: item.duration_us,
        has_map: item.map.is_some(),
        params: item.params.clone(),
    }
}

fn admit(
    request: &EditorialPlanV2Request,
    candidates: Vec<PlanItemV2>,
) -> (Vec<PlanItemV2>, IntakeV2) {
    let budgets = request.budgets.as_ref().expect("validated budgets");
    let mut intake = IntakeV2 {
        scenes: request.scenes.len() as i64,
        timed_scenes: request
            .scenes
            .iter()
            .filter(|scene| scene.duration_us > 0)
            .count() as i64,
        budget: IntakeBudgetV2 {
            phrase_overlays: budgets.phrase_overlays,
            image_overlays: budgets.image_overlays,
            map_overlays: budgets.map_overlays,
            max_overlays_per_scene: budgets.max_per_scene,
        },
        candidates: candidates.len() as i64,
        ..Default::default()
    };
    for item in &candidates {
        *intake
            .candidates_by_kind
            .entry(item.kind.clone())
            .or_default() += 1;
    }
    let max_per_scene = if budgets.max_per_scene > 0 {
        budgets.max_per_scene
    } else {
        request.canvas.max_overlays_per_scene
    };
    let mut group_counts: BTreeMap<&'static str, i64> = BTreeMap::new();
    let mut scene_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut phrases = 0;
    let mut images = 0;
    let mut maps = 0;
    let mut bullets = 0;
    let mut selected = Vec::new();
    for item in candidates {
        if request.canvas.maps_only && !is_map(&item) {
            drop_reason(&mut intake, "maps_only");
            continue;
        }
        if category_disabled(request, &item) {
            drop_reason(&mut intake, "category");
            continue;
        }
        let group = group_for(&item);
        if request.canvas.group_limits.get(group).is_some_and(|limit| {
            *limit > 0 && group_counts.get(group).copied().unwrap_or(0) >= *limit
        }) {
            drop_reason(&mut intake, "group");
            continue;
        }
        if item.kind == "text_phrase" {
            if budgets.phrase_overlays > 0 && phrases >= budgets.phrase_overlays {
                drop_reason(&mut intake, "budget");
                continue;
            }
            if role(&item) == "scene_bullet" && budgets.bullet > 0 && bullets >= budgets.bullet {
                drop_reason(&mut intake, "budget");
                continue;
            }
        }
        if is_image(&item) && budgets.image_overlays > 0 && images >= budgets.image_overlays {
            drop_reason(&mut intake, "budget");
            continue;
        }
        if is_map(&item) && budgets.map_overlays > 0 && maps >= budgets.map_overlays {
            drop_reason(&mut intake, "budget");
            continue;
        }
        if max_per_scene > 0
            && scene_counts
                .get(item.scene_id.as_str())
                .copied()
                .unwrap_or(0)
                >= max_per_scene
        {
            drop_reason(&mut intake, "per_scene");
            continue;
        }
        *group_counts.entry(group).or_default() += 1;
        *scene_counts.entry(item.scene_id.clone()).or_default() += 1;
        if item.kind == "text_phrase" {
            phrases += 1;
            if role(&item) == "scene_bullet" {
                bullets += 1;
            }
        }
        if is_image(&item) {
            images += 1;
        }
        if is_map(&item) {
            maps += 1;
        }
        *intake.planned_by_kind.entry(item.kind.clone()).or_default() += 1;
        selected.push(item);
    }
    intake.planned = selected.len() as i64;
    (selected, intake)
}

fn drop_reason(intake: &mut IntakeV2, reason: &str) {
    *intake
        .dropped_by_reason
        .entry(reason.to_string())
        .or_default() += 1;
}

fn role(item: &PlanItemV2) -> &str {
    item.params
        .as_ref()
        .and_then(|params| params.get("editorial_role"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn category_for(item: &PlanItemV2) -> &'static str {
    match item.kind.as_str() {
        "text_phrase" => match role(item) {
            "scene_title" => "titles",
            "scene_bullet" | "chapter_bullet" => "bullets",
            _ => "phrases",
        },
        "image" | "entity_image" | "product" | "logo" => "images",
        "map" => "maps",
        "number" => "numbers",
        "quote" => "quotes",
        "brand_text" => "brand_text",
        _ => "phrases",
    }
}

fn category_disabled(request: &EditorialPlanV2Request, item: &PlanItemV2) -> bool {
    let category = category_for(item);
    if let Some(scene) = request
        .scenes
        .iter()
        .find(|scene| scene.id == item.scene_id)
    {
        if let Some(toggle) = scene.overlay_categories.get(category) {
            if !toggle.is_empty() && toggle != "default" {
                return toggle == "disabled";
            }
        }
    }
    request
        .canvas
        .overlay_categories
        .get(category)
        .map(|toggle| toggle == "disabled")
        .unwrap_or(false)
}

fn group_for(item: &PlanItemV2) -> &'static str {
    match item.kind.as_str() {
        "text_phrase" if role(item) == "scene_bullet" => "bullets",
        "text_phrase" => "phrases",
        "image" | "entity_image" | "product" | "logo" => "images",
        "map" => "maps",
        "entity_card" | "organization" | "location" | "concept" => "entities",
        _ => "phrases",
    }
}

fn is_image(item: &PlanItemV2) -> bool {
    matches!(
        item.kind.as_str(),
        "image" | "entity_image" | "product" | "logo"
    )
}

fn is_map(item: &PlanItemV2) -> bool {
    item.kind == "map" && item.map.is_some()
}

fn cascade(items: &mut [PlanItemV2], request: &EditorialPlanV2Request) {
    for item in items.iter_mut() {
        if matches!(
            item.kind.as_str(),
            "text_phrase" | "image" | "entity_image" | "number"
        ) {
            item.motion_params = Some(json!({"enter_frames": 8}));
        }
    }
    let entrance_items: Vec<CascadeEntranceItem> = items
        .iter()
        .map(|item| CascadeEntranceItem {
            id: item.id.clone(),
            kind: item.kind.clone(),
            template_id: item.template_id.clone(),
            start_ms: item.start_ms,
            end_ms: item.end_ms,
            motion_params: item.motion_params.clone(),
            image_layer_count: 0,
            asset_ref_count: item.asset_refs.len(),
            entity_caption: item.entity_caption.clone(),
            text: item.text.clone(),
            editorial_role: json!(role(item)),
        })
        .collect();
    let edits = cascade_clustered_entrances(CascadeEntrancesRequest {
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        items: entrance_items,
    });
    for edit in edits {
        if let Some(item) = items.get_mut(edit.index) {
            let mut params = item.motion_params.take().unwrap_or_else(|| json!({}));
            if let Some(object) = params.as_object_mut() {
                object.insert("enter_frames".to_string(), json!(edit.enter_frames));
            }
            item.motion_params = Some(params);
        }
    }
}

fn resolve_overlaps(request: &EditorialPlanV2Request, items: Vec<PlanItemV2>) -> Vec<PlanItemV2> {
    if items.is_empty() {
        return items;
    }
    let input: Vec<TimingItem> = items.iter().map(timing_item).collect();
    let edits = crate::resolve_map_overlaps(crate::ResolveMapOverlapsRequest {
        fps_num: request.plan.fps_num,
        fps_den: request.plan.fps_den,
        scene_windows: request.scene_windows.clone(),
        items: input,
    });
    if edits.len() != items.len() {
        return items;
    }
    let mut resolved = Vec::with_capacity(items.len());
    for (index, edit) in edits.into_iter().enumerate() {
        if !edit.keep {
            continue;
        }
        let mut item = items[index].clone();
        if edit.start_us != item.start_us || edit.duration_us != item.duration_us {
            item.start_us = edit.start_us;
            item.duration_us = edit.duration_us;
            item.start_ms = edit.start_ms;
            item.end_ms = edit.end_ms;
        }
        resolved.push(item);
    }
    resolved
}

fn record_drops(intake: &mut IntakeV2, reason: &str, before: usize, after: usize) {
    if before > after {
        *intake
            .dropped_by_reason
            .entry(reason.to_string())
            .or_default() += (before - after) as i64;
    }
}

fn finalize_intake(intake: &mut IntakeV2, items: &[PlanItemV2]) {
    intake.planned = items.len() as i64;
    intake.planned_by_kind.clear();
    for item in items {
        *intake.planned_by_kind.entry(item.kind.clone()).or_default() += 1;
    }
}

fn seal(request: &EditorialPlanV2Request, items: Vec<PlanItemV2>, intake: IntakeV2) -> FinalPlanV2 {
    let seal = request.seal.as_ref().expect("validated seal");
    let scene_end_ms = request
        .scenes
        .iter()
        .map(|scene| (scene.timeline_start_us + scene.duration_us) / 1000)
        .max()
        .unwrap_or(0);
    let duration_ms = (seal.audio_duration_us / 1000).max(scene_end_ms);
    let recap_cards = items
        .iter()
        .filter(|item| {
            item.params
                .as_ref()
                .and_then(|params| params.get("recap"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count() as i64;
    let chart_cards = items
        .iter()
        .filter(|item| {
            item.params
                .as_ref()
                .and_then(|params| params.get("chart"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count() as i64;
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
        recap_placements: 0,
        recap_cards,
        chart_placements: 0,
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
            "canvas": {"group_limits": {}},
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
        assert_eq!(3, compiled.plan.items.len());
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
    fn maps_only_drops_every_non_map_candidate() {
        let mut request = request();
        request.canvas.maps_only = true;
        let compiled = compile_plan(&request).expect("plan");
        assert!(compiled.plan.items.is_empty());
        assert_eq!(3, compiled.plan.intake.dropped_by_reason["maps_only"]);
    }
}
