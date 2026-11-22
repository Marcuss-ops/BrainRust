use super::*;

fn pin() -> EditorialBulletStyleV2 {
    EditorialBulletStyleV2 {
        scene_id: "scene-1".to_string(),
        font_family: "oswald".to_string(),
        font_size_px: 96.0,
        motion_id: "air_rise_type_on".to_string(),
    }
}

fn bullet_item(role: &str) -> PlanItemV2 {
    PlanItemV2 {
        id: "bullet".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "text_phrase".to_string(),
        template_id: "IMPORTANT_PHRASE".to_string(),
        text: "Il debito pubblico sale al nord".to_string(),
        params: Some(json!({
            "editorial_role": role,
            "font_family": "playfair_display_italic"
        })),
        style: Some(json!({
            "font_family": "playfair_display_italic",
            "font_size": 72.0
        })),
        ..Default::default()
    }
}

#[test]
fn pin_wins_over_the_presentation_font_and_sets_the_entrance() {
    let items = apply_presentation(vec![bullet_item("scene_bullet")], "plan", &[pin()]);
    let item = &items[0];
    let params = item.params.as_ref().expect("params");
    let style = item.style.as_ref().expect("style");
    assert_eq!(Some("oswald"), params["font_family"].as_str());
    assert_eq!(Some(96.0), params["font_size_px"].as_f64());
    assert_eq!(Some("oswald"), style["font_family"].as_str());
    assert_eq!(Some(96.0), style["font_size"].as_f64());
    assert_eq!("air_rise_type_on", item.motion_id);
}

#[test]
fn chapter_bullet_uses_the_same_pin_lane() {
    let items = apply_presentation(vec![bullet_item("chapter_bullet")], "plan", &[pin()]);
    assert_eq!("air_rise_type_on", items[0].motion_id);
    assert_eq!(
        Some("oswald"),
        items[0].params.as_ref().expect("params")["font_family"].as_str()
    );
}

#[test]
fn non_bullet_text_keeps_the_presentation_default() {
    let items = apply_presentation(vec![bullet_item("")], "plan", &[pin()]);
    let item = &items[0];
    assert_ne!("air_rise_type_on", item.motion_id);
    assert_eq!(
        Some("playfair_display_italic"),
        item.params.as_ref().expect("params")["font_family"].as_str()
    );
}

#[test]
fn split_bullet_keeps_the_pin_on_every_part() {
    let mut bullet = bullet_item("scene_bullet");
    bullet.text = "Il debito pubblico sale al nord. ".repeat(10);
    bullet.duration_us = 6_000_000;
    bullet.end_ms = 6_000;
    let items = apply_presentation(vec![bullet], "plan", &[pin()]);
    assert!(items.len() > 1, "long bullet should split");
    for item in &items {
        let params = item.params.as_ref().expect("params");
        assert_eq!(Some("scene_bullet"), params["editorial_role"].as_str());
        assert_eq!(Some("oswald"), params["font_family"].as_str());
        assert_eq!(Some(96.0), params["font_size_px"].as_f64());
        assert_eq!("air_rise_type_on", item.motion_id);
    }
}

#[test]
fn long_text_inside_a_chain_splits_and_keeps_the_original_window() {
    let text = "parole di prova ".repeat(20);
    let item = PlanItemV2 {
        id: "chain".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "mixed_chain".to_string(),
        template_id: "MIXED_CHAIN".to_string(),
        text: text.clone(),
        params: Some(json!({"chain_segments": [
            {
                "kind": "text_phrase", "text": text,
                "motion_id": "air_rise_type_on", "start_us": 1_000_000,
                "duration_us": 8_000_000
            },
            {
                "kind": "entity_card", "text": "Corrado Augias",
                "start_us": 9_400_000, "duration_us": 2_000_000
            }
        ]})),
        ..Default::default()
    };

    let items = apply_presentation(vec![item], "plan", &[]);
    assert_eq!(1, items.len(), "the chain remains one overlay item");
    let params = items[0].params.as_ref().expect("chain params");
    let segments = params["chain_segments"].as_array().expect("segments");
    assert!(segments.len() > 2, "long text is split into chain segments");
    let text_segments = segments
        .iter()
        .filter(|segment| segment["kind"] == "text_phrase")
        .collect::<Vec<_>>();
    assert!(text_segments.len() > 1);
    assert!(text_segments
        .iter()
        .all(|segment| { segment["text"].as_str().unwrap().chars().count() <= MAX_TEXT_RUNES }));
    assert_eq!(1_000_000, text_segments[0]["start_us"].as_i64().unwrap());
    let last_text = text_segments.last().unwrap();
    assert_eq!(
        9_000_000,
        last_text["start_us"].as_i64().unwrap() + last_text["duration_us"].as_i64().unwrap()
    );
    assert_eq!(
        segments.len(),
        params["chain_count"].as_u64().unwrap() as usize
    );
}

