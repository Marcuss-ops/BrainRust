use super::*;

/// The canonical key set of one `chain_segments` entry. PipelineGen
/// (`capmodel.ChainSegment`) and RenderingGen (`chainSegment`) decode these
/// exact names, so adding or renaming a key here is a port-wide contract
/// change, not a local refactor.
const CANONICAL_KEYS: [&str; 26] = [
    "asset_id",
    "assets",
    "caption_color",
    "caption_font_family",
    "caption_layout",
    "caption_motion_id",
    "caption_motion_params",
    "duration_us",
    "entity_caption",
    "entity_id",
    "entity_style_id",
    "frame",
    "group_id",
    "image_layers",
    "image_preset_id",
    "kind",
    "map",
    "motion_id",
    "motion_params",
    "params",
    "preset_id",
    "start_us",
    "style",
    "subgroup_id",
    "template_id",
    "text",
];

fn fully_populated(id: &str, start_us: i64) -> PlanItemV2 {
    PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        group_id: "group-a".to_string(),
        subgroup_id: "subgroup-a".to_string(),
        entity_id: "ent_ada".to_string(),
        kind: "entity_card".to_string(),
        start_ms: start_us / 1000,
        end_ms: (start_us + 2_000_000) / 1000,
        start_us,
        duration_us: 2_000_000,
        template_id: "person_default".to_string(),
        preset_id: "card_default".to_string(),
        image_preset_id: "image_scale_in".to_string(),
        motion_id: "card_reveal".to_string(),
        entity_style_id: "premium_below_random_v1".to_string(),
        motion_params: Some(json!({"enter_frames": 17})),
        text: "Ada Lovelace".to_string(),
        entity_caption: "Matematica".to_string(),
        caption_motion_id: "caption_rise".to_string(),
        caption_motion_params: Some(json!({"rise_px": 12})),
        caption_font_family: "playfair_display_italic".to_string(),
        caption_layout: "below".to_string(),
        caption_color: "#000000".to_string(),
        style: Some(json!({"font_family": "Inter"})),
        asset_refs: vec![AssetRefV2 {
            asset_id: "portrait-ada".to_string(),
            url: "https://example.test/ada.jpg".to_string(),
            sha256: "1".repeat(64),
            media_type: "image/jpeg".to_string(),
        }],
        image_layers: Some(json!([{"asset_id": "layer-a"}])),
        frame: Some(json!({"x": 0})),
        map: Some(json!({"provider": "local"})),
        params: Some(json!({"recap": true})),
        ..Default::default()
    }
}

fn fused_segments(group: Vec<PlanItemV2>) -> Vec<Value> {
    let chain = fuse_chain(&group, 0, CHAIN_HANDOFF_US, &[], "segment-contract");
    chain.params.as_ref().expect("chain params")["chain_segments"]
        .as_array()
        .expect("chain segments")
        .clone()
}

#[test]
fn chain_segment_wire_keys_are_the_documented_contract() {
    let segments = fused_segments(vec![
        fully_populated("a", 100_000),
        fully_populated("b", 3_000_000),
    ]);
    assert_eq!(2, segments.len());
    let keys: std::collections::BTreeSet<&str> = segments[0]
        .as_object()
        .expect("segment object")
        .keys()
        .map(String::as_str)
        .collect();
    let want: std::collections::BTreeSet<&str> = CANONICAL_KEYS.iter().copied().collect();
    assert_eq!(want, keys, "chain segment wire keys drifted");
}

#[test]
fn blank_string_fields_stay_off_the_wire() {
    let mut first = fully_populated("a", 0);
    first.caption_color = "   ".to_string();
    let mut second = fully_populated("b", 3_000_000);
    second.caption_color = "#ffffff".to_string();
    let segments = fused_segments(vec![first, second]);
    assert!(
        segments[0].get("caption_color").is_none(),
        "a whitespace-only segment field must not be emitted"
    );
    assert_eq!(
        Some("#ffffff"),
        segments[1]["caption_color"].as_str(),
        "a populated segment field must reach the wire"
    );
}
