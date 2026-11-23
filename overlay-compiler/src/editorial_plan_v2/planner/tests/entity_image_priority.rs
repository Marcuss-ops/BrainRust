// Regression tests for the verified-portrait priority (Augias R41): a scene
// whose ten overlay slots are already filled by numbers/phrases must still
// admit the named card that carries a verified portrait, otherwise the run
// reaches the final-job preflight with zero verified image overlays.

use serde_json::json;

#[test]
fn a_dense_scene_admits_the_verified_portrait_over_a_number() {
    let mut request = request();
    request.budgets.as_mut().expect("budgets").max_per_scene = 2;
    request.scenes[0].annotations = Some(
        serde_json::from_value(json!({
            "numbers": [
                {"text": "1992", "type": "NUMBER", "start_us": 300000, "duration_us": 300000, "score": 0.9},
                {"text": "1995", "type": "NUMBER", "start_us": 700000, "duration_us": 300000, "score": 0.9}
            ]
        }))
        .expect("annotations"),
    );
    let compiled = compile_plan(&request).expect("plan");
    assert!(
        compiled
            .plan
            .items
            .iter()
            .any(|item| item.kind == "entity_image" && item.entity_id == "person-tim"),
        "the verified portrait must survive a full per-scene ceiling: {:?}",
        compiled
            .plan
            .items
            .iter()
            .map(|item| (item.kind.clone(), item.text.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        compiled
            .plan
            .intake
            .dropped_by_reason
            .get("per_scene")
            .copied()
            .unwrap_or(0)
            >= 1,
        "every candidate that lost its slot to the portrait is accounted for: {:?}",
        compiled.plan.intake.dropped_by_reason
    );
}

#[test]
fn a_verified_portrait_displaces_a_generic_still_at_the_image_ceiling() {
    let mut request = request();
    request.scenes[0].annotations = Some(
        serde_json::from_value(json!({
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
    let images: Vec<&str> = compiled
        .plan
        .items
        .iter()
        .filter(|item| item.kind == "image" || item.kind == "entity_image")
        .map(|item| item.kind.as_str())
        .collect();
    assert_eq!(vec!["entity_image"], images);
    assert_eq!(1, compiled.plan.intake.dropped_by_reason["budget"]);
}
