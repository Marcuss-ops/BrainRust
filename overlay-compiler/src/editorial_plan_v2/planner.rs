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
    items = apply_presentation(items, &request.plan.plan_id);
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

fn build_base_candidates(
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
            append_annotations(&mut items, scene, annotations, request);
        }
    }
    Ok(items)
}

struct RecapCandidates {
    items: Vec<PlanItemV2>,
    placements: i64,
}

/// Ports the Automatic Recap Builder over the projected scene briefs: sources
/// are strictly before the display scene, score-ordered, title-deduplicated
/// and never allowed to leak a later chapter into an earlier card.
fn build_recap_candidates(
    request: &EditorialPlanV2Request,
) -> Result<RecapCandidates, EditorialPlanError> {
    if request.canvas.disable_recap_overlays || request.recap_briefs.len() < 3 {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let display = (request.recap_briefs.len() / 2 + 1).min(request.recap_briefs.len() - 1);
    if display < 1 {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let mut sources = request.recap_briefs[..display]
        .iter()
        .filter(|brief| !brief.title.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut seen = BTreeSet::new();
    sources.retain(|brief| seen.insert(name_slug(&brief.title)));
    sources.truncate(3);
    if sources.is_empty() {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let window = &request.recap_briefs[display];
    if window.end_us <= window.start_us {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    }
    let Some(scene) = request.scenes.iter().find(|scene| scene.id == window.id) else {
        return Ok(RecapCandidates {
            items: Vec::new(),
            placements: 0,
        });
    };
    let mut cursor = window.start_us;
    let mut items = Vec::new();
    let mut used_ids = BTreeSet::new();
    for source in sources {
        let remaining = window.end_us.saturating_sub(cursor);
        if remaining < 2_000_000 {
            break;
        }
        let duration = remaining.min(4_000_000);
        let base = format!("{}-recap-{}", scene.id, name_slug(&source.title));
        let mut id = base.clone();
        let mut suffix = 2;
        while !used_ids.insert(id.clone()) {
            id = format!("{base}-{suffix}");
            suffix += 1;
        }
        let mut item = scene_item(
            id,
            scene,
            "quote",
            "QUOTE",
            cursor,
            duration,
            source.title.clone(),
        );
        item.preset_id = "phrase_default".to_string();
        item.params = Some(json!({
            "position": "center",
            "style": "quote",
            "priority": source.score,
            "recap": true,
        }));
        items.push(item);
        cursor = cursor.saturating_add(duration);
    }
    Ok(RecapCandidates {
        placements: if items.is_empty() { 0 } else { 1 },
        items,
    })
}

fn build_entity_card_candidates(
    request: &EditorialPlanV2Request,
) -> Result<Vec<PlanItemV2>, EditorialPlanError> {
    let mut items = Vec::new();
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
        if !scene.short_phrase_overlay && word_count(&phrase.text) < 3 {
            // Go filters every phrase lane through `phrasesAtLeastThreeWords`
            // after the editorial projection. A two-word title is not a
            // renderable block overlay unless the scene explicitly owns the
            // short-phrase lane.
            continue;
        }
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
    request: &EditorialPlanV2Request,
) {
    for (index, phrase) in annotations.phrases.iter().enumerate() {
        if !phrase.presentation_role.is_empty() {
            continue; // editorial projection owns title/bullet candidates
        }
        if !scene.short_phrase_overlay && word_count(&phrase.text) < 3 {
            continue;
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
        if let Some(item) = number_item(scene, index, number, request) {
            items.push(item);
        }
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

/// Routes a grounded numeric surface to its certified presentation family.
///
/// Dates are capped at the owning scene and kept on screen for the same
/// five-second target as the Go planner; a date with less than two readable
/// seconds left is dropped before admission instead of consuming a slot.
/// `42`-style values use the metric card family. Chart numbers stay separate
/// candidates and are lowered by the chart lane.
fn number_item(
    scene: &SceneV2,
    _index: usize,
    annotation: &TimedAnnotationV2,
    request: &EditorialPlanV2Request,
) -> Option<PlanItemV2> {
    let family = annotation.annotation_type.trim().to_ascii_uppercase();
    let is_date = matches!(family.as_str(), "DATE" | "TIME");
    let is_metric = matches!(
        family.as_str(),
        "NUMBER"
            | "NUM"
            | "CARDINAL"
            | "ORDINAL"
            | "MONEY"
            | "PERCENT"
            | "PERCENTAGE"
            | "METRIC"
            | "METRICS"
            | "STATISTIC"
            | "STATISTICS"
            | "QUANTITY"
    );
    let scene_end = scene.timeline_start_us.saturating_add(scene.duration_us);
    let mut start_us = annotation.start_us.max(scene.timeline_start_us);
    let mut duration_us = annotation.duration_us.max(0);
    if is_date {
        let remaining = scene_end.saturating_sub(start_us);
        if remaining < 2_000_000 {
            return None;
        }
        duration_us = duration_us.max(5_000_000).min(remaining);
    }
    let id = go_item_id(&scene.id, "number", &annotation.text);
    let mut item = scene_item(
        id.clone(),
        scene,
        "number",
        if is_date {
            "TIMELINE_DATE_CARD"
        } else if is_metric {
            "METRIC_STAT_CARD"
        } else {
            "NUMBER"
        },
        start_us,
        duration_us,
        annotation.text.clone(),
    );
    start_us = item.start_us;
    if is_date || is_metric {
        let (pool, lane, family_key) = if is_date {
            (
                date_motion_pool(request),
                "timeline_date",
                "date_presentation",
            )
        } else {
            (
                METRIC_MOTIONS.to_vec(),
                "metric_stat",
                "metric_presentation",
            )
        };
        let limit = request
            .canvas
            .animation_counts
            .get(lane)
            .copied()
            .unwrap_or(0)
            .max(0) as usize;
        let pool = if limit > 0 {
            limited_shuffled_pool(
                &pool,
                limit,
                &format!("{}\x00{}", request.plan.plan_id, family_key),
            )
        } else {
            pool
        };
        item.motion_id = seeded_preset(&request.plan.plan_id, &scene.id, &id, family_key, &pool)
            .unwrap_or_default();
    }
    if is_date {
        let remaining = scene_end.saturating_sub(start_us);
        item.duration_us = item.duration_us.max(5_000_000).min(remaining);
        item.end_ms = (item.start_us + item.duration_us + 999) / 1000;
    }
    let font_size = if is_date || is_metric { 140.0 } else { 112.0 };
    item.params = Some(json!({
        "position": "center",
        "style": "stat",
        "priority": annotation.score,
        "font_size_px": font_size,
    }));
    Some(item)
}

fn date_motion_pool(request: &EditorialPlanV2Request) -> Vec<&'static str> {
    let family = request
        .canvas
        .date_motion_family
        .trim()
        .to_ascii_lowercase();
    DATE_MOTIONS
        .iter()
        .copied()
        .filter(|motion| match family.as_str() {
            "typewriter" | "date_typewriter_v1" => motion.starts_with("date_typewriter_"),
            "didone" | "date_didone_v1" => motion.starts_with("date_didone_"),
            "classic" | "date_v1" => {
                !motion.starts_with("date_typewriter_") && !motion.starts_with("date_didone_")
            }
            _ => true,
        })
        .collect()
}

#[derive(Serialize)]
struct PresetSeed<'a> {
    job_fingerprint: &'a str,
    scene_id: &'a str,
    semantic_id: &'a str,
    preset_family: &'a str,
}

/// Mirrors Go's `DeterministicPresetSampler`: a SHA-256 over the stable JSON
/// seed maps its first eight bytes onto the candidate list. This is the one
/// sampler used by date cards, light leaks and every seeded presentation
/// choice; wall-clock time never participates.
fn seeded_preset(
    job_id: &str,
    scene_id: &str,
    item_id: &str,
    family: &str,
    candidates: &[&str],
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let encoded = serde_json::to_vec(&PresetSeed {
        job_fingerprint: job_id,
        scene_id,
        semantic_id: item_id,
        preset_family: family,
    })
    .ok()?;
    // Go's digest.SHA256Bytes returns the lowercase hex string, and the sampler
    // reads the first eight ASCII hex bytes as a big-endian integer. Keeping
    // the hex form here is what makes the two planners choose the same motion.
    let hex = format!("{:x}", Sha256::digest(encoded));
    let bytes: [u8; 8] = hex.as_bytes().get(..8)?.try_into().ok()?;
    let index = u64::from_be_bytes(bytes) % candidates.len() as u64;
    Some(candidates[index as usize].to_string())
}

/// Go's `itemID`: lowercase the concatenated identity, collapse every run of
/// non-letter/non-digit characters to one hyphen and cap the result at 110
/// runes with a stable hash suffix.
fn go_item_id(scene_id: &str, kind: &str, value: &str) -> String {
    let raw = format!("{scene_id}-{kind}-{value}").to_lowercase();
    let mut id = String::new();
    let mut separator = false;
    for ch in raw.chars() {
        if ch.is_alphanumeric() {
            if separator && !id.is_empty() {
                id.push('-');
            }
            id.push(ch);
            separator = false;
        } else {
            separator = true;
        }
    }
    let id = id.trim_matches('-').to_string();
    if id.chars().count() <= 110 {
        return id;
    }
    let head: String = id.chars().take(110).collect();
    let sum = format!("{:x}", Sha256::digest(raw.as_bytes()));
    format!("{}-{}", head.trim_end_matches('-'), &sum[..12])
}

/// Go's `limitedShuffledMotionPool`: a Fisher-Yates shuffle seeded by raw
/// SHA-256 bytes over `seed + NUL + rune(i)`, then capped to `limit`.
fn limited_shuffled_pool(pool: &[&'static str], limit: usize, seed: &str) -> Vec<&'static str> {
    let mut order = pool.to_vec();
    for i in (1..order.len()).rev() {
        let digest = Sha256::digest(format!(
            "{seed}\x00{}",
            char::from_u32(i as u32).unwrap_or('\0')
        ));
        let bytes: [u8; 8] = digest[..8].try_into().expect("sha256 prefix");
        let j = (u64::from_be_bytes(bytes) % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order.truncate(limit.min(order.len()));
    order
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
                if assets.is_empty() {
                    template
                } else {
                    "image_popup"
                },
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

/// Recovers a verified organization/place portrait that admission dropped with
/// its scene. The fallback uses only verified bytes already present on the
/// request, only when an occurrence gives it a certified window, and only
/// while the run-level image ceiling has room. It never searches or invents an
/// asset.
fn append_verified_intent_images(
    request: &EditorialPlanV2Request,
    items: &mut Vec<PlanItemV2>,
) -> Result<(), EditorialPlanError> {
    let Some(media) = request.media.as_ref() else {
        return Ok(());
    };
    let Some(timeline) = request.entity_timeline.as_ref() else {
        return Ok(());
    };
    let budgets = request.budgets.as_ref().expect("validated budgets");
    let max_images = if budgets.image_overlays > 0 {
        budgets.image_overlays
    } else {
        request.canvas.max_image_overlays
    };
    if max_images <= 0 {
        return Ok(());
    }
    let mut used = BTreeSet::new();
    let mut image_count = 0_i64;
    for item in items.iter() {
        if is_image(item) {
            image_count += 1;
        }
        for asset in &item.asset_refs {
            if !asset.sha256.trim().is_empty() {
                used.insert(asset.sha256.to_ascii_lowercase());
            }
        }
    }
    for verified in &media.verified_entity_assets {
        if image_count >= max_images {
            break;
        }
        let asset = verified
            .assets
            .iter()
            .find(|asset| !asset.url.trim().is_empty() && asset.sha256.trim().len() == 64);
        let Some(asset) = asset else {
            continue;
        };
        let sha = asset.sha256.to_ascii_lowercase();
        if !used.insert(sha.clone()) {
            continue;
        }
        let occurrence = timeline.scenes.iter().find_map(|scene| {
            scene.entities.iter().find(|entity| {
                let type_ok = matches!(
                    entity.entity_type.trim().to_ascii_uppercase().as_str(),
                    "ORG" | "ORGANIZATION" | "GPE" | "PLACE" | "LOCATION"
                );
                if !type_ok {
                    return false;
                }
                let id_match = !verified.entity_id.is_empty()
                    && (entity.entity_id == verified.entity_id
                        || entity.canonical_entity_id == verified.entity_id);
                let canonical_match = !verified.canonical_entity_id.is_empty()
                    && (entity.canonical_entity_id == verified.canonical_entity_id
                        || entity.entity_id == verified.canonical_entity_id);
                id_match || canonical_match
            })
        });
        let Some(entity) = occurrence else {
            continue;
        };
        let Some(scene) = request
            .scenes
            .iter()
            .find(|scene| scene.id == entity.scene_id)
        else {
            continue;
        };
        let Some((start_us, duration_us)) = entity_window(scene, entity) else {
            continue;
        };
        let name = entity_name(entity);
        let mut item = scene_item(
            format!("image-focus-reveal__{name}__entity_image__01"),
            scene,
            "entity_image",
            "image_popup",
            start_us,
            duration_us,
            String::new(),
        );
        item.entity_id = entity.entity_id.clone();
        item.entity_caption = name;
        item.asset_refs = vec![asset.clone()];
        item.params = Some(json!({
            "entity_type": entity.entity_type,
            "canonical_entity_id": entity.canonical_entity_id,
            "priority": entity.confidence,
        }));
        items.push(item);
        image_count += 1;
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
                    || (!map.label.is_empty() && map.label.eq_ignore_ascii_case(&entity.name))
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
    // The entity resolver keeps a card readable for at least one second even
    // when the spoken name is shorter; the scene boundary still wins.
    .max(start.saturating_add(1_000_000))
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

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
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

fn new_intake(request: &EditorialPlanV2Request, candidates: &[PlanItemV2]) -> IntakeV2 {
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
    for item in candidates {
        *intake
            .candidates_by_kind
            .entry(item.kind.clone())
            .or_default() += 1;
    }
    intake
}

fn admit(
    request: &EditorialPlanV2Request,
    candidates: Vec<PlanItemV2>,
    mut intake: IntakeV2,
) -> (Vec<PlanItemV2>, IntakeV2) {
    let budgets = request.budgets.as_ref().expect("validated budgets");
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

/// Generated documentary cards keep the name below the portrait. The generic
/// `random` selector can sample a side composition, so a captioned card is
/// pinned to the below-caption pool exactly like Go's presentation pass.
/// Certified date-card and metric-card pools, in the registration order the
/// Go accessors expose. Motion ids are renderer vocabulary, not presets.
const DATE_MOTIONS: &[&str] = &[
    "date_fade_rise",
    "date_timeline_tick",
    "date_chronology_focus",
    "date_calendar_drop",
    "date_month_wipe",
    "date_era_zoom",
    "date_bracket_draw",
    "date_didone_archive_fade",
    "date_didone_chronicle_rise",
    "date_didone_era_settle",
    "date_didone_fade_tracking",
    "date_didone_focus_hold",
    "date_didone_margin_wipe",
    "date_didone_rule_draw",
    "date_didone_stamp_drop",
    "date_didone_underline_hold",
    "date_didone_year_slide",
    "date_digit_flip",
    "date_history_stack",
    "date_marker_drop",
    "date_marker_pop",
    "date_page_turn",
    "date_range_draw",
    "date_segment_stagger",
    "date_split_year",
    "date_stamp_reveal",
    "date_timeline_sweep",
    "date_typewriter_bracket_tap",
    "date_typewriter_day_stamp",
    "date_typewriter_era_carriage",
    "date_typewriter_ink_stamp",
    "date_typewriter_margin_return",
    "date_typewriter_month_strike",
    "date_typewriter_range_type",
    "date_typewriter_timeline_feed",
    "date_typewriter_underline_key",
    "date_typewriter_year_type",
    "date_underline_focus",
    "date_year_count",
];

const LIGHT_LEAK_MOTIONS: &[&str] = &[
    "lightleak_anamorphic_pass",
    "lightleak_bokeh_pass",
    "lightleak_corner_bloom",
    "lightleak_diagonal_sweep",
    "lightleak_dual_edge",
    "lightleak_edge_warm",
    "lightleak_film_burn",
    "lightleak_focus_glow",
    "lightleak_horizontal_sweep",
    "lightleak_prismatic",
    "lightleak_pulse",
    "lightleak_soft_flare",
];

fn light_leak_motion(plan_id: &str, target_id: &str) -> String {
    seeded_preset(
        plan_id,
        target_id,
        target_id,
        "light_leak",
        LIGHT_LEAK_MOTIONS,
    )
    .unwrap_or_default()
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Ports `ClipStartLightLeakItems`: one short leak at every real video clip
/// start. Freeze tails are not clips and receive nothing.
fn append_light_leak_items(
    request: &EditorialPlanV2Request,
    mut items: Vec<PlanItemV2>,
) -> Vec<PlanItemV2> {
    let seal_duration = request
        .seal
        .as_ref()
        .map(|seal| seal.audio_duration_us)
        .unwrap_or(0);
    if request.canvas.light_leak_on_clip_start {
        for clip in &request.clips {
            if clip.freeze
                || clip.asset_id.trim().is_empty()
                || clip.duration_us <= 0
                || clip.start_us < 0
                || (seal_duration > 0 && clip.start_us >= seal_duration)
            {
                continue;
            }
            let scene = request
                .scenes
                .iter()
                .find(|scene| scene.id == clip.scene_id);
            let Some(scene) = scene else {
                continue;
            };
            let mut duration = clip.duration_us.min(1_200_000);
            if seal_duration > 0 {
                duration = duration.min(seal_duration.saturating_sub(clip.start_us));
            }
            if duration <= 0 {
                continue;
            }
            let id = format!(
                "light-leak-clip-{}",
                short_hash(&format!(
                    "{}\0{}\0{}",
                    clip.scene_id, clip.index, clip.asset_id
                ))
            );
            let mut item = scene_item(
                id.clone(),
                scene,
                "light_leak",
                "LIGHT_LEAK",
                clip.start_us,
                duration,
                String::new(),
            );
            item.group_id = scene.id.clone();
            item.subgroup_id = id.clone();
            item.motion_id = light_leak_motion(&request.plan.plan_id, &id);
            items.push(item);
        }
    }
    if request.canvas.light_leak_on_overlay_start {
        let mut leaks = Vec::new();
        for overlay in items.iter() {
            if overlay.kind == "light_leak" || overlay.duration_us <= 0 || overlay.id.is_empty() {
                continue;
            }
            let Some(scene) = request
                .scenes
                .iter()
                .find(|scene| scene.id == overlay.scene_id)
            else {
                continue;
            };
            let id = format!(
                "light-leak-overlay-{}",
                short_hash(&format!("{}\0{}", request.plan.plan_id, overlay.id))
            );
            let duration = overlay.duration_us.min(1_200_000);
            let mut item = scene_item(
                id.clone(),
                scene,
                "light_leak",
                "LIGHT_LEAK",
                overlay.start_us,
                duration,
                String::new(),
            );
            item.group_id = overlay.group_id.clone();
            item.subgroup_id = id.clone();
            item.motion_id = light_leak_motion(&request.plan.plan_id, &id);
            leaks.push(item);
        }
        items.extend(leaks);
    }
    items
}

/// Ports `CoalesceChainedOverlaysWithSceneEnds`: nearby, same-scene overlays
/// fuse into one sequential chain only after admission. Image-backed item runs
/// are coalesced separately so a text card never interrupts a slideshow.
fn coalesce_chained_overlays(
    items: Vec<PlanItemV2>,
    request: &EditorialPlanV2Request,
    scene_ends: &BTreeMap<String, i64>,
) -> Vec<PlanItemV2> {
    // Go attaches the resolved portrait before consulting the chain config, so
    // a chain-disabled plan still gives a text card its certified image.
    let items = attach_resolved_entity_images(items);
    let Some((max_gap_us, max_segments)) = chain_config(&request.canvas.animation_counts) else {
        return items;
    };
    if items.len() < 2 {
        return items;
    }
    let image_count = items
        .iter()
        .filter(|item| chain_family(&item.kind) == "image")
        .count();
    if image_count > 0 && image_count < items.len() {
        let mut images = Vec::new();
        let mut others = Vec::new();
        for item in items {
            if chain_family(&item.kind) == "image" {
                images.push(item);
            } else {
                others.push(item);
            }
        }
        let mut out = coalesce_chain_subset(images, max_gap_us, max_segments, scene_ends, 0);
        out.extend(coalesce_chain_subset(
            others,
            max_gap_us,
            max_segments,
            scene_ends,
            0,
        ));
        out.sort_by(|left, right| {
            left.window_us()
                .0
                .cmp(&right.window_us().0)
                .then_with(|| left.id.cmp(&right.id))
        });
        return out;
    }
    coalesce_chain_subset(items, max_gap_us, max_segments, scene_ends, 0)
}

fn chain_config(counts: &BTreeMap<String, i64>) -> Option<(i64, usize)> {
    if counts.get("chain_disabled").copied().unwrap_or(0) > 0 {
        return None;
    }
    let max_gap_us = if let Some(value) = counts.get("chain_gap_ms").copied().filter(|v| *v > 0) {
        // Go bounds the value before multiplying so a malformed oversized
        // configuration cannot overflow int64; saturate here for the same
        // fail-closed result instead of panicking or wrapping.
        value.saturating_mul(1000).min(5_000_000)
    } else if counts.get("chain_enabled").copied().unwrap_or(0) > 0 {
        5_000_000
    } else {
        return None;
    };
    let max_segments = counts
        .get("chain_max_segments")
        .copied()
        .filter(|value| *value > 0)
        .unwrap_or(0) as usize;
    Some((max_gap_us, max_segments))
}

fn coalesce_chain_subset(
    items: Vec<PlanItemV2>,
    max_gap_us: i64,
    max_segments: usize,
    scene_ends: &BTreeMap<String, i64>,
    chain_index_start: u64,
) -> Vec<PlanItemV2> {
    let mut sorted = items;
    sorted.sort_by(|left, right| {
        left.window_us()
            .0
            .cmp(&right.window_us().0)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut out = Vec::with_capacity(sorted.len());
    let mut index = 0;
    let mut chain_index = chain_index_start;
    while index < sorted.len() {
        if !chainable(&sorted[index]) {
            out.push(sorted[index].clone());
            index += 1;
            continue;
        }
        let scene_id = sorted[index].scene_id.clone();
        let mut group = vec![sorted[index].clone()];
        let mut group_end = sorted[index].window_us().1;
        let mut next = index + 1;
        while next < sorted.len() && (max_segments == 0 || group.len() < max_segments) {
            let candidate = &sorted[next];
            if !chainable(candidate) || candidate.scene_id != scene_id {
                break;
            }
            let gap = candidate.window_us().0.saturating_sub(group_end);
            let mut gap_limit = max_gap_us;
            if max_gap_us >= 5_000_000
                && is_image_backed_entity_card(group.last().expect("group"))
                && is_image_backed_entity_card(candidate)
            {
                gap_limit = gap_limit.max(7_500_000);
            }
            if gap > gap_limit {
                break;
            }
            group.push(candidate.clone());
            group_end = group_end.max(candidate.window_us().1);
            next += 1;
        }
        if group.len() < 2 {
            out.push(sorted[index].clone());
            index += 1;
            continue;
        }
        if chain_exceeds_scene_end(&group, scene_ends.get(&scene_id).copied()) {
            out.extend(group);
            index = next;
            continue;
        }
        out.push(fuse_chain(&group, chain_index));
        chain_index += 1;
        index = next;
    }
    out
}

/// Only text, image and card items enter automatic coalescing. Maps are
/// deliberately absent: a map owns the full georeferenced canvas, pin timing
/// and LOD assets, so it must keep its standalone window. The wire contract
/// still accepts an explicitly authored `map_chain`; this compiler never
/// produces one.
fn chainable(item: &PlanItemV2) -> bool {
    if item.params.as_ref().is_some_and(|params| {
        params.get("chart_card") == Some(&Value::Bool(true))
            || params.get("chart_bar") == Some(&Value::Bool(true))
    }) {
        return false;
    }
    if item.template_id.trim() == "TIMELINE_DATE_CARD" {
        return false;
    }
    if item
        .image_layers
        .as_ref()
        .is_some_and(|layers| layers.as_array().is_some_and(|layers| !layers.is_empty()))
        || !item.group_motion_id.trim().is_empty()
    {
        return false;
    }
    let family = chain_family(&item.kind);
    if family.is_empty() {
        return false;
    }
    if family == "image" && item.asset_refs.is_empty() {
        return false;
    }
    if item.sound_effect.is_some() || item.window_us().1 <= item.window_us().0 {
        return false;
    }
    if family == "text" && item.motion_id.starts_with("short_phrase_") {
        return false;
    }
    true
}

fn is_image_backed_entity_card(item: &PlanItemV2) -> bool {
    chain_family(&item.kind) == "card" && !item.asset_refs.is_empty()
}

fn attach_resolved_entity_images(mut items: Vec<PlanItemV2>) -> Vec<PlanItemV2> {
    let mut donors: BTreeMap<String, Vec<AssetRefV2>> = BTreeMap::new();
    for item in items.iter() {
        let id = item.entity_id.trim();
        if id.is_empty() || item.asset_refs.is_empty() {
            continue;
        }
        if chain_family(&item.kind) != "card" && chain_family(&item.kind) != "image" {
            continue;
        }
        donors
            .entry(id.to_string())
            .or_insert_with(|| item.asset_refs.clone());
    }
    if donors.is_empty() {
        return items;
    }
    for item in items.iter_mut() {
        let kind = item.kind.trim().to_ascii_lowercase();
        if !item.asset_refs.is_empty()
            || !matches!(
                kind.as_str(),
                "entity_card" | "organization" | "location" | "concept"
            )
        {
            continue;
        }
        if let Some(assets) = donors.get(item.entity_id.trim()) {
            item.asset_refs = assets.clone();
        }
    }
    items
}

const METRIC_MOTIONS: &[&str] = &[
    "metric_counter_scale_settle",
    "metric_odometer_vertical",
    "metric_digits_stagger",
    "metric_delta_reveal",
    "metric_focus_punch",
    "metric_before_after",
    "metric_count_flip",
    "metric_split_odometer",
    "metric_arc_sweep",
    "metric_bar_grow",
    "metric_bounce_settle",
    "metric_compare_wipe",
    "metric_counter_rise",
    "metric_debt_flip",
    "metric_didone_column_rise",
    "metric_didone_drop_in",
    "metric_didone_focus_pull",
    "metric_didone_hairline_rise",
    "metric_didone_hold_breathe",
    "metric_didone_ink_settle",
    "metric_didone_numeral_settle",
    "metric_didone_rule_wipe",
    "metric_didone_side_slide",
    "metric_didone_tracking_open",
    "metric_digit_cascade",
    "metric_multi_stat_focus",
    "metric_pulse_hold",
    "metric_ribbon_unfold",
    "metric_ring_draw",
    "metric_slide_left",
    "metric_typewriter_backspace_settle",
    "metric_typewriter_carriage_slide",
    "metric_typewriter_cursor_in",
    "metric_typewriter_digit_tick",
    "metric_typewriter_ink_stamp",
    "metric_typewriter_key_strike",
    "metric_typewriter_letter_drop",
    "metric_typewriter_line_feed",
    "metric_typewriter_ribbon_strike",
    "metric_typewriter_stepped_reveal",
];

const ENTITY_STYLE_BELOW: &str = "premium_below_random_v1";
const ENTITY_CARD_IMAGE_PRESET: &str = "image_scale_in";
const ENTITY_CAPTION_FONT: &str = "playfair_display_italic";
const ENTITY_CAPTION_COLOR: &str = "#000000";

/// Assigns the run-level entity-card entrance and composition decisions after
/// admission, so dropped cards never consume a pool slot.
///
/// Go's `assignEntityImageMotions` rotates the certified request pools by
/// `(image_motion_offset + ordinal) % len`; replaying a request therefore picks
/// the same entrance, and a retry whose offset advanced picks a different one.
/// Captions rotate in their own lane. The composition selector is stamped only
/// on a card that carries both a portrait and a caption — the same fail-closed
/// precondition RenderingGen enforces. `render_key` stays empty: sealing clears
/// it and Go recomputes it from the finished document.
fn assign_entity_media(
    request: &EditorialPlanV2Request,
    items: &mut [PlanItemV2],
) -> Result<(), EditorialPlanError> {
    if !items.iter().any(|item| item.kind == "entity_image") {
        return Ok(());
    }
    let canvas = &request.canvas;
    if canvas.entity_image_motions.is_empty() {
        return Err(EditorialPlanError::invalid(
            "canvas.entity_image_motions",
            "a non-empty certified pool is required when an entity image is admitted",
        ));
    }
    let pinned = canvas.entity_style_id.trim();
    let offset = canvas.image_motion_offset;
    let mut caption_ordinal = 0_i64;
    for (ordinal, item) in items
        .iter_mut()
        .filter(|item| item.kind == "entity_image")
        .enumerate()
    {
        item.motion_id = rotate_motion(&canvas.entity_image_motions, offset, ordinal as i64);
        item.image_preset_id = ENTITY_CARD_IMAGE_PRESET.to_string();
        let captioned = !item.entity_caption.trim().is_empty() && !item.asset_refs.is_empty();
        if captioned {
            if canvas.entity_caption_motions.is_empty() {
                return Err(EditorialPlanError::invalid(
                    "canvas.entity_caption_motions",
                    "a non-empty certified pool is required when an entity caption is admitted",
                ));
            }
            item.caption_motion_id =
                rotate_motion(&canvas.entity_caption_motions, offset, caption_ordinal);
            item.caption_layout = "below".to_string();
            item.caption_font_family = ENTITY_CAPTION_FONT.to_string();
            item.caption_color = ENTITY_CAPTION_COLOR.to_string();
            item.entity_style_id = ENTITY_STYLE_BELOW.to_string();
            caption_ordinal += 1;
        } else if !pinned.is_empty() {
            // An uncaptioned card has no name to place below the portrait, so
            // an explicit pinned selector survives instead of being replaced.
            item.entity_style_id = pinned.to_string();
        }
    }
    Ok(())
}

fn rotate_motion(pool: &[String], offset: i64, ordinal: i64) -> String {
    let len = pool.len() as i64;
    let index = (offset.rem_euclid(len) + ordinal).rem_euclid(len) as usize;
    pool[index].clone()
}

fn cascade(items: &mut [PlanItemV2], request: &EditorialPlanV2Request) {
    for item in items.iter_mut() {
        if item.kind == "entity_image" {
            // Entity cards keep the certified fast 8-frame snap even when
            // their one-second minimum window overlaps an earlier image.
            item.motion_params = Some(json!({"enter_frames": 8}));
            continue;
        }
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
            if item.kind == "entity_image" {
                continue;
            }
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

/// Visible review identity: motion, subject and overlay family. Mirrors
/// `overlays.NameItemsForReview`, including the per-prefix two-digit counter.
/// The final documentary presentation pass. It owns typography, font alias
/// normalization, image-caption placement, chain-segment captions and the
/// long-copy split. It never changes selection or timing windows except for
/// the split chunks, which conserve the original interval.
fn apply_presentation(items: Vec<PlanItemV2>, plan_id: &str) -> Vec<PlanItemV2> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if let Some(parts) = split_long_overlay_text(&item, plan_id) {
            out.extend(parts);
            continue;
        }
        out.push(item);
    }
    for item in &mut out {
        normalize_presentation_item(item);
    }
    out
}

const PLAYFAIR_RUNTIME_FAMILY: &str = "playfair_display_italic";
const PRESENTATION_TEXT_FILL: &str = "#000000";
const PRESENTATION_GLOW_COLOR: &str = "#FFFFFF";
const PRESENTATION_GLOW_PX: f64 = 10.0;
const PRESENTATION_TEXT_STYLE_KEYS: &[&str] = &[
    "font_family",
    "font_size_px",
    "fill",
    "glow_size",
    "glow_color",
    "stroke_size",
    "shadow_blur_px",
    "shadow_opacity",
    "shadow_offset_x_px",
    "shadow_offset_y_px",
];

fn is_chain_kind(kind: &str) -> bool {
    matches!(
        kind,
        "phrase_chain" | "image_chain" | "mixed_chain" | "map_chain"
    )
}

fn presentation_text_params(base: Option<&Value>, font_size: f64) -> Value {
    let mut params = base.and_then(Value::as_object).cloned().unwrap_or_default();
    params.remove("background");
    for key in [
        "shadow_color",
        "shadow_blur_px",
        "shadow_opacity",
        "shadow_offset_x_px",
        "shadow_offset_y_px",
    ] {
        params.remove(key);
    }
    params.insert("font_family".to_string(), json!(PLAYFAIR_RUNTIME_FAMILY));
    params.insert("fill".to_string(), json!(PRESENTATION_TEXT_FILL));
    params.insert("glow_size".to_string(), json!(PRESENTATION_GLOW_PX));
    params.insert("glow_color".to_string(), json!(PRESENTATION_GLOW_COLOR));
    params.insert("font_size_px".to_string(), json!(font_size));
    params.insert("width".to_string(), json!(1700.0));
    params.insert("height".to_string(), json!(600.0));
    params.insert("position".to_string(), json!("center"));
    params.remove("position_x");
    Value::Object(params)
}

fn presentation_text_style(base: Option<&Value>, font_size: f64) -> Value {
    let mut style = base.and_then(Value::as_object).cloned().unwrap_or_default();
    style.remove("background");
    style.remove("shadow");
    style.insert("font_family".to_string(), json!(PLAYFAIR_RUNTIME_FAMILY));
    style.insert("fill".to_string(), json!(PRESENTATION_TEXT_FILL));
    style.insert(
        "glow".to_string(),
        json!({"color": PRESENTATION_GLOW_COLOR}),
    );
    style.insert("font_size".to_string(), json!(font_size));
    Value::Object(style)
}

fn strip_presentation_text_keys(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for key in PRESENTATION_TEXT_STYLE_KEYS {
            object.remove(*key);
        }
    }
}

fn normalize_fonts(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                match key.as_str() {
                    "font_family" | "caption_font_family"
                        if child.as_str().is_some_and(|v| !v.trim().is_empty()) =>
                    {
                        *child = json!(PLAYFAIR_RUNTIME_FAMILY);
                    }
                    "caption_layout" => *child = json!("below"),
                    "caption_motion_id" => {
                        if child
                            .as_str()
                            .is_some_and(|v| v.to_ascii_lowercase().contains("split_side"))
                        {
                            *child = json!("");
                        }
                    }
                    _ => normalize_fonts(child),
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize_fonts(child);
            }
        }
        _ => {}
    }
}

fn normalize_presentation_item(item: &mut PlanItemV2) {
    if item.caption_font_family.trim().is_empty() {
        if !item.entity_caption.trim().is_empty() {
            item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
        }
    } else {
        item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
    }
    if !item.entity_caption.trim().is_empty() {
        item.caption_layout = "below".to_string();
        item.caption_color = PRESENTATION_TEXT_FILL.to_string();
    }
    if item
        .caption_motion_id
        .to_ascii_lowercase()
        .contains("split_side")
    {
        item.caption_motion_id.clear();
    }
    if let Some(params) = item.params.as_mut() {
        normalize_fonts(params);
    }
    if let Some(style) = item.style.as_mut() {
        normalize_fonts(style);
    }
    if is_chain_kind(&item.kind) {
        item.caption_layout = "below".to_string();
        item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
        if let Some(params) = item.params.as_mut() {
            strip_presentation_text_keys(params);
            if let Some(segments) = params
                .get_mut("chain_segments")
                .and_then(Value::as_array_mut)
            {
                for segment in segments {
                    normalize_fonts(segment);
                    if let Some(object) = segment.as_object_mut() {
                        let has_caption = object
                            .get("entity_caption")
                            .and_then(Value::as_str)
                            .is_some_and(|caption| !caption.trim().is_empty());
                        if has_caption {
                            object.insert("caption_layout".to_string(), json!("below"));
                            object.insert(
                                "caption_font_family".to_string(),
                                json!(PLAYFAIR_RUNTIME_FAMILY),
                            );
                            object
                                .insert("caption_color".to_string(), json!(PRESENTATION_TEXT_FILL));
                        }
                    }
                }
            }
        }
        if let Some(style) = item.style.as_mut() {
            strip_presentation_text_keys(style);
        }
        return;
    }
    match item.kind.as_str() {
        "text_phrase" | "important_phrase" => {
            let runes = item.text.chars().count();
            let existing = item
                .params
                .as_ref()
                .and_then(|params| params.get("font_size_px"))
                .and_then(Value::as_f64)
                .unwrap_or(72.0);
            let font_size = if runes > 160 {
                64.0
            } else if runes > 120 {
                existing.min(72.0)
            } else {
                existing
            };
            item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
            item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
        }
        "number" => {
            let font_size = 140.0;
            item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
            item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
        }
        "quote" => {
            let font_size = 72.0;
            item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
            item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
        }
        "entity_image" | "entity_card" | "location" | "organization" | "concept" => {
            if !item.asset_refs.is_empty() || !item.entity_caption.trim().is_empty() {
                item.caption_layout = "below".to_string();
                item.caption_font_family = PLAYFAIR_RUNTIME_FAMILY.to_string();
                item.caption_color = PRESENTATION_TEXT_FILL.to_string();
                item.entity_style_id = "premium_below_random_v1".to_string();
                if let Some(params) = item.params.as_mut() {
                    strip_presentation_text_keys(params);
                    if let Some(object) = params.as_object_mut() {
                        object.insert("caption_layout".to_string(), json!("below"));
                        object.insert("position_x".to_string(), json!(0.0));
                        object.entry("position_y".to_string()).or_insert(json!(0.0));
                    }
                }
                item.style = None;
            } else {
                let font_size = 96.0;
                item.params = Some(presentation_text_params(item.params.as_ref(), font_size));
                item.style = Some(presentation_text_style(item.style.as_ref(), font_size));
            }
        }
        _ => {}
    }
}

fn split_long_overlay_text(item: &PlanItemV2, plan_id: &str) -> Option<Vec<PlanItemV2>> {
    let text = item.text.trim();
    if text.chars().count() <= 180 || item.kind != "text_phrase" || !item.asset_refs.is_empty() {
        return None;
    }
    let chunks = split_phrase_text(text, 180);
    if chunks.len() <= 1 {
        return None;
    }
    let (start_us, duration_us) = item.window_us();
    let total: usize = chunks.iter().map(|chunk| chunk.chars().count()).sum();
    if total == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(chunks.len());
    let mut cursor = start_us;
    for (index, chunk) in chunks.iter().enumerate() {
        let chunk_duration = if index + 1 == chunks.len() {
            start_us + duration_us - cursor
        } else {
            (duration_us * chunk.chars().count() as i64 / total as i64).max(1_200_000)
        }
        .min(start_us + duration_us - cursor);
        if chunk_duration <= 0 {
            continue;
        }
        let mut part = item.clone();
        part.id = format!("{}-part-{}", item.id, (b'a' + index as u8) as char);
        part.kind = "text_phrase".to_string();
        part.template_id = "IMPORTANT_PHRASE".to_string();
        part.preset_id = "phrase_default".to_string();
        part.image_preset_id.clear();
        part.entity_caption.clear();
        part.caption_motion_id.clear();
        part.caption_font_family.clear();
        part.caption_layout.clear();
        part.caption_color.clear();
        part.asset_refs.clear();
        part.image_layers = None;
        part.group_motion_id.clear();
        part.frame = None;
        part.map = None;
        part.entity_id.clear();
        part.params = None;
        part.style = None;
        part.render_key.clear();
        part.text = chunk.clone();
        part.start_us = cursor;
        part.duration_us = chunk_duration;
        part.start_ms = cursor / 1000;
        part.end_ms = (cursor + chunk_duration + 999) / 1000;
        if part.motion_id.trim().is_empty() {
            part.motion_id = seeded_preset(
                plan_id,
                &part.scene_id,
                &part.id,
                "text_motion",
                &[
                    "text_phrase_rise",
                    "text_phrase_reveal",
                    "text_phrase_focus",
                ],
            )
            .unwrap_or_default();
        }
        out.push(part);
        cursor += chunk_duration;
    }
    (!out.is_empty()).then_some(out)
}

fn split_phrase_text(text: &str, max_runes: usize) -> Vec<String> {
    let mut runes: Vec<char> = text.trim().chars().collect();
    let mut out = Vec::new();
    while !runes.is_empty() {
        if runes.len() <= max_runes {
            let chunk = runes.iter().collect::<String>().trim().to_string();
            if !chunk.is_empty() {
                out.push(chunk);
            }
            break;
        }
        let mut cut = max_runes;
        for index in (80..max_runes).rev() {
            if matches!(runes[index - 1], '.' | '!' | '?' | ';' | ':') {
                cut = index;
                break;
            }
        }
        if cut == max_runes {
            for index in (80..max_runes).rev() {
                if runes[index - 1] == ' ' {
                    cut = index;
                    break;
                }
            }
        }
        let chunk = runes[..cut].iter().collect::<String>().trim().to_string();
        if !chunk.is_empty() {
            out.push(chunk);
        }
        runes.drain(..cut);
        while runes.first() == Some(&' ') {
            runes.remove(0);
        }
    }
    if out.is_empty() {
        vec![text.to_string()]
    } else {
        out
    }
}

fn name_items_for_review(items: &mut [PlanItemV2]) {
    let mut seen: BTreeMap<String, i64> = BTreeMap::new();
    for item in items.iter_mut() {
        let motion = if !item.motion_id.trim().is_empty() {
            item.motion_id.trim().to_string()
        } else {
            "static".to_string()
        };
        let mut entity = String::new();
        if let Some(map) = item.map.as_ref() {
            entity = map
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if entity.is_empty() {
            entity = item.text.trim().to_string();
        }
        if entity.is_empty() {
            entity = item.entity_caption.trim().to_string();
        }
        if entity.is_empty() {
            entity = item.id.clone();
        }
        let category = name_category(&item.kind, &motion);
        let base = format!(
            "{}__{}__{}",
            name_slug(&motion),
            name_slug(&entity),
            category
        );
        let count = seen.entry(base.clone()).or_default();
        *count += 1;
        item.id = format!("{base}__{count:02}");
    }
}

fn name_category(kind: &str, motion: &str) -> String {
    match kind {
        "map" => "map".to_string(),
        "text_phrase" if motion.starts_with("short_phrase_") => "short_phrase".to_string(),
        "text_phrase" => "phrase".to_string(),
        "number" => "date_number".to_string(),
        "entity_image" | "entity_card" => "entity_image".to_string(),
        "image" => "image".to_string(),
        "quote" => "quote".to_string(),
        "brand_text" => "brand_text".to_string(),
        other => name_slug(other),
    }
}

fn name_slug(value: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::new();
    let mut separator = false;
    for ch in value.trim().to_lowercase().nfd() {
        if unicode_normalization::char::is_combining_mark(ch) {
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            if separator && !out.is_empty() {
                out.push('-');
            }
            out.push(ch);
            separator = false;
        } else {
            separator = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let mut result = trimmed.chars().take(48).collect::<String>();
    result = result.trim_end_matches('-').to_string();
    if result.is_empty() {
        "unnamed".to_string()
    } else {
        result
    }
}

fn count_marked_items(items: &[PlanItemV2], key: &str) -> i64 {
    let mut count = 0;
    for item in items {
        if item
            .params
            .as_ref()
            .and_then(|params| params.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            count += 1;
        }
        if let Some(segments) = item
            .params
            .as_ref()
            .and_then(|params| params.get("chain_segments"))
            .and_then(Value::as_array)
        {
            count += segments
                .iter()
                .filter(|segment| {
                    segment
                        .get("params")
                        .and_then(|params| params.get(key))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .count() as i64;
        }
    }
    count
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

/// Automatic chain families. `map` is intentionally not a family: maps stay
/// standalone and `map_chain` is accepted only as an explicitly authored wire
/// item, never emitted by the Rust compiler.
fn chain_family(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "text_phrase" | "important_word" | "keyword" | "quote" | "brand_text" | "brand-text"
        | "number" => "text",
        "image" | "entity_image" | "product" | "logo" => "image",
        "entity_card" | "organization" | "location" | "concept" | "lower_third" => "card",
        _ => "",
    }
}

fn captioned_entity_image_card(item: &PlanItemV2) -> bool {
    chain_family(&item.kind) == "card"
        && !item.asset_refs.is_empty()
        && !item.entity_caption.trim().is_empty()
        && item.sound_effect.is_none()
}

/// Mirrors `projectedChainEndUS`: the first segment keeps the group start,
/// later segments follow after the handoff gap, and card segments get the same
/// 2.5s minimum hold as the fused output.
fn projected_chain_end_us(group: &[PlanItemV2]) -> i64 {
    let Some(first) = group.first() else {
        return 0;
    };
    let start = group
        .iter()
        .map(|item| item.window_us().0)
        .min()
        .unwrap_or_else(|| first.window_us().0);
    let mut cursor = start;
    for (index, item) in group.iter().enumerate() {
        let (item_start, item_end) = item.window_us();
        let mut duration = item_end.saturating_sub(item_start);
        if chain_family(&item.kind) != "text" && duration < 2_500_000 {
            duration = 2_500_000;
        }
        if index > 0 {
            cursor = cursor.saturating_add(400_000);
        }
        cursor = cursor.saturating_add(duration);
    }
    cursor
}

fn chain_exceeds_scene_end(group: &[PlanItemV2], scene_end: Option<i64>) -> bool {
    matches!(scene_end, Some(end) if end > 0 && group.len() >= 2 && projected_chain_end_us(group) > end)
}

fn insert_non_empty(object: &mut serde_json::Map<String, Value>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_optional(object: &mut serde_json::Map<String, Value>, key: &str, value: &Option<Value>) {
    if let Some(value) = value.as_ref() {
        object.insert(key.to_string(), value.clone());
    }
}

/// Builds one chain container exactly like the former Go `model.fuseChain`:
/// segments are laid
/// out sequentially with a 400ms handoff, card segments keep a 2.5s minimum
/// hold, and every segment keeps its own text, motion, caption, assets and
/// opaque metadata. The container carries no preset/motion of its own.
fn fuse_chain(group: &[PlanItemV2], chain_index: u64) -> PlanItemV2 {
    let first = &group[0];
    let start_us = group
        .iter()
        .map(|item| item.window_us().0)
        .min()
        .unwrap_or(0);
    let mut families = BTreeSet::new();
    for item in group {
        families.insert(chain_family(&item.kind));
    }
    let family = if families.len() == 1 {
        families.iter().next().copied().unwrap_or("mixed")
    } else {
        "mixed"
    };
    let (kind, template) = match family {
        "text" => ("phrase_chain", "PHRASE_CHAIN"),
        "image" => ("image_chain", "IMAGE_CHAIN"),
        _ => ("mixed_chain", "MIXED_CHAIN"),
    };
    let mut cursor_us = start_us;
    let mut segments: Vec<Value> = Vec::with_capacity(group.len());
    let mut motions: Vec<Value> = Vec::with_capacity(group.len());
    let mut assets: Vec<AssetRefV2> = Vec::new();
    let mut seen_assets: BTreeSet<String> = BTreeSet::new();
    let mut recap_count = 0_i64;
    for (index, item) in group.iter().enumerate() {
        let (item_start, item_end) = item.window_us();
        let mut duration_us = item_end.saturating_sub(item_start);
        if chain_family(&item.kind) != "text" && duration_us < 2_500_000 {
            duration_us = 2_500_000;
        }
        let segment_start_us = if index == 0 {
            item_start
        } else {
            cursor_us.saturating_add(400_000)
        };
        let mut segment = serde_json::Map::new();
        insert_non_empty(&mut segment, "text", &item.text);
        insert_non_empty(&mut segment, "group_id", &item.group_id);
        insert_non_empty(&mut segment, "subgroup_id", &item.subgroup_id);
        insert_non_empty(&mut segment, "motion_id", &item.motion_id);
        insert_non_empty(&mut segment, "preset_id", &item.preset_id);
        insert_non_empty(&mut segment, "image_preset_id", &item.image_preset_id);
        insert_non_empty(&mut segment, "template_id", &item.template_id);
        insert_non_empty(&mut segment, "kind", &item.kind);
        insert_non_empty(&mut segment, "entity_id", &item.entity_id);
        insert_non_empty(&mut segment, "entity_caption", &item.entity_caption);
        insert_non_empty(&mut segment, "caption_motion_id", &item.caption_motion_id);
        insert_optional(
            &mut segment,
            "caption_motion_params",
            &item.caption_motion_params,
        );
        insert_non_empty(
            &mut segment,
            "caption_font_family",
            &item.caption_font_family,
        );
        insert_non_empty(&mut segment, "caption_layout", &item.caption_layout);
        insert_non_empty(&mut segment, "caption_color", &item.caption_color);
        insert_non_empty(&mut segment, "entity_style_id", &item.entity_style_id);
        insert_non_empty(&mut segment, "camera_motion_id", &item.camera_motion_id);
        segment.insert("start_us".to_string(), json!(segment_start_us));
        segment.insert("duration_us".to_string(), json!(duration_us));
        insert_optional(&mut segment, "motion_params", &item.motion_params);
        if let Some(asset) = item.asset_refs.first() {
            segment.insert("asset_id".to_string(), json!(asset.asset_id));
        }
        if !item.asset_refs.is_empty() {
            segment.insert("assets".to_string(), json!(item.asset_refs));
        }
        insert_optional(&mut segment, "image_layers", &item.image_layers);
        insert_optional(&mut segment, "frame", &item.frame);
        insert_optional(&mut segment, "params", &item.params);
        insert_optional(&mut segment, "style", &item.style);
        insert_optional(&mut segment, "map", &item.map);
        if item
            .params
            .as_ref()
            .and_then(|params| params.get("recap"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            recap_count += 1;
        }
        segments.push(Value::Object(segment));
        motions.push(json!(item.motion_id));
        cursor_us = segment_start_us.saturating_add(duration_us);
        for asset in &item.asset_refs {
            if asset.asset_id.trim().is_empty() || !seen_assets.insert(asset.asset_id.clone()) {
                continue;
            }
            assets.push(asset.clone());
        }
    }
    let mut params = serde_json::Map::new();
    if recap_count > 0 {
        params.insert("recap_count".to_string(), json!(recap_count));
    }
    params.insert("chain_segments".to_string(), Value::Array(segments));
    params.insert("chain_motions".to_string(), Value::Array(motions));
    params.insert("chain_count".to_string(), json!(group.len()));
    params.insert("chain_camera".to_string(), json!("fly_under_fade"));
    params.insert("chain_gap_us".to_string(), json!(400_000_i64));
    let legacy_text = if first.text.trim().is_empty() {
        group
            .iter()
            .map(|item| item.text.as_str())
            .find(|text| !text.trim().is_empty())
            .unwrap_or("")
            .to_string()
    } else {
        first.text.clone()
    };
    PlanItemV2 {
        id: if first.scene_id.trim().is_empty() {
            format!("chain-{family}:{chain_index}")
        } else {
            format!("{}:chain-{family}:{chain_index}", first.scene_id.trim())
        },
        scene_id: first.scene_id.clone(),
        kind: kind.to_string(),
        template_id: template.to_string(),
        text: legacy_text,
        start_ms: start_us / 1000,
        end_ms: (cursor_us + 999) / 1000,
        start_us,
        duration_us: cursor_us.saturating_sub(start_us),
        asset_refs: assets,
        params: Some(Value::Object(params)),
        ..PlanItemV2::default()
    }
}

/// Ports `CoalesceCaptionedEntityImageCards`: build runs from the card lane per
/// scene (not adjacency in the full list), fuse only same-scene runs within the
/// 7.5s entity-card gap, and keep the group standalone when the sequential
/// layout would cross the scene end.
fn fuse_captioned_entity_image_chains(
    items: Vec<PlanItemV2>,
    scene_ends: &BTreeMap<String, i64>,
) -> Vec<PlanItemV2> {
    if items.len() < 2 {
        return items;
    }
    // No-op plans must stay byte-identical: only a run with at least two
    // captioned entity cards can produce a chain, so do not re-sort the rest.
    if items
        .iter()
        .filter(|item| captioned_entity_image_card(item))
        .count()
        < 2
    {
        return items;
    }
    let mut sorted = items;
    sorted.sort_by(|left, right| {
        left.window_us()
            .0
            .cmp(&right.window_us().0)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut by_scene: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, item) in sorted.iter().enumerate() {
        if captioned_entity_image_card(item) {
            by_scene
                .entry(item.scene_id.clone())
                .or_default()
                .push(index);
        }
    }
    let mut fused_at: BTreeMap<usize, PlanItemV2> = BTreeMap::new();
    let mut suppressed: BTreeSet<usize> = BTreeSet::new();
    for indices in by_scene.values() {
        let mut index = 0;
        while index < indices.len() {
            let first = indices[index];
            let mut group = vec![sorted[first].clone()];
            let mut members = vec![first];
            let mut group_end = sorted[first].window_us().1;
            let mut next = index + 1;
            while next < indices.len() {
                let candidate_index = indices[next];
                let candidate = &sorted[candidate_index];
                let gap = candidate.window_us().0.saturating_sub(group_end);
                if gap > 7_500_000 {
                    break;
                }
                group.push(candidate.clone());
                members.push(candidate_index);
                group_end = group_end.max(candidate.window_us().1);
                next += 1;
            }
            if group.len() > 1 {
                let scene_end = scene_ends.get(&group[0].scene_id).copied();
                if !chain_exceeds_scene_end(&group, scene_end) {
                    fused_at.insert(first, fuse_chain(&group, 1000 + first as u64));
                    for member in members.iter().skip(1) {
                        suppressed.insert(*member);
                    }
                }
            }
            index = next;
        }
    }
    let mut out = Vec::with_capacity(sorted.len().saturating_sub(suppressed.len()));
    for (index, item) in sorted.into_iter().enumerate() {
        if let Some(chain) = fused_at.remove(&index) {
            out.push(chain);
        } else if !suppressed.contains(&index) {
            out.push(item);
        }
    }
    out.sort_by(|left, right| {
        left.window_us()
            .0
            .cmp(&right.window_us().0)
            .then_with(|| left.id.cmp(&right.id))
    });
    out
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
