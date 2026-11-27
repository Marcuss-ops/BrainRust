//! Media projection: images, maps, entity cards and the verified-asset
//! binding. Everything here decides WHICH asset a plan item points at; the
//! bytes stay owned by the Go side.

use super::*;
use semantic_core::entity_capabilities::{fold_entity_kind, EntityKind};

/// Adds the payload-selected caption plate after presentation normalization,
/// which deliberately owns and replaces typography style maps.
pub(super) fn apply_entity_badge_color(items: &mut [PlanItemV2], color: &str) {
    let color = color.trim();
    if color.is_empty() {
        return;
    }
    for item in items.iter_mut().filter(|item| {
        item.kind == "entity_image"
            && !item.entity_caption.trim().is_empty()
            && !item.asset_refs.is_empty()
    }) {
        let mut style_object = item
            .style
            .take()
            .and_then(|style| style.as_object().cloned())
            .unwrap_or_default();
        style_object.insert(
            "style".to_string(),
            json!({"background": {"color": color, "radius": 0, "padding": [12, 24]}}),
        );
        item.style = Some(Value::Object(style_object));
    }
}

pub(super) fn append_image(
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
    let mut params = json!({"priority": candidate.score});
    if kind == "image" {
        // Authored, readable viewport. Without one the renderer falls back to a
        // 320px card anchored by the preset's side layout: tiny and off-centre
        // at 1080p. The box and the centring are plan data, the renderer only
        // interprets them.
        if let Some(object) = params.as_object_mut() {
            object.insert("width".to_string(), json!(IMAGE_VIEWPORT_WIDTH));
            object.insert("height".to_string(), json!(IMAGE_VIEWPORT_HEIGHT));
            object.insert("position".to_string(), json!("center"));
            object.insert("fit".to_string(), json!("contain"));
        }
    }
    item.params = Some(params);
    items.push(item);
}

/// Viewport an independent image card occupies: about 70% of a 1920x1080
/// canvas. Shared with `normalize_image_presentation` so the two writers of an
/// image card cannot disagree.
const IMAGE_VIEWPORT_WIDTH: f64 = 1340.0;
const IMAGE_VIEWPORT_HEIGHT: f64 = 760.0;

/// Side of the square portrait box an entity card is authored into.
const ENTITY_IMAGE_VIEWPORT: f64 = 820.0;

/// Stamps the readable square viewport on an entity card. The layout belongs to
/// the card, not to the standalone-image pass: a card fused into an image_chain
/// never reaches `normalize_image_presentation`, so the same geometry must be
/// authored before coalescing. Existing explicit values always win.
fn stamp_entity_image_viewport(item: &mut PlanItemV2) {
    let Some(Value::Object(params)) = item.params.as_mut() else {
        return;
    };
    params
        .entry("width".to_string())
        .or_insert_with(|| json!(ENTITY_IMAGE_VIEWPORT));
    params
        .entry("height".to_string())
        .or_insert_with(|| json!(ENTITY_IMAGE_VIEWPORT));
    params
        .entry("position_x".to_string())
        .or_insert_with(|| json!(0.0));
    params
        .entry("position_y".to_string())
        .or_insert_with(|| json!(0.0));
}

