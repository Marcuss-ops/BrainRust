//! End-to-end proof that the semantic engine emits the new grounded visual
//! recipes, not just the raw groups/dates they come from.

mod common;

use common::*;
use serde_json::Value;

fn recipe<'a>(value: &'a Value, name: &str) -> &'a Value {
    value["visual_consolidation"]["production_candidates"]
        .as_array()
        .expect("production candidates")
        .iter()
        .find(|candidate| candidate["recipe"] == name)
        .unwrap_or_else(|| panic!("missing {name} recipe: {}", value["visual_consolidation"]))
}

#[test]
fn resolved_relative_date_emits_a_grounded_time_jump() {
    let text = "Nel 2015 lo stadio aprì. Tre anni dopo lo stadio chiuse.";
    let value = manifest_value(vec![scene("salto-temporale", text)]);
    let candidate = recipe(&value, "relative_time_jump");
    let payload = &candidate["visual_payload"];
    assert_eq!(payload["recipe"], "relative_time_jump");
    assert_eq!(payload["resolved_date"], "2018");
    assert_eq!(payload["delta_label"], "+3 ANNI");
    assert_eq!(payload["direction"], "after");
    assert_eq!(payload["amount"], 3);
    assert_eq!(payload["unit"], "year");
    let start = candidate["start_byte"].as_u64().unwrap() as usize;
    let end = candidate["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(start..end), candidate["evidence"].as_str());
    assert_eq!(candidate["scene_id"], "salto-temporale");
}

#[test]
fn asserted_causal_relation_emits_a_grounded_cause_effect_card() {
    let text = "Lo Stadio Aurora causa l'espansione di Torino.";
    let value = manifest_value(vec![scene("causa-effetto", text)]);
    let candidate = recipe(&value, "cause_effect_card");
    let payload = &candidate["visual_payload"];
    assert_eq!(payload["recipe"], "cause_effect_card");
    assert_eq!(payload["cause_text"], "Stadio Aurora");
    assert_eq!(payload["effect_text"], "Torino");
    for key in ["cause_start_byte", "cause_end_byte"] {
        assert!(payload[key].as_u64().is_some(), "missing {key}: {payload}");
    }
    let start = candidate["start_byte"].as_u64().unwrap() as usize;
    let end = candidate["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(start..end), candidate["evidence"].as_str());
    let cause_start = payload["cause_start_byte"].as_u64().unwrap() as usize;
    let cause_end = payload["cause_end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(cause_start..cause_end), Some("Stadio Aurora"));
    let effect_start = payload["effect_start_byte"].as_u64().unwrap() as usize;
    let effect_end = payload["effect_end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(effect_start..effect_end), Some("Torino"));
}

#[test]
fn explicit_process_phases_emit_a_grounded_stepper() {
    let text = "Lo Stadio Aurora fu progettato. La costruzione dello Stadio Aurora iniziò. Lo Stadio Aurora fu inaugurato.";
    let value = manifest_value(vec![scene("processo", text)]);
    let candidate = recipe(&value, "process_stepper");
    let payload = &candidate["visual_payload"];
    assert_eq!(payload["recipe"], "process_stepper");
    let steps = payload["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 3, "{steps:?}");
    for (index, step) in steps.iter().enumerate() {
        assert_eq!(step["index"], index + 1);
        let start = step["start_byte"].as_u64().unwrap() as usize;
        let end = step["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), step["evidence"].as_str());
        assert!(step["evidence"].as_str().unwrap().contains("Stadio Aurora"));
    }
    let start = candidate["start_byte"].as_u64().unwrap() as usize;
    let end = candidate["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(text.get(start..end), candidate["evidence"].as_str());
}
