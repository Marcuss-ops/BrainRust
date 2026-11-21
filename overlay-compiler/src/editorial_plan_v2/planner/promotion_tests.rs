//! Unit tests for the post-coalescing entity-card promotion.

use super::super::tests::request;
use super::*;

fn portrait(asset_id: &str, sha: char) -> AssetRefV2 {
    AssetRefV2 {
        asset_id: asset_id.to_string(),
        url: format!("https://example.test/{asset_id}.jpg"),
        sha256: sha.to_string().repeat(64),
        media_type: "image/jpeg".to_string(),
    }
}

fn presets() -> Vec<String> {
    [
        "image_fade_in",
        "image_focus_in",
        "image_scale_in",
        "image_slide_left",
        "image_slide_right",
    ]
    .iter()
    .map(|id| id.to_string())
    .collect()
}

fn request_with_promotable(entity_id: &str, canonical: &str) -> EditorialPlanV2Request {
    let mut request = request();
    request.canvas.image_presets = presets();
    request.media = Some(MediaV2 {
        promotable_entity_assets: vec![PromotableEntityAssetV2 {
            entity_id: entity_id.to_string(),
            canonical_entity_id: canonical.to_string(),
            assets: vec![portrait("portrait-tim", 'c')],
        }],
        ..MediaV2::default()
    });
    request
}

fn card(id: &str, text: &str, entity_id: &str) -> PlanItemV2 {
    PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        kind: "entity_card".to_string(),
        template_id: "person_default".to_string(),
        entity_id: entity_id.to_string(),
        text: text.to_string(),
        start_us: 0,
        duration_us: 2_500_000,
        start_ms: 0,
        end_ms: 2_500,
        params: Some(json!({"canonical_entity_id": "person:tim-cook", "priority": 0.9})),
        ..PlanItemV2::default()
    }
}

fn chain(segments: Vec<Value>) -> PlanItemV2 {
    PlanItemV2 {
        id: "scene-1:chain-mixed:0".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "mixed_chain".to_string(),
        template_id: "MIXED_CHAIN".to_string(),
        text: "Tim Cook".to_string(),
        start_us: 0,
        duration_us: 6_000_000,
        start_ms: 0,
        end_ms: 6_000,
        params: Some(json!({
            "chain_segments": segments,
            "chain_motions": ["card_reveal", "card_reveal"],
            "chain_count": 2,
            "chain_gap_us": 400_000,
        })),
        ..PlanItemV2::default()
    }
}

fn segment(entity_id: &str, canonical: &str, text: &str, start_us: i64) -> Value {
    json!({
        "kind": "entity_card",
        "template_id": "person_default",
        "text": text,
        "entity_id": entity_id,
        "motion_id": "card_reveal",
        "entity_style_id": "premium_below_random_v1",
        "params": {"canonical_entity_id": canonical},
        "start_us": start_us,
        "duration_us": 2_500_000,
    })
}

#[test]
fn a_promotable_card_takes_the_go_sampler_preset() {
    let request = request_with_promotable("person-tim", "person:tim-cook");
    let item = card("static__tim__entity_image__01", "Tim Cook", "person-tim");
    let want = select_preset(
        &request.plan.plan_id,
        "scene-1",
        "static__tim__entity_image__01",
        "entity_image",
        &request.canvas.image_presets,
    );
    assert!(!want.is_empty());

    let promoted = promote_entity_cards(&request, vec![item]);

    assert_eq!(1, promoted.len());
    let item = &promoted[0];
    assert_eq!("entity_image", item.kind);
    assert_eq!("image_popup", item.template_id);
    assert_eq!(want, item.preset_id);
    assert_eq!("image_scale_in", item.image_preset_id);
    assert_eq!("Tim Cook", item.entity_caption);
    assert!(item.text.is_empty(), "an image item renders no text field");
    assert!(item.params.is_none(), "card-only metadata is dropped");
    assert_eq!(1, item.asset_refs.len());
    assert_eq!("portrait-tim", item.asset_refs[0].asset_id);
}