pub(super) fn append_map(
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

pub(super) fn append_entity_cards(
    items: &mut Vec<PlanItemV2>,
    request: &EditorialPlanV2Request,
    decisions: &mut Vec<PlanDecisionV2>,
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
        // A disabled images category keeps the identity visible as a text card.
        // Attaching the portrait here would turn it into an `entity_image` that
        // admission drops by the same toggle, losing the card altogether.
        let images_disabled = category_toggle_disabled(request, &scene.id, "images");
        let mut text_cards = 0;
        let mut admitted_identities = std::collections::HashSet::new();
        for &entity_index in indices {
            let Some(entity) = entity_scene.entities.get(entity_index) else {
                continue;
            };
            let candidate_id = entity_overlay_id(&entity_scene.scene_id, entity);
            let Some((kind, template)) = entity_card_kind(&entity.entity_type) else {
                push_entity_decision(
                    decisions,
                    REASON_ENTITY_KIND_UNSUPPORTED,
                    &candidate_id,
                    &entity_scene.scene_id,
                    entity,
                );
                continue;
            };
            if scene_has_map_for(scene, entity) {
                // A grounded place already renders as a map pin. Emitting a
                // location card as well would render the same entity twice.
                push_entity_decision(
                    decisions,
                    REASON_MAP_COVERS_ENTITY,
                    &candidate_id,
                    &entity_scene.scene_id,
                    entity,
                );
                continue;
            }
            let identity = entity_card_identity(entity);
            if !identity.is_empty() && admitted_identities.contains(identity) {
                // One card per identity per scene. Two occurrences of the same
                // person are spans of one card, not two portraits competing
                // for the same image budget.
                push_entity_decision(
                    decisions,
                    REASON_DUPLICATE_IDENTITY,
                    &candidate_id,
                    &entity_scene.scene_id,
                    entity,
                );
                continue;
            }
            let assets = if images_disabled {
                Vec::new()
            } else {
                verified_entity_assets(media, entity)
            };
            if assets.is_empty() {
                if text_cards >= 2 {
                    push_entity_decision(
                        decisions,
                        REASON_TEXT_CARD_LIMIT,
                        &candidate_id,
                        &entity_scene.scene_id,
                        entity,
                    );
                    continue;
                }
                text_cards += 1;
            }
            let Some((start_us, duration_us)) = entity_window(scene, entity) else {
                push_entity_decision(
                    decisions,
                    REASON_NO_TIMING_WINDOW,
                    &candidate_id,
                    &entity_scene.scene_id,
                    entity,
                );
                continue;
            };
            let name = entity_name(entity);
            let mut item = scene_item(
                candidate_id,
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
            item.entity_ref = entity_ref_of(entity, &name);
            item.params = Some(json!({
                "entity_type": entity.entity_type,
                "canonical_entity_id": entity.canonical_entity_id,
                "priority": entity.confidence,
            }));
            if !identity.is_empty() {
                admitted_identities.insert(identity.to_string());
            }
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
                    || (!map.label.is_empty() && map.label.eq_ignore_ascii_case(&entity.name))
            })
        })
        .unwrap_or(false)
}

fn entity_window(scene: &SceneV2, entity: &EntityOccurrenceV2) -> Option<(i64, i64)> {
    let scene_start = scene.timeline_start_us;
    let scene_end = scene_start.saturating_add(scene.duration_us);
    // The scene-local window is the authority. `timeline_start_us` can be the
    // scene anchor (Go stamps it that way for every occurrence of a scene), so
    // preferring it made every card in scene 2+ start at the scene beginning
    // and run until its own mention ended: 10-20 second stills instead of a
    // 2.5-5 second card, and no two cards close enough to chain.
    //
    // `audio_start_us` alone is not enough either: the envelope carries it
    // absolute (`timeline_start_us + local_start_us`), while other stages hand
    // the planner an already scene-relative value. Anchoring `local_start_us`
    // on the scene satisfies both shapes; a scene-2 mention then starts at its
    // own word instead of at the scene anchor.
    let has_local = entity.local_end_us > entity.local_start_us;
    let start = if has_local || entity.local_start_us > 0 {
        scene_start.saturating_add(entity.local_start_us.max(0))
    } else if entity.audio_start_us > scene_start && entity.audio_start_us < scene_end {
        entity.audio_start_us
    } else if entity.timeline_start_us > scene_start && entity.timeline_start_us < scene_end {
        entity.timeline_start_us
    } else {
        scene_start
    }
    .clamp(scene_start, scene_end);
    let spoken_end = if has_local {
        scene_start.saturating_add(entity.local_end_us.max(0))
    } else if entity.audio_end_us > entity.audio_start_us {
        entity.audio_end_us
    } else {
        start.saturating_add(2_000_000)
    };
    let end = spoken_end
        // A card must stay readable for at least 2.5s even when the spoken name is
        // shorter: one second was a flash, not an overlay (a name spoken in ~0.8s
        // produced a 1.0s card that the viewer could not read). 2.5s matches the
        // minimum hold a chained image segment already gets, so a standalone card
        // and the first segment of a chain have the same floor. The scene boundary
        // still wins.
        .max(start.saturating_add(2_500_000))
        .min(scene_end);
    (end > start).then_some((start, end - start))
}

