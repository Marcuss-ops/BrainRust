use std::collections::{BTreeSet, HashSet};

use semantic_engine::{analyze_script, SceneInput, ScriptRequest};
use serde_json::Value;

fn scene(id: &str, text: &str) -> SceneInput {
    SceneInput {
        scene_id: id.to_string(),
        text: text.to_string(),
    }
}

fn request(scenes: Vec<SceneInput>) -> ScriptRequest {
    ScriptRequest {
        language: "it".to_string(),
        scenes,
        editorial: None,
        entity_safety_cap: 0,
    }
}

fn manifest(scenes: Vec<SceneInput>) -> Value {
    serde_json::to_value(analyze_script(&request(scenes)).expect("analysis succeeds"))
        .expect("manifest serializes")
}

#[test]
fn consolidation_dedupes_facts_queries_and_proposals_without_deleting_inventory() {
    let scenes = vec![scene(
        "garlasco",
        "Alberto Stasi fu assolto il 13 agosto 2007. La Cassazione confermò l'assoluzione il 19 settembre 2007. Andrea Sempio è indagato. Sempio è indagato. Sempio respinse le accuse. Andrea Sempio non è stato condannato. L'impronta indicata come numero 33 è un reperto.",
    )];
    let value = manifest(scenes);
    let consolidation = &value["visual_consolidation"];

    let raw_facts: usize = value["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|scene| scene["entity_facts"].as_array().unwrap().len())
        .sum();
    let raw_proposals: usize = value["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|scene| scene["visual_proposals"].as_array().unwrap().len())
        .sum();
    let raw_queries: usize = value["search_feedback"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["queries"].as_array().unwrap().len())
        .sum();

    assert_eq!(consolidation["inventory"]["facts"], raw_facts);
    assert_eq!(consolidation["inventory"]["proposals"], raw_proposals);
    assert_eq!(consolidation["inventory"]["queries"], raw_queries);
    assert_eq!(
        consolidation["inventory"]["canonical_facts"],
        consolidation["facts"].as_array().unwrap().len()
    );
    assert!(
        consolidation["metrics"]["duplicate_facts_merged"]
            .as_u64()
            .unwrap()
            >= 1,
        "the repeated Sempio under_investigation claim must be one canonical fact: {consolidation}"
    );
    assert!(
        consolidation["metrics"]["production_candidates"]
            .as_u64()
            .unwrap()
            < raw_proposals as u64,
        "production must be selective: {consolidation}"
    );
    assert!(
        consolidation["metrics"]["production_queries"].as_u64().unwrap() < raw_queries as u64,
        "one query per canonical identity must be fewer than one ladder per mention: {consolidation}"
    );

    let facts = consolidation["facts"].as_array().unwrap();
    assert!(!facts.is_empty());
    for fact in facts {
        let represented = fact["representation"].is_object();
        let reason = fact["uncovered_reason"].as_str().unwrap_or("");
        assert!(
            represented || !reason.is_empty(),
            "every canonical fact must be represented or explicitly uncovered: {fact}"
        );
    }
    let sempio_under_investigation = facts
        .iter()
        .find(|fact| {
            fact["subject_entity_id"].as_str() == Some("person_02")
                && fact["predicate"].as_str() == Some("investigation_status")
                && fact["normalized_value"].as_str() == Some("under_investigation")
        })
        .or_else(|| {
            facts.iter().find(|fact| {
                fact["predicate"].as_str() == Some("investigation_status")
                    && fact["normalized_value"].as_str() == Some("under_investigation")
            })
        })
        .expect("under_investigation fact");
    assert!(
        sempio_under_investigation["fact_ids"]
            .as_array()
            .unwrap()
            .len()
            >= 2,
        "the repeated status mention must retain every raw fact id: {sempio_under_investigation}"
    );
    assert!(
        sempio_under_investigation["representation"]["recipe"]
            .as_str()
            .is_some_and(|recipe| recipe.contains("card")),
        "an undated status claim must be represented by a contextual card: {sempio_under_investigation}"
    );

    let reference = consolidation["reference_numbers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["surface"] == "33");
    assert!(
        reference.is_some(),
        "numero 33 must be kept as a reference number: {consolidation}"
    );
    let charts = consolidation["production_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|candidate| candidate["recipe"] == "chart");
    for chart in charts {
        for item in chart["chart_items"].as_array().unwrap_or(&Vec::new()) {
            assert_ne!(
                item["text"], "33",
                "a reference number must never enter a quantitative chart: {chart}"
            );
        }
    }

    // Completeness, not just counts: every raw fact id survives in exactly one
    // canonical fact. Consolidation selects a representation; it never drops
    // an extracted claim from the inventory.
    let mut raw_ids: Vec<String> = value["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|scene| scene["entity_facts"].as_array().unwrap())
        .map(|fact| fact["id"].as_str().unwrap().to_string())
        .collect();
    raw_ids.sort();
    let mut canonical_ids: Vec<String> = facts
        .iter()
        .flat_map(|fact| fact["fact_ids"].as_array().unwrap())
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    canonical_ids.sort();
    assert_eq!(
        canonical_ids, raw_ids,
        "every raw fact id must be retained exactly once across canonical facts"
    );

    // A representation must point at a real candidate that declares the fact.
    let candidates = consolidation["production_candidates"].as_array().unwrap();
    for fact in facts {
        let Some(representation) = fact["representation"].as_object() else {
            continue;
        };
        let candidate_id = representation["candidate_id"].as_str().unwrap();
        let candidate = candidates
            .iter()
            .find(|candidate| candidate["id"].as_str() == Some(candidate_id))
            .unwrap_or_else(|| panic!("fact points at missing candidate {candidate_id}"));
        assert!(
            candidate["covered_fact_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == &fact["id"]),
            "candidate does not declare the fact it represents: {candidate}"
        );
    }
}

#[test]
fn consolidation_keeps_one_canonical_date_with_every_original_span() {
    let value = manifest(vec![scene(
        "dates",
        "Il 13 agosto 2007 Chiara Poggi viene uccisa. Nel 2009 Alberto Stasi fu assolto. L'assoluzione del 2009 fu confermata nel 2011.",
    )]);
    let consolidation = &value["visual_consolidation"];
    let dates = consolidation["dates"].as_array().unwrap();
    let year = dates
        .iter()
        .find(|date| date["normalized_date"] == "2009")
        .expect("the repeated 2009 date must have one canonical entry");
    assert_eq!(
        year["count"], 2,
        "both 2009 mentions must be counted: {year}"
    );
    let spans = year["spans"].as_array().unwrap();
    assert_eq!(spans.len(), 2, "both original spans must survive: {year}");
    let text = "Il 13 agosto 2007 Chiara Poggi viene uccisa. Nel 2009 Alberto Stasi fu assolto. L'assoluzione del 2009 fu confermata nel 2011.";
    for span in spans {
        let start = span["start_byte"].as_u64().unwrap() as usize;
        let end = span["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&text[start..end], "2009", "span must stay verbatim");
    }
    assert_eq!(
        consolidation["inventory"]["canonical_dates"],
        dates.len(),
        "the inventory count must match the canonical date list"
    );
}

#[test]
fn consolidation_counts_a_repeated_claim_as_one_canonical_micro_summary() {
    let value = manifest(vec![
        scene("a", "Andrea Sempio è indagato."),
        scene("b", "Sempio è indagato."),
    ]);
    let consolidation = &value["visual_consolidation"];
    let inventory = &consolidation["inventory"];
    assert_eq!(
        inventory["micro_summaries"], 2,
        "both scenes keep their raw summary: {inventory}"
    );
    assert_eq!(
        inventory["canonical_micro_summaries"], 1,
        "the same subject, predicate, value and period is one canonical summary: {inventory}"
    );
    assert_eq!(
        inventory["canonical_facts"], 1,
        "the repeated status is one canonical fact: {inventory}"
    );
    let facts = consolidation["facts"].as_array().unwrap();
    assert_eq!(
        facts[0]["fact_ids"].as_array().unwrap().len(),
        2,
        "both raw facts must be retained on the canonical fact: {facts:?}"
    );
    assert!(
        facts[0]["representation"].is_object(),
        "the canonical status fact must be represented: {facts:?}"
    );
}

#[test]
fn consolidation_keeps_one_timeline_for_one_fact_set_and_emits_event_links() {
    let value = manifest(vec![scene(
        "timeline",
        "Alberto Stasi fu assolto il 13 agosto 2007. Il 19 settembre 2007 la Corte d'Appello confermò l'assoluzione. Il 26 settembre 2007 la Cassazione annullò con rinvio.",
    )]);
    let consolidation = &value["visual_consolidation"];
    let timelines: Vec<&Value> = consolidation["production_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|candidate| candidate["recipe"] == "timeline")
        .collect();
    assert_eq!(
        timelines.len(),
        1,
        "one fact set must become one timeline, not one per group: {consolidation}"
    );
    let covered = timelines[0]["covered_fact_ids"].as_array().unwrap().len();
    assert!(
        covered >= 2,
        "the timeline must cover its facts: {timelines:?}"
    );
    assert!(
        !consolidation["event_links"].as_array().unwrap().is_empty(),
        "consecutive timeline members must yield grounded event-event links: {consolidation}"
    );
    for link in consolidation["event_links"].as_array().unwrap() {
        assert_eq!(link["kind"], "precedes");
        assert!(!link["evidence"].as_str().unwrap().is_empty());
    }
}

#[test]
fn consolidation_does_not_duplicate_identity_searches_or_queries() {
    let value = manifest(vec![scene(
        "identities",
        "Alberto Stasi e Andrea Sempio sono due persone. Stasi e Sempio tornano nel racconto. Chiara Poggi è ricordata.",
    )]);
    let consolidation = &value["visual_consolidation"];
    let candidates = consolidation["production_candidates"].as_array().unwrap();
    let mut dedup_keys = HashSet::new();
    let mut search_assets = HashSet::new();
    let mut media_assets = HashSet::new();
    for candidate in candidates {
        let key = candidate["dedup_key"].as_str().unwrap();
        assert!(
            dedup_keys.insert(key.to_string()),
            "duplicate candidate: {key}"
        );
        if candidate["recipe"] == "entity_search" {
            let asset = candidate["asset_key"].as_str().unwrap();
            assert!(
                search_assets.insert(asset.to_string()),
                "one asset search per canonical identity, duplicate: {asset}"
            );
        }
        if candidate["requires_media"] == true {
            let asset = candidate["asset_key"].as_str().unwrap();
            assert!(
                media_assets.insert(asset.to_string()),
                "two media candidates must not request the same asset twice: {asset}"
            );
            assert!(
                !candidate["queries"].as_array().unwrap().is_empty(),
                "a media candidate must carry a grounded query: {candidate}"
            );
        } else {
            assert!(
                candidate["queries"].as_array().unwrap().is_empty(),
                "a non-media candidate must not reach an external search: {candidate}"
            );
        }
        let mut queries = BTreeSet::new();
        for query in candidate["queries"].as_array().unwrap() {
            let text = query["text"].as_str().unwrap();
            assert!(!text.is_empty());
            assert!(
                queries.insert(text.to_string()),
                "duplicate query in one candidate"
            );
        }
    }
    let cards = candidates
        .iter()
        .filter(|candidate| candidate["recipe"] == "entity_card")
        .count();
    assert!(
        cards >= 3,
        "named people must keep a card candidate: {consolidation}"
    );
}

#[test]
fn consolidation_types_vigevano_as_location_and_ris_as_organization() {
    let value = manifest(vec![scene(
        "typing",
        "Vigevano e Pavia entrano nel racconto attraverso le sedi. Il RIS di Parma ha acquisito documenti.",
    )]);
    let entities = value["scenes"][0]["entities"].as_array().unwrap();
    let vigevano = entities
        .iter()
        .find(|entity| entity["text"] == "Vigevano")
        .expect("Vigevano must be grounded");
    assert_eq!(vigevano["type"], "LOCATION");
    let ris = entities
        .iter()
        .find(|entity| entity["text"] == "RIS")
        .expect("RIS must be grounded");
    assert_eq!(ris["type"], "ORG");
}