#[test]
fn fused_chain_pins_only_its_bullet_segment() {
    let chain = PlanItemV2 {
        id: "chain".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "phrase_chain".to_string(),
        template_id: "PHRASE_CHAIN".to_string(),
        text: "Il debito pubblico sale al nord".to_string(),
        params: Some(json!({"chain_segments": [
            {
                "kind": "text_phrase", "template_id": "IMPORTANT_PHRASE",
                "motion_id": "motion_a", "start_us": 0, "duration_us": 2_000_000,
                "params": {"editorial_role": "scene_bullet", "font_family": "playfair_display_italic"},
                "style": {"font_family": "playfair_display_italic", "font_size": 72.0}
            },
            {
                "kind": "text_phrase", "template_id": "IMPORTANT_PHRASE",
                "motion_id": "motion_b", "start_us": 2_400_000, "duration_us": 2_000_000,
                "params": {"font_family": "playfair_display_italic"},
                "style": {"font_family": "playfair_display_italic", "font_size": 72.0}
            }
        ]})),
        ..Default::default()
    };
    let items = apply_presentation(vec![chain], "plan", &[pin()]);
    let segments = items[0].params.as_ref().expect("params")["chain_segments"]
        .as_array()
        .expect("segments");
    assert_eq!(
        "air_rise_type_on",
        segments[0]["motion_id"].as_str().unwrap()
    );
    assert_eq!(
        "oswald",
        segments[0]["params"]["font_family"].as_str().unwrap()
    );
    assert_eq!(
        96.0,
        segments[0]["params"]["font_size_px"].as_f64().unwrap()
    );
    assert_eq!(
        "oswald",
        segments[0]["style"]["font_family"].as_str().unwrap()
    );
    assert_eq!("motion_b", segments[1]["motion_id"].as_str().unwrap());
    assert_eq!(
        "playfair_display_italic",
        segments[1]["params"]["font_family"].as_str().unwrap()
    );
}

#[test]
fn number_card_preserves_an_explicit_payload_font() {
    let item = PlanItemV2 {
        id: "metric".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "number".to_string(),
        template_id: "METRIC_STAT_CARD".to_string(),
        text: "42 per cento".to_string(),
        params: Some(json!({"font_family": "oswald", "font_size_px": 88.0})),
        style: Some(json!({"font_family": "oswald", "font_size": 88.0})),
        ..Default::default()
    };
    let items = apply_presentation(vec![item], "plan", &[]);
    let params = items[0].params.as_ref().expect("params");
    let style = items[0].style.as_ref().expect("style");
    assert_eq!(Some("oswald"), params["font_family"].as_str());
    assert_eq!(Some("oswald"), style["font_family"].as_str());
    assert_eq!(Some(140.0), params["font_size_px"].as_f64());
}

#[test]
fn legacy_playfair_alias_normalizes_but_other_families_survive() {
    let item = PlanItemV2 {
        id: "date".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "number".to_string(),
        template_id: "TIMELINE_DATE_CARD".to_string(),
        text: "3 giugno 2007".to_string(),
        params: Some(json!({"font_family": "playfair_display"})),
        style: Some(json!({"font_family": "bricolage_grotesque"})),
        ..Default::default()
    };
    let items = apply_presentation(vec![item], "plan", &[]);
    let params = items[0].params.as_ref().expect("params");
    assert_eq!(
        Some("playfair_display_italic"),
        params["font_family"].as_str()
    );

    let mut other = PlanItemV2 {
        id: "number".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "number".to_string(),
        template_id: "METRIC_STAT_CARD".to_string(),
        text: "42".to_string(),
        params: Some(json!({"font_family": "abril_fatface"})),
        ..Default::default()
    };
    other.style = Some(json!({"font_family": "abril_fatface"}));
    let items = apply_presentation(vec![other], "plan", &[]);
    assert_eq!(
        Some("abril_fatface"),
        items[0].params.as_ref().expect("params")["font_family"].as_str()
    );
}

#[test]
fn chain_segment_text_is_capped_before_render() {
    let long = "parola ".repeat(120); // 840 runes
    let mut item = PlanItemV2 {
        id: "chain".to_string(),
        kind: "mixed_chain".to_string(),
        start_us: 0,
        duration_us: 12_000_000,
        params: Some(json!({
            "chain_segments": [{
                "kind": "text_phrase",
                "text": long,
                "start_us": 0,
                "duration_us": 12_000_000
            }]
        })),
        ..Default::default()
    };
    split_long_chain_text(&mut item);
    let segments = item.params.as_ref().expect("params")["chain_segments"]
        .as_array()
        .expect("segments");
    assert!(segments.len() > 1, "a 840-rune segment must be split");
    assert!(
        segments
            .iter()
            .all(|segment| { segment["text"].as_str().unwrap().chars().count() <= MAX_TEXT_RUNES }),
        "every emitted chain segment must respect the 180-rune cap"
    );
    let total: i64 = segments
        .iter()
        .map(|segment| segment["duration_us"].as_i64().unwrap())
        .sum();
    assert_eq!(total, 12_000_000, "splitting must conserve the window");
}