pub(super) fn verified_entity_assets(
    media: &MediaV2,
    entity: &EntityOccurrenceV2,
) -> Vec<AssetRefV2> {
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

/// The card lane for one entity, sourced from the single fold in
/// `semantic_core::entity_capabilities`. `MEDIA_OUTLET` lands on the
/// organization family here, which is what `identity.rs` already asserts when
/// it prefixes an outlet's canonical id with `org`.
fn entity_card_kind(entity_type: &str) -> Option<(&'static str, &'static str)> {
    match fold_entity_kind(entity_type) {
        EntityKind::Person => Some(("entity_card", "person_default")),
        EntityKind::Organization => Some(("organization", "org_default")),
        EntityKind::Place => Some(("location", "gpe_default")),
        EntityKind::Concept | EntityKind::Event => Some(("concept", "concept_default")),
        _ => None,
    }
}

pub(super) fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The planner-owned identity of one card. Emitted only when the three fields
/// the plan contract requires are non-empty, so a malformed occurrence degrades
/// to "no ref" instead of a plan that cannot validate.
fn entity_ref_of(entity: &EntityOccurrenceV2, name: &str) -> Option<EntityRefV2> {
    let entity_id = entity.entity_id.trim();
    let entity_type = entity.entity_type.trim();
    let name = name.trim();
    if entity_id.is_empty() || entity_type.is_empty() || name.is_empty() {
        return None;
    }
    Some(EntityRefV2 {
        entity_id: entity_id.to_string(),
        entity_type: entity_type.to_string(),
        name: name.to_string(),
        canonical_entity_id: entity.canonical_entity_id.trim().to_string(),
    })
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

/// The identity a card is deduplicated by. The canonical id is authoritative
/// when present; the stable occurrence id is the fallback for producers that
/// have not linked the entity yet. Scoping happens at the call site, because
/// the same identity legitimately gets one card per scene.
fn entity_card_identity(entity: &EntityOccurrenceV2) -> &str {
    let canonical = entity.canonical_entity_id.trim();
    if canonical.is_empty() {
        entity.entity_id.trim()
    } else {
        canonical
    }
}

pub(super) fn is_image_backed_entity_card(item: &PlanItemV2) -> bool {
    chain_family(&item.kind) == "card" && !item.asset_refs.is_empty()
}

// Identity ownership (ADR-029 exception 2): the planner carries the bytes a card
// needs, but it never takes the card→image decision. The catalog-backed half of
// that decision needs the certified-portrait index (PostgreSQL, PipelineGen) and
// the seeded preset sampler whose candidate vocabulary is Chronon-owned, so
// PipelineGen promotes the card and this file only supplies assets. The test
// below pins that split: a future edit that starts writing `kind` or
// `preset_id` here would create a second decision site and fail loudly.
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
/// A card a viewer can actually read. Below this the image is a flash.
const ENTITY_IMAGE_MIN_HOLD_US: i64 = 2_500_000;
/// Breathing room between two cards of the same scene; the same handoff the
/// image chain container uses.
const ENTITY_IMAGE_GAP_US: i64 = 400_000;

/// Kinds that share the scene's image lane and must take turns.
fn sequenced_image_kind(kind: &str) -> bool {
    matches!(kind.trim(), "entity_image" | "image")
}

/// Gives the images of one scene their own turn on screen.
///
/// An entity that is only named has no audio window of its own, so
/// `entity_window` falls back to the scene start. Three named entities in one
/// scene therefore produced three cards at the exact same instant: stacked on
/// top of each other, which reads as "one image behind the other" rather than a
/// sequence, and the last admitted card is the only one a viewer ever sees.
/// Cards now start in plan order, each keeping its admitted hold (never shorter
/// than the readable floor) with the chain handoff between them, all clamped to
/// the scene window so a card never outlives its scene.
pub(super) fn sequence_scene_image_windows(
    scene_windows: &std::collections::BTreeMap<String, [i64; 2]>,
    items: Vec<PlanItemV2>,
) -> Vec<PlanItemV2> {
    // Take turns in timeline order, never in vector order: a card admitted
    // later in the list can start earlier, and shuffling by list position used
    // to give the same instant to two images.
    let mut order: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| sequenced_image_kind(&item.kind) && item.duration_us > 0)
        .map(|(index, _)| index)
        .collect();
    order.sort_by(|left, right| {
        items[*left]
            .start_us
            .cmp(&items[*right].start_us)
            .then_with(|| items[*left].id.cmp(&items[*right].id))
    });
    let mut cursor: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut dropped: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let mut sequenced = items;
    for index in order {
        let item = &mut sequenced[index];
        let scene_id = item.scene_id.trim().to_string();
        let (scene_start, scene_end) = match scene_windows.get(&scene_id) {
            Some([start, end]) => (*start, *end),
            None => (
                item.start_us,
                item.start_us.saturating_add(item.duration_us),
            ),
        };
        if scene_end <= scene_start {
            continue;
        }
        let start = match cursor.get(&scene_id) {
            Some(next) => item.start_us.max(*next),
            None => item.start_us.max(scene_start),
        };
        if start >= scene_end {
            // No readable slot left in this scene. An image that cannot get one
            // is not shown at all: holding it over the previous one is the
            // stacking defect this pass exists to remove.
            dropped.insert(index);
            continue;
        }
        let available = scene_end - start;
        let hold = if available >= ENTITY_IMAGE_MIN_HOLD_US {
            item.duration_us
                .max(ENTITY_IMAGE_MIN_HOLD_US)
                .min(available)
        } else {
            available
        };
        item.start_us = start;
        item.duration_us = hold;
        item.start_ms = start / 1000;
        item.end_ms = (start.saturating_add(hold) + 999) / 1000;
        cursor.insert(scene_id, start.saturating_add(hold + ENTITY_IMAGE_GAP_US));
    }
    if dropped.is_empty() {
        return sequenced;
    }
    sequenced
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| (!dropped.contains(&index)).then_some(item))
        .collect()
}

