//! Recipe lowering tests: each recipe either enriches/reorders an existing
//! item or, for the lower third only, appends the certified template the
//! producer never used. No recipe may duplicate a fact.

use super::super::request::SceneAnnotationsV2;
use super::visual_recipes::apply_visual_recipes;
use super::*;

fn annotations(recipe: &str, payload: Value) -> SceneAnnotationsV2 {
    SceneAnnotationsV2 {
        visual_recipe: recipe.to_string(),
        visual_payload: Some(payload),
        ..SceneAnnotationsV2::default()
    }
}

fn number_item(text: &str) -> PlanItemV2 {
    PlanItemV2 {
        id: "scene-1-number-0".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "number".to_string(),
        template_id: "NUMBER".to_string(),
        text: text.to_string(),
        start_us: 500_000,
        duration_us: 1_000_000,
        start_ms: 500,
        end_ms: 1_500,
        ..PlanItemV2::default()
    }
}

#[test]
fn counter_odometer_enriches_the_existing_number_card() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "counter_odometer",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "counter_odometer",
            "subject_label": "Debito",
            "value": 12.0,
            "unit_surface": "%",
        }),
    );
    let mut items = vec![number_item("12%")];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 1, "the recipe must not add a second card");
    assert_eq!(items[0].text, "12%\nDebito");
    assert_eq!(items[0].template_id, "METRIC_STAT_CARD");
    assert!(!items[0].motion_id.is_empty());
    assert_eq!(
        items[0]
            .params
            .as_ref()
            .and_then(|params| params.get("font_size_px"))
            .and_then(Value::as_f64),
        Some(110.0)
    );
}

#[test]
fn counter_odometer_without_its_number_annotation_is_a_no_op() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "counter_odometer",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "counter_odometer",
            "subject_label": "Debito",
            "value": 99.0,
            "unit_surface": "%",
        }),
    );
    let mut items = vec![number_item("12%")];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items[0].text, "12%");
    assert_eq!(items[0].template_id, "NUMBER");
}

#[test]
fn counter_odometer_accepts_localized_thousands_grouping() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "counter_odometer",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "counter_odometer",
            "subject_label": "Popolazione",
            "value": 1234.0,
            "unit_surface": "",
        }),
    );
    let mut items = vec![number_item("1.234")];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items[0].text, "1.234\nPopolazione");
}

#[test]
fn role_lower_third_uses_the_entity_occurrence_window() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "role_lower_third",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "role_lower_third",
            "person_id": "person-tim",
            "person_label": "Tim Cook",
            "role_surface": "CEO",
            "organization_label": "Apple",
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 1, "{items:?}");
    let card = &items[0];
    assert_eq!(card.kind, "lower_third");
    assert_eq!(card.template_id, "LOWER_THIRD");
    assert_eq!(card.text, "Tim Cook\nCEO, Apple");
    assert_eq!(card.start_us, 200_000);
    assert_eq!(card.duration_us, 1_800_000, "clamped to the scene end");
}

#[test]
fn role_lower_third_without_a_timeline_occurrence_is_a_no_op() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "role_lower_third",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "role_lower_third",
            "person_label": "Nobody Here",
            "role_surface": "CEO",
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert!(items.is_empty());
}

#[test]
fn role_lower_third_without_a_role_is_a_no_op() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "role_lower_third",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "role_lower_third",
            "person_label": "Tim Cook",
            "role_surface": "",
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert!(items.is_empty());
}

#[test]
fn quote_attribution_credits_the_existing_quote_item() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "quote_attribution",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "quote_attribution",
            "quote_text": "«Non molliamo»",
            "author_label": "Mario Rossi",
            "role_surface": "sindaco",
        }),
    );
    let mut items = vec![PlanItemV2 {
        id: "scene-1-quote-0".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "quote".to_string(),
        template_id: "QUOTE".to_string(),
        text: "«Non molliamo»".to_string(),
        ..PlanItemV2::default()
    }];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].text, "«Non molliamo»\n— Mario Rossi, sindaco");
}

#[test]
fn quote_attribution_does_not_invent_a_quote_item() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "quote_attribution",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "quote_attribution",
            "quote_text": "«Non molliamo»",
            "author_label": "Mario Rossi",
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert!(items.is_empty());
}

#[test]
fn source_highlight_pins_the_certified_motion_on_the_named_phrase() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "source_highlight",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "source_highlight",
            "source_label": "ANSA",
            "highlight_text": "Secondo ANSA, il debito è salito.",
        }),
    );
    let mut items = vec![
        PlanItemV2 {
            id: "scene-1-phrase-0".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "text_phrase".to_string(),
            template_id: "IMPORTANT_PHRASE".to_string(),
            text: "Secondo ANSA, il debito è salito.".to_string(),
            ..PlanItemV2::default()
        },
        PlanItemV2 {
            id: "scene-1-phrase-1".to_string(),
            scene_id: "scene-1".to_string(),
            kind: "text_phrase".to_string(),
            template_id: "IMPORTANT_PHRASE".to_string(),
            text: "Il museo chiude.".to_string(),
            ..PlanItemV2::default()
        },
    ];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items[0].motion_id, "doc_04_archival_source_highlight");
    assert!(items[1].motion_id.is_empty());
}