#[test]
fn a_chain_segment_is_promoted_with_the_fused_chain_id_as_seed() {
    let request = request_with_promotable("person-tim", "person:tim-cook");
    let item = chain(vec![
        segment("person-tim", "person:tim-cook", "Tim Cook", 0),
        segment("person-other", "person:steve-jobs", "Steve Jobs", 2_900_000),
    ]);
    let want = select_preset(
        &request.plan.plan_id,
        "scene-1",
        "scene-1:chain-mixed:0",
        "entity_image",
        &request.canvas.image_presets,
    );

    let promoted = promote_entity_cards(&request, vec![item]);

    assert_eq!(1, promoted.len());
    let item = &promoted[0];
    // One segment stayed a card, so the container keeps its mixed family.
    assert_eq!("mixed_chain", item.kind);
    assert_eq!(1, item.asset_refs.len());
    assert_eq!(want, item.preset_id, "the chain mirrors its first segment");
    assert!(item.text.is_empty());
    let segments = item.params.as_ref().expect("params")["chain_segments"]
        .as_array()
        .expect("segments")
        .clone();
    assert_eq!(2, segments.len(), "promotion never drops a segment");
    let first = segments[0].as_object().expect("segment");
    assert_eq!(
        Some("entity_image"),
        first.get("kind").and_then(Value::as_str)
    );
    assert_eq!(
        Some(want.as_str()),
        first.get("preset_id").and_then(Value::as_str)
    );
    assert_eq!(
        Some("image_scale_in"),
        first.get("image_preset_id").and_then(Value::as_str)
    );
    assert_eq!(
        Some("Tim Cook"),
        first.get("entity_caption").and_then(Value::as_str)
    );
    for dropped in [
        "text",
        "entity_style_id",
        "motion_id",
        "motion_params",
        "params",
    ] {
        assert!(
            first.get(dropped).is_none(),
            "the card-only key {dropped} must not survive promotion"
        );
    }
    // Timing and identity are the planner's and stay verbatim.
    assert_eq!(Some(0), first.get("start_us").and_then(Value::as_i64));
    assert_eq!(
        Some(2_500_000),
        first.get("duration_us").and_then(Value::as_i64)
    );
    assert_eq!(
        Some("person-tim"),
        first.get("entity_id").and_then(Value::as_str)
    );
    let second = segments[1].as_object().expect("segment");
    assert_eq!(
        Some("entity_card"),
        second.get("kind").and_then(Value::as_str)
    );
    assert_eq!(
        Some("Steve Jobs"),
        second.get("text").and_then(Value::as_str)
    );
}

#[test]
fn a_fully_promoted_chain_becomes_an_image_chain() {
    let mut request = request_with_promotable("person-tim", "person:tim-cook");
    request
        .media
        .as_mut()
        .expect("media")
        .promotable_entity_assets
        .push(PromotableEntityAssetV2 {
            entity_id: "person-other".to_string(),
            canonical_entity_id: "person:steve-jobs".to_string(),
            assets: vec![portrait("portrait-steve", 'd')],
        });
    let item = chain(vec![
        segment("person-tim", "person:tim-cook", "Tim Cook", 0),
        segment("person-other", "person:steve-jobs", "Steve Jobs", 2_900_000),
    ]);

    let promoted = promote_entity_cards(&request, vec![item]);

    assert_eq!("image_chain", promoted[0].kind);
    assert_eq!("IMAGE_CHAIN", promoted[0].template_id);
    assert_eq!(2, promoted[0].asset_refs.len());
}

#[test]
fn a_card_that_already_carries_assets_is_never_promoted() {
    let request = request_with_promotable("person-tim", "person:tim-cook");
    let mut item = card("card-tim", "Tim Cook", "person-tim");
    item.asset_refs = vec![portrait("verified-tim", 'e')];
    let before = serde_json::to_string(&item).expect("encode");

    let promoted = promote_entity_cards(&request, vec![item]);

    assert_eq!(
        before,
        serde_json::to_string(&promoted[0]).expect("encode"),
        "an asset-bearing card is not promotable"
    );
}

#[test]
fn images_disabled_keeps_the_card_text_only() {
    let mut request = request_with_promotable("person-tim", "person:tim-cook");
    request
        .canvas
        .overlay_categories
        .insert("images".to_string(), "disabled".to_string());
    let item = card("card-tim", "Tim Cook", "person-tim");
    let before = serde_json::to_string(&item).expect("encode");

    let promoted = promote_entity_cards(&request, vec![item]);

    assert_eq!(before, serde_json::to_string(&promoted[0]).expect("encode"));
}

#[test]
fn no_promotable_surface_is_a_byte_identical_noop() {
    let mut request = request();
    request.canvas.image_presets = presets();
    request.media = Some(MediaV2::default());
    let items = vec![
        card("card-tim", "Tim Cook", "person-tim"),
        chain(vec![
            segment("person-tim", "person:tim-cook", "Tim Cook", 0),
            segment("person-other", "person:steve-jobs", "Steve Jobs", 2_900_000),
        ]),
    ];
    let before = serde_json::to_string(&items).expect("encode");

    let promoted = promote_entity_cards(&request, items);

    assert_eq!(before, serde_json::to_string(&promoted).expect("encode"));
}

#[test]
fn the_ceiling_drops_entity_images_only_and_never_on_a_noop_plan() {
    let request = request_with_promotable("person-tim", "person:tim-cook");
    let promoted = promote_entity_cards(
        &request,
        vec![
            card("card-tim", "Tim Cook", "person-tim"),
            PlanItemV2 {
                id: "phrase".to_string(),
                scene_id: "scene-1".to_string(),
                kind: "text_phrase".to_string(),
                text: "una frase".to_string(),
                start_us: 0,
                duration_us: 1_000_000,
                ..PlanItemV2::default()
            },
        ],
    );
    assert_eq!(2, promoted.len(), "only the entity image consumed a slot");
    assert_eq!("text_phrase", promoted[1].kind);
}