pub(super) fn assign_entity_media(
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
    // `entity_split_side` animates position_x and is cleared by the
    // presentation pass for the below-caption layout. Selecting it here used to
    // spend a caption slot on a motion the renderer then discards, leaving the
    // caption with no entrance. The planner owns the draw, so it filters the
    // incompatible gesture before it can be selected.
    let caption_pool: Vec<String> = canvas
        .entity_caption_motions
        .iter()
        .filter(|motion| !motion.to_ascii_lowercase().contains("split_side"))
        .cloned()
        .collect();
    // ANIMRAND-06: a seeded shuffle bag, not a rotation. Every card takes a
    // different entrance until the certified pool is exhausted; the seed is the
    // ONE creative seed plus the lane, so a replay is identical.
    let mut image_bag = SeededMotionBag::new(
        &canvas.entity_image_motions,
        &format!("{}\x00image.entity", creative_seed(request)),
    );
    let mut caption_bag = SeededMotionBag::new(
        &caption_pool,
        &format!("{}\x00image.caption", creative_seed(request)),
    );
    for item in items.iter_mut().filter(|item| item.kind == "entity_image") {
        // An explicit entrance is authoritative: the lane only fills empty ids
        // (ANIMRAND-09).
        if item.motion_id.trim().is_empty() {
            item.motion_id = image_bag.take().unwrap_or_default();
            mark_motion_origin(item, ORIGIN_AUTO);
        }
        item.image_preset_id = ENTITY_CARD_IMAGE_PRESET.to_string();
        let captioned = !item.entity_caption.trim().is_empty() && !item.asset_refs.is_empty();
        if captioned {
            if item.caption_motion_id.trim().is_empty() {
                if caption_bag.is_empty() {
                    return Err(EditorialPlanError::invalid(
                        "canvas.entity_caption_motions",
                        "a non-empty certified pool is required when an entity caption is admitted",
                    ));
                }
                item.caption_motion_id = caption_bag.take().unwrap_or_default();
                mark_caption_origin(item, ORIGIN_AUTO);
            }
            item.caption_layout = "below".to_string();
            item.caption_font_family = ENTITY_CAPTION_FONT.to_string();
            item.caption_color = ENTITY_CAPTION_COLOR.to_string();
            item.entity_style_id = ENTITY_STYLE_BELOW.to_string();
        } else if !pinned.is_empty() {
            // An uncaptioned card has no name to place below the portrait, so
            // an explicit pinned selector survives instead of being replaced.
            item.entity_style_id = pinned.to_string();
        }
        // Authored here, before coalescing, so a captioned card keeps its
        // readable center viewport even when it becomes a chain segment.
        stamp_entity_image_viewport(item);
    }
    // Sequencing runs on the finished set, after coalescing: it only rewrites
    // windows, so a dropped card never leaves a gap in the run.
    Ok(())
}

