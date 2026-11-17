//! Contextual query composition for inflected Polish/Russian places.

mod common;

use common::*;

fn queries(language: &str, id: &str, text: &str) -> Vec<String> {
    let value = manifest_value_for(language, vec![scene(id, text)]);
    value["scenes"][0]["contextual_image_queries"]
        .as_array()
        .unwrap_or_else(|| panic!("contextual queries expected: {value}"))
        .iter()
        .filter_map(|query| query["query"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn polish_inflected_place_uses_the_canonical_nominative() {
    let text = "Stadion Północny znajduje się w Poznaniu i został otwarty w 1976 roku.";
    assert!(
        queries("pl", "pl-place", text)
            .iter()
            .any(|query| query == "Stadion Północny Poznań"),
        "canonical Polish query expected"
    );
}

#[test]
fn russian_inflected_place_keeps_the_source_case() {
    let text = "Стадион Северный находится в Казани и был открыт в 1977 году.";
    let value = manifest_value_for("ru", vec![scene("ru-place", text)]);
    let scene = &value["scenes"][0];
    let actual: Vec<&str> = scene["contextual_image_queries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|query| query["query"].as_str())
        .collect();
    let entities: Vec<(&str, &str)> = scene["entities"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|entity| Some((entity["text"].as_str()?, entity["type"].as_str()?)))
        .collect();
    assert!(
        actual
            .iter()
            .any(|query| *query == "Стадион Северный Казани"),
        "source-case Russian query expected: queries={actual:?} entities={entities:?}"
    );
}
