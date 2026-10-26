//! Ranking, caps, evidence rule and determinism invariants.

use super::super::*;
use super::common::*;
use crate::ExtractOptions;

#[test]
fn numeric_value_survives_a_low_entity_cap() {
    // Four identities outrank the value; with a cap of 3 the value must still
    // be kept, otherwise the chart loses its number.
    let text = "Alice Johnson met Bob Smith in London, Paris, Berlin and Madrid, where sales reached 1,200 units.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 3,
        },
    )
    .unwrap();
    assert_eq!(entities.len(), 3);
    assert!(
        entities
            .iter()
            .any(|e| matches!(e.r#type.as_str(), "NUMBER" | "PERCENT" | "MONEY")),
        "no numeric value in {:?}",
        texts(&entities)
    );
}

// TestEntitiesRequireSourceEvidence — the killer rule.
// "Imagine the" / "ready" must NEVER become entities because they are
// stopword-seeded and never produce candidates in the first place.

#[test]
fn imagine_the_and_ready_are_rejected() {
    let entities = top3("Imagine the vibrant world of Greek cuisine, ready to discover...");
    let surfaces: Vec<&str> = entities.iter().map(|e| e.text.as_str()).collect();
    assert!(
        !surfaces
            .iter()
            .any(|s| s.to_lowercase().contains("imagine")),
        "Imagine the must not become an entity: {:?}",
        surfaces
    );
    assert!(
        !surfaces.iter().any(|s| s.to_lowercase() == "ready"),
        "ready must not become an entity: {:?}",
        surfaces
    );
    assert!(
        !surfaces
            .iter()
            .any(|s| s.to_lowercase().contains("discover")),
        "discover must not become an entity: {:?}",
        surfaces
    );
}

// TestExactEntityLimit — requesting 3 returns exactly 3 when the text
// has ≥3 candidates.

#[test]
fn exactly_three_entities_returned() {
    let entities =
        top3("Greek salad combines fresh tomatoes, cucumbers, olives, feta cheese and olive oil.");
    assert_eq!(
        entities.len(),
        3,
        "expected exactly 3 entities, got {entities:?}"
    );
}

// Golden-fixture regression, domain-free: the scorer has no dish or
// ingredient vocabulary, so the guarantees here are structural — a multi-word
// noun phrase outranks singletons on phrase length alone, every retained
// entity keeps its verbatim evidence, and no candidate outside the source
// can appear (NO EVIDENCE → NO ENTITY).

#[test]
fn multiword_phrases_outrank_singletons_and_stay_grounded() {
    let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
    let surfaces = texts(&entities);
    assert!(
        surfaces.iter().any(|s| s.to_lowercase() == "feta cheese"),
        "feta cheese missing: {surfaces:?}"
    );
    assert_eq!(
        surfaces[0].to_lowercase(),
        "feta cheese",
        "the multi-word phrase must win on phrase length alone: {surfaces:?}"
    );
    // All 3 must be source-grounded.
    for e in &entities {
        assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
        assert_eq!(e.evidence, e.text, "evidence must equal text verbatim");
    }
}

// Hummus regression — the second golden-fixture segment.
// Expected candidates include hummus, chickpeas, tahini, lemon juice, olive oil;
// top 3 must all be source-grounded.

#[test]
fn hummus_returns_source_grounded_entities() {
    let entities =
        top3("Hummus is traditionally made with chickpeas, tahini, lemon juice and olive oil.");
    // All returned entities must be source-grounded (NO EVIDENCE → NO ENTITY).
    for e in &entities {
        assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
        assert_eq!(
            e.evidence, e.text,
            "evidence must equal text verbatim for {}",
            e.text
        );
    }
    // The top 3 must come from the expected candidate set.
    let expected = ["hummus", "chickpeas", "tahini", "lemon juice", "olive oil"];
    for e in &entities {
        let lower = e.text.to_lowercase();
        assert!(
            expected.iter().any(|s| *s == lower),
            "unexpected entity {}: not in {expected:?}",
            e.text
        );
    }
}

#[test]
fn entity_candidates_remain_verbatim_spans_and_keep_numeric_values() {
    let source = "Tesla reported $2.5 billion in revenue and employed 5,000 workers.";
    let entities = extract(
        source,
        &ExtractOptions {
            language: "en".to_string(),
            entity_count: 20,
        },
    )
    .unwrap();
    assert!(entities.iter().any(|entity| entity.text == "Tesla"));
    assert!(entities
        .iter()
        .any(|entity| entity.r#type == "MONEY" && entity.text == "$2.5 billion"));
    assert!(entities
        .iter()
        .any(|entity| entity.r#type == "NUMBER" && entity.text == "5,000"));
    for entity in &entities {
        assert_eq!(&source[entity.start..entity.end], entity.text);
        assert_eq!(entity.evidence, entity.text);
    }
}

#[test]
fn no_entity_without_evidence() {
    let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
    for e in &entities {
        // evidence must be a non-empty verbatim slice of the source.
        assert!(e.start < e.end, "empty span for {}", e.text);
        assert!(!e.evidence.is_empty(), "empty evidence for {}", e.text);
        assert_eq!(
            e.evidence, e.text,
            "evidence must equal text verbatim for {}",
            e.text
        );
    }
}

// Determinism: 100 runs must produce the same winner set.