/// Stamps the run-level entrance and the readable viewport on every independent
/// image item.
///
/// A standalone image used to reach the renderer with no motion and no authored
/// box: a 320px card parked at the preset's side layout that simply switched on.
/// The entrance pool rotates exactly like the entity-card pool, and the box is
/// only filled in when the item authored none, so an explicit plan wins.
pub(super) fn normalize_image_presentation(
    request: &EditorialPlanV2Request,
    items: &mut [PlanItemV2],
) -> Result<(), EditorialPlanError> {
    let canvas = &request.canvas;
    // Timeline order, not vector order: two images 30 s apart are not
    // "consecutive" just because admission appended them next to each other.
    let mut order: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| sequenced_image_kind(&item.kind))
        .map(|(index, _)| index)
        .collect();
    order.sort_by_key(|index| (items[*index].start_us, items[*index].id.clone()));
    // A lane of its own: the standalone images walk the certified single-image
    // pool with a seeded shuffle bag, so two runs are not the same rotation.
    let mut bag = SeededMotionBag::new(
        &canvas.image_motions,
        &format!("{}\x00image.standalone", creative_seed(request)),
    );
    for index in order {
        let item = &mut items[index];
        if item.motion_id.trim().is_empty() {
            // ANIMRAND-04: a missing pool is a configuration error, not a
            // reason to pin one entrance on every still. The silent fallback
            // hid an incomplete catalog and made a run of images look like a
            // single repeated card. `auto` means draw from the certified
            // pool; an empty pool fails closed here.
            if bag.is_empty() {
                return Err(EditorialPlanError::invalid(
                    "canvas.image_motions",
                    "a non-empty certified pool is required when a standalone image has no motion",
                ));
            }
            item.motion_id = bag.take().unwrap_or_default();
            mark_motion_origin(item, ORIGIN_AUTO);
        }
        if item.kind == "entity_image" {
            // Entity cards are portraits: a square box keeps the aspect and
            // makes the card roughly twice the width of the preset fallback,
            // while the caption still fits below it on a 1080p canvas. This
            // stays as the idempotent second writer for standalone cards; the
            // same helper already stamped them before coalescing.
            stamp_entity_image_viewport(item);
            continue;
        }
        let Some(serde_json::Value::Object(params)) = item.params.as_mut() else {
            continue;
        };
        params
            .entry("width".to_string())
            .or_insert_with(|| json!(IMAGE_VIEWPORT_WIDTH));
        params
            .entry("height".to_string())
            .or_insert_with(|| json!(IMAGE_VIEWPORT_HEIGHT));
        // The renderer reads an explicit offset before the string selector,
        // and an entrance motion can leave its own Position behind, so the
        // offsets are pinned at 0 instead of merely omitted: that is what
        // actually centres the card on the canvas.
        params.insert("position".to_string(), json!("center"));
        params.insert("position_x".to_string(), json!(0.0));
        params.insert("position_y".to_string(), json!(0.0));
        params
            .entry("fit".to_string())
            .or_insert_with(|| json!("contain"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