#[test]
fn source_highlight_respects_an_existing_motion_pin() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "source_highlight",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "source_highlight",
            "source_label": "ANSA",
            "highlight_text": "Secondo ANSA, il debito è salito.",
        }),
    );
    let mut items = vec![PlanItemV2 {
        id: "scene-1-phrase-0".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "text_phrase".to_string(),
        template_id: "IMPORTANT_PHRASE".to_string(),
        text: "Secondo ANSA, il debito è salito.".to_string(),
        motion_id: "typewriter_key_strike".to_string(),
        ..PlanItemV2::default()
    }];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items[0].motion_id, "typewriter_key_strike");
}

#[test]
fn route_map_reorders_the_existing_pins() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "route_map",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "route_map",
            "stops": [{"name": "Roma"}, {"name": "Parigi"}, {"name": "Berlino"}],
        }),
    );
    let map = |id: &str, label: &str| PlanItemV2 {
        id: id.to_string(),
        scene_id: "scene-1".to_string(),
        kind: "map".to_string(),
        template_id: "MAP".to_string(),
        map: Some(json!({"entity_id": label, "label": label})),
        ..PlanItemV2::default()
    };
    let mut items = vec![map("m1", "Parigi"), map("m2", "Berlino"), map("m3", "Roma")];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    let labels: Vec<&str> = items
        .iter()
        .map(|item| {
            item.map
                .as_ref()
                .and_then(|map| map.get("label"))
                .and_then(Value::as_str)
                .unwrap()
        })
        .collect();
    assert_eq!(labels, ["Roma", "Parigi", "Berlino"]);
}

fn recipe(recipe: &str, payload: Value) -> super::super::request::VisualRecipeSpecV2 {
    super::super::request::VisualRecipeSpecV2 {
        recipe: recipe.to_string(),
        payload: Some(payload),
    }
}

#[test]
fn one_scene_applies_every_listed_recipe_once() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = SceneAnnotationsV2 {
        visual_recipes: vec![
            recipe(
                "counter_odometer",
                json!({
                    "schema": "visual_recipe.v1",
                    "recipe": "counter_odometer",
                    "subject_label": "Debito",
                    "value": 12.0,
                }),
            ),
            recipe(
                "role_lower_third",
                json!({
                    "schema": "visual_recipe.v1",
                    "recipe": "role_lower_third",
                    "person_id": "person-tim",
                    "person_label": "Tim Cook",
                    "role_surface": "CEO",
                }),
            ),
        ],
        ..SceneAnnotationsV2::default()
    };
    let mut items = vec![number_item("12%")];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0].text, "12%\nDebito");
    assert_eq!(items[1].kind, "lower_third");
    assert_eq!(items[1].text, "Tim Cook\nCEO");
}

#[test]
fn legacy_pair_and_its_list_twin_apply_once() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let payload = json!({
        "schema": "visual_recipe.v1",
        "recipe": "counter_odometer",
        "subject_label": "Debito",
        "value": 12.0,
    });
    let annotations = SceneAnnotationsV2 {
        visual_recipe: "counter_odometer".to_string(),
        visual_payload: Some(payload.clone()),
        visual_recipes: vec![recipe("counter_odometer", payload)],
        ..SceneAnnotationsV2::default()
    };
    let mut items = vec![number_item("12%")];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].text, "12%\nDebito", "one enrichment, not two");
}

#[test]
fn a_repeated_counter_recipe_does_not_stack_a_second_line() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let payload = json!({
        "schema": "visual_recipe.v1",
        "recipe": "counter_odometer",
        "subject_label": "Debito",
        "value": 12.0,
    });
    let annotations = SceneAnnotationsV2 {
        visual_recipes: vec![
            recipe("counter_odometer", payload.clone()),
            recipe("counter_odometer", payload),
        ],
        ..SceneAnnotationsV2::default()
    };
    let mut items = vec![number_item("12%")];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items[0].text, "12%\nDebito");
}

#[test]
fn an_absent_visual_recipe_list_serializes_without_the_key() {
    let scene = SceneAnnotationsV2 {
        visual_recipe: "route_map".to_string(),
        visual_payload: Some(json!({"schema": "visual_recipe.v1", "recipe": "route_map"})),
        ..SceneAnnotationsV2::default()
    };
    let value = serde_json::to_value(&scene).expect("annotations serialize");
    assert!(
        value.get("visual_recipes").is_none(),
        "a frozen request must stay byte-identical: {value}"
    );
}

