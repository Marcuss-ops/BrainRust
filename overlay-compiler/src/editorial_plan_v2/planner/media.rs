//! Media projection: images, maps, entity cards and the verified-asset
//! binding. Everything here decides WHICH asset a plan item points at; the
//! bytes stay owned by the Go side.

use super::*;

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

/// Entrance used when the run handed no image-motion pool: the certified
/// single-image focus reveal, the same fallback a chain segment gets. An image
/// with no motion is a card that simply switches on, which is one of the
/// reasons the sequence read as "one image behind the other".
const FALLBACK_IMAGE_MOTION: &str = "image_focus_reveal";

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
pub(super) fn append_verified_intent_images(
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

fn entity_card_kind(entity_type: &str) -> Option<(&'static str, &'static str)> {
    match entity_type.trim().to_ascii_uppercase().as_str() {
        "PERSON" => Some(("entity_card", "person_default")),
        "ORG" | "ORGANIZATION" => Some(("organization", "org_default")),
        "GPE" | "PLACE" | "LOCATION" => Some(("location", "gpe_default")),
        "CONCEPT" | "EVENT" => Some(("concept", "concept_default")),
        _ => None,
    }
}

pub(super) fn word_count(text: &str) -> usize {
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

pub(super) fn is_image_backed_entity_card(item: &PlanItemV2) -> bool {
    chain_family(&item.kind) == "card" && !item.asset_refs.is_empty()
}

pub(super) fn attach_resolved_entity_images(mut items: Vec<PlanItemV2>) -> Vec<PlanItemV2> {
    let mut donors: BTreeMap<String, Vec<AssetRefV2>> = BTreeMap::new();
    for item in items.iter() {
        let id = item.entity_id.trim();
        if id.is_empty() || item.asset_refs.is_empty() {
            continue;
        }
        if chain_family(&item.kind) != "card" && chain_family(&item.kind) != "image" {
            continue;
        }
        // Only a renderable, content-addressed asset may be copied onto a card.
        // The same rule guards `append_verified_intent_images`: an unverified
        // portrait would reach the renderer as an impossible path and fail the
        // whole plan downstream.
        let verified: Vec<AssetRefV2> = item
            .asset_refs
            .iter()
            .filter(|asset| !asset.url.trim().is_empty() && asset.sha256.trim().len() == 64)
            .cloned()
            .collect();
        if verified.is_empty() {
            continue;
        }
        donors.entry(id.to_string()).or_insert(verified);
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

#[cfg(test)]
mod identity_ownership_tests {
    use super::*;

    fn asset(id: &str, sha: &str) -> AssetRefV2 {
        AssetRefV2 {
            asset_id: id.to_string(),
            url: format!("https://example.test/{id}.jpg"),
            sha256: sha.to_string(),
            media_type: "image/jpeg".to_string(),
        }
    }

    /// A card that already has a donor portrait in the request must keep its
    /// own kind, text and (empty) preset: attaching bytes is not promoting.
    #[test]
    fn attaching_a_resolved_image_never_takes_the_identity_decision() {
        let card = PlanItemV2 {
            id: "card-ada".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "entity_card".to_string(),
            template_id: "person_default".to_string(),
            entity_id: "ent_ada".to_string(),
            text: "Ada Lovelace".to_string(),
            entity_caption: "Matematica".to_string(),
            start_us: 0,
            duration_us: 2_000_000,
            ..Default::default()
        };
        let donor = PlanItemV2 {
            id: "image-ada".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "image".to_string(),
            entity_id: "ent_ada".to_string(),
            asset_refs: vec![asset("portrait-ada", &"1".repeat(64))],
            ..Default::default()
        };

        let out = attach_resolved_entity_images(vec![card, donor]);
        let card = &out[0];
        assert_eq!(
            "entity_card", card.kind,
            "the planner must not promote the card"
        );
        assert_eq!("person_default", card.template_id);
        assert_eq!("Ada Lovelace", card.text, "the card keeps its own text");
        assert!(
            card.preset_id.is_empty(),
            "preset selection belongs to the Go sampler"
        );
        assert!(card.image_preset_id.is_empty());
        assert_eq!(1, card.asset_refs.len(), "the donor bytes are attached");
        assert_eq!("portrait-ada", card.asset_refs[0].asset_id);
    }

    /// A card whose donor carries no content-hashed asset is not promoted and
    /// gets no bytes: an unverified portrait must never reach the renderer.
    #[test]
    fn unverified_donors_attach_nothing() {
        let card = PlanItemV2 {
            id: "card-ada".to_string(),
            kind: "entity_card".to_string(),
            entity_id: "ent_ada".to_string(),
            text: "Ada Lovelace".to_string(),
            ..Default::default()
        };
        let donor = PlanItemV2 {
            id: "image-ada".to_string(),
            kind: "image".to_string(),
            entity_id: "ent_ada".to_string(),
            asset_refs: vec![asset("portrait-ada", "")],
            ..Default::default()
        };

        let out = attach_resolved_entity_images(vec![card, donor]);
        assert_eq!("entity_card", out[0].kind);
        assert!(out[0].asset_refs.is_empty());
    }
}

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
) {
    let canvas = &request.canvas;
    for (ordinal, item) in items
        .iter_mut()
        .filter(|item| sequenced_image_kind(&item.kind))
        .enumerate()
    {
        if item.motion_id.trim().is_empty() {
            item.motion_id = if canvas.image_motions.is_empty() {
                FALLBACK_IMAGE_MOTION.to_string()
            } else {
                rotate_motion(
                    &canvas.image_motions,
                    canvas.image_motion_offset,
                    ordinal as i64,
                )
            };
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
}

fn rotate_motion(pool: &[String], offset: i64, ordinal: i64) -> String {
    let len = pool.len() as i64;
    let index = (offset.rem_euclid(len) + ordinal).rem_euclid(len) as usize;
    pool[index].clone()
}

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
}
