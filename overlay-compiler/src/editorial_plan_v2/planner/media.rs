//! Media projection: images, maps, entity cards and the verified-asset
//! binding. Everything here decides WHICH asset a plan item points at; the
//! bytes stay owned by the Go side.

use super::*;

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
    item.params = Some(json!({"priority": candidate.score}));
    items.push(item);
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
            .filter(|asset| {
                !asset.url.trim().is_empty() && asset.sha256.trim().len() == 64
            })
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
        assert_eq!("entity_card", card.kind, "the planner must not promote the card");
        assert_eq!("person_default", card.template_id);
        assert_eq!("Ada Lovelace", card.text, "the card keeps its own text");
        assert!(card.preset_id.is_empty(), "preset selection belongs to the Go sampler");
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
    }
    Ok(())
}

fn rotate_motion(pool: &[String], offset: i64, ordinal: i64) -> String {
    let len = pool.len() as i64;
    let index = (offset.rem_euclid(len) + ordinal).rem_euclid(len) as usize;
    pool[index].clone()
}