#[test]
fn layout_only_recipes_stay_validated_but_inert() {
    // TICKET-48: the contract accepts these three; the planner must not lower
    // them until a certified layout exists. The fixture
    // `visual_recipes.request.json` proves the validation half.
    for recipe_name in [
        "relationship_graph",
        "choropleth_value_map",
        "lifespan_timeline",
    ] {
        let request = super::tests::request();
        let scene = request.scenes[0].clone();
        let annotations = annotations(
            recipe_name,
            json!({"schema": "visual_recipe.v1", "recipe": recipe_name}),
        );
        let mut items = vec![number_item("12%")];
        apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
        assert_eq!(items.len(), 1, "{recipe_name} must not add items");
        assert_eq!(items[0].text, "12%", "{recipe_name} must not mutate items");
    }
}

#[test]
fn relative_time_jump_enriches_the_existing_date_card() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "relative_time_jump",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "relative_time_jump",
            "surface": "Nike",
            "delta_label": "+3 ANNI",
            "resolved_date": "2021",
        }),
    );
    let mut items = vec![PlanItemV2 {
        id: "scene-1-number-date".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "number".to_string(),
        template_id: "TIMELINE_DATE_CARD".to_string(),
        text: "2021".to_string(),
        ..PlanItemV2::default()
    }];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(
        items.len(),
        1,
        "the recipe must reuse the date card: {items:?}"
    );
    assert_eq!(items[0].text, "2021\n+3 ANNI");
    assert_eq!(items[0].template_id, "TIMELINE_DATE_CARD");
    assert!(!items[0].motion_id.is_empty(), "the card keeps an entrance");
}

#[test]
fn relative_time_jump_without_a_date_item_appends_one_on_the_spoken_surface() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "relative_time_jump",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "relative_time_jump",
            "surface": "Nike",
            "delta_label": "+3 ANNI",
            "resolved_date": "2021",
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].kind, "number");
    assert_eq!(items[0].template_id, "TIMELINE_DATE_CARD");
    assert_eq!(items[0].text, "2021\n+3 ANNI");
    assert_eq!(items[0].start_us, 100_000, "anchored on the spoken surface");
}

#[test]
fn cause_effect_card_appends_one_two_slot_phrase() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "cause_effect_card",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "cause_effect_card",
            "cause_text": "Nike",
            "effect_text": "museo",
            "cause_evidence": "Nike",
            "effect_evidence": "al museo",
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].kind, "text_phrase");
    assert_eq!(items[0].text, "CAUSA: Nike\n→ EFFETTO: museo");
    assert_eq!(items[0].start_us, 100_000);
    assert!(!items[0].motion_id.is_empty());
}

#[test]
fn cause_effect_card_is_a_no_op_when_the_phrase_already_names_both_slots() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "cause_effect_card",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "cause_effect_card",
            "cause_text": "Nike",
            "effect_text": "museo",
            "cause_evidence": "Nike",
        }),
    );
    let mut items = vec![PlanItemV2 {
        id: "scene-1-phrase".to_string(),
        scene_id: "scene-1".to_string(),
        kind: "text_phrase".to_string(),
        template_id: "IMPORTANT_PHRASE".to_string(),
        text: "Nike corre al museo.".to_string(),
        ..PlanItemV2::default()
    }];
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 1, "no duplicate card: {items:?}");
    assert_eq!(items[0].text, "Nike corre al museo.");
}

#[test]
fn process_stepper_appends_each_grounded_step_in_source_order() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "process_stepper",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "process_stepper",
            "steps": [
                {"index": 1, "text": "Nike", "evidence": "Nike"},
                {"index": 2, "text": "museo", "evidence": "museo"}
            ],
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0].text, "1. Nike");
    assert_eq!(items[1].text, "2. museo");
    assert_eq!(items[0].start_us, 100_000);
    assert_eq!(items[1].start_us, 1_000_000);
    assert!(items
        .iter()
        .all(|item| item.template_id == "IMPORTANT_PHRASE" && !item.motion_id.is_empty()));
}

#[test]
fn process_stepper_drops_steps_without_certified_word_timing() {
    let request = super::tests::request();
    let scene = request.scenes[0].clone();
    let annotations = annotations(
        "process_stepper",
        json!({
            "schema": "visual_recipe.v1",
            "recipe": "process_stepper",
            "steps": [
                {"index": 1, "text": "Nike", "evidence": "Nike"},
                {"index": 2, "text": "parole mai pronunciate", "evidence": "parole mai pronunciate"}
            ],
        }),
    );
    let mut items = Vec::new();
    apply_visual_recipes(&mut items, &scene, &annotations, &request, 0);
    assert_eq!(
        items.len(),
        1,
        "only the grounded step is placed: {items:?}"
    );
    assert_eq!(items[0].text, "1. Nike");
}
