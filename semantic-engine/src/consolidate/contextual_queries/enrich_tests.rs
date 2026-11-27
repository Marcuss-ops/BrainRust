use super::*;
use semantic_core::{Candidate as ExtractedCandidate, RelationView, SceneManifest};

fn extracted(id: &str, text: &str, kind: &str, start: usize, end: usize) -> ExtractedCandidate {
    ExtractedCandidate {
        id: id.to_string(),
        text: text.to_string(),
        kind: kind.to_string(),
        start_byte: start,
        end_byte: end,
        sentence_index: 0,
        evidence: text.to_string(),
        score: 1.0,
        provenance: "test".to_string(),
    }
}

fn scene(scene_id: &str, entities: Vec<ExtractedCandidate>) -> SceneManifest {
    SceneManifest {
        scene_id: scene_id.to_string(),
        status: "valid".to_string(),
        reason: None,
        entities,
        identities: Vec::new(),
        person_roles: Vec::new(),
        age_facts: Vec::new(),
        entity_attribute_lists: Vec::new(),
        person_relations: Vec::new(),
        places: Vec::new(),
        routes: Vec::new(),
        relations: RelationView::default(),
        numeric_facts: Vec::new(),
        quantities: Vec::new(),
        quantity_comparisons: Vec::new(),
        temporal_facts: Vec::new(),
        entity_facts: Vec::new(),
        bullets: Vec::new(),
        short_phrases: Vec::new(),
        long_phrases: Vec::new(),
        quotes: Vec::new(),
        important_words: Vec::new(),
        important_segments: Vec::new(),
        composed_image_queries: Vec::new(),
        contextual_image_queries: Vec::new(),
        visual_proposals: Vec::new(),
        semantic_groups: Vec::new(),
        micro_summaries: Vec::new(),
        rejected_items: 0,
    }
}

fn entity_candidate(
    scene_id: &str,
    id: &str,
    title: &str,
    start: usize,
    end: usize,
) -> ProductionCandidate {
    ProductionCandidate {
        id: String::new(),
        recipe: "entity_search".to_string(),
        rank: 0,
        priority: 0.8,
        scene_id: scene_id.to_string(),
        scene_ids: vec![scene_id.to_string()],
        entity_ids: vec![id.to_string()],
        fact_ids: Vec::new(),
        group_ids: Vec::new(),
        title: title.to_string(),
        evidence: title.to_string(),
        start_byte: start,
        end_byte: end,
        suggested_visual: "search_asset".to_string(),
        chart_items: Vec::new(),
        chart_meta: None,
        visual_payload: None,
        temporal_surfaces: Vec::new(),
        asset_key: format!("entity:{id}"),
        queries: Vec::new(),
        requires_media: true,
        dedup_key: format!("entity:{id}:search"),
        covered_fact_ids: Vec::new(),
        rationale: Vec::new(),
    }
}

fn component(kind: &str, text: &str, start: usize, end: usize) -> QueryComponent {
    QueryComponent {
        kind: "entity".to_string(),
        component_type: kind.to_string(),
        text: text.to_string(),
        start_byte: start,
        end_byte: end,
    }
}

fn own_context(
    source: &str,
    ada_start: usize,
    ada_end: usize,
    paris_start: usize,
    paris_end: usize,
) -> ContextualImageQuery {
    let evidence_start = 0;
    let evidence_end = source.len();
    ContextualImageQuery {
        id: "ctx_1".to_string(),
        kind: "contextual_image_query".to_string(),
        query: "Ada Lovelace Paris".to_string(),
        evidence: source.to_string(),
        evidence_start_byte: evidence_start,
        evidence_end_byte: evidence_end,
        sentence_index: 0,
        components: vec![
            component("PERSON", "Ada Lovelace", ada_start, ada_end),
            component("LOCATION", "Paris", paris_start, paris_end),
        ],
    }
}

#[test]
fn an_owned_context_is_searched_before_the_literal_fallback() {
    let source = "Ada Lovelace built a bridge in Paris.";
    let ada_start = source.find("Ada Lovelace").unwrap();
    let ada_end = ada_start + "Ada Lovelace".len();
    let paris_start = source.find("Paris").unwrap();
    let paris_end = paris_start + "Paris".len();
    let sources = vec![SceneInput {
        scene_id: "s1".to_string(),
        text: source.to_string(),
    }];
    let mut scene_manifest = scene(
        "s1",
        vec![
            extracted("person_ada", "Ada Lovelace", "PERSON", ada_start, ada_end),
            extracted("object_bridge", "bridge", "OBJECT", 21, 27),
            extracted("place_paris", "Paris", "LOCATION", paris_start, paris_end),
        ],
    );
    scene_manifest.contextual_image_queries = vec![own_context(
        source,
        ada_start,
        ada_end,
        paris_start,
        paris_end,
    )];
    let mut candidates = vec![entity_candidate(
        "s1",
        "person_ada",
        "Ada Lovelace",
        ada_start,
        ada_end,
    )];
    enrich_contextual_search_queries(
        "en",
        &sources,
        &[scene_manifest],
        &HashMap::new(),
        &mut candidates,
    );
    let candidate = &candidates[0];
    assert!(candidate.requires_media);
    assert_eq!(candidate.queries.len(), 2);
    assert_eq!(candidate.queries[0].origin, "derived_from_source_context");
    assert_eq!(candidate.queries[0].text, "Ada Lovelace Paris");
    assert_eq!(candidate.queries[1].origin, "literal");
    assert_eq!(candidate.queries[1].text, "Ada Lovelace");
}

#[test]
fn a_context_owned_by_another_identity_suppresses_the_search() {
    let source = "Ada Lovelace built a bridge in Paris.";
    let ada_start = source.find("Ada Lovelace").unwrap();
    let ada_end = ada_start + "Ada Lovelace".len();
    let bridge_start = source.find("bridge").unwrap();
    let bridge_end = bridge_start + "bridge".len();
    let paris_start = source.find("Paris").unwrap();
    let paris_end = paris_start + "Paris".len();
    let sources = vec![SceneInput {
        scene_id: "s1".to_string(),
        text: source.to_string(),
    }];
    let mut scene_manifest = scene(
        "s1",
        vec![
            extracted("person_ada", "Ada Lovelace", "PERSON", ada_start, ada_end),
            extracted(
                "object_bridge",
                "bridge",
                "OBJECT",
                bridge_start,
                bridge_end,
            ),
            extracted("place_paris", "Paris", "LOCATION", paris_start, paris_end),
        ],
    );
    scene_manifest.contextual_image_queries = vec![ContextualImageQuery {
        id: "ctx_1".to_string(),
        kind: "contextual_image_query".to_string(),
        query: "Ada Lovelace bridge Paris".to_string(),
        evidence: source.to_string(),
        evidence_start_byte: 0,
        evidence_end_byte: source.len(),
        sentence_index: 0,
        components: vec![
            component("PERSON", "Ada Lovelace", ada_start, ada_end),
            component("OBJECT", "bridge", bridge_start, bridge_end),
            component("LOCATION", "Paris", paris_start, paris_end),
        ],
    }];
    let mut candidates = vec![
        entity_candidate("s1", "person_ada", "Ada Lovelace", ada_start, ada_end),
        entity_candidate("s1", "object_bridge", "bridge", bridge_start, bridge_end),
    ];
    enrich_contextual_search_queries(
        "en",
        &sources,
        &[scene_manifest],
        &HashMap::new(),
        &mut candidates,
    );
    assert!(!candidates[0].requires_media);
    assert!(candidates[0].queries.is_empty());
    // The lower-ranked object owns and receives the contextual rung.
    assert!(candidates[1].requires_media);
    assert_eq!(
        candidates[1].queries[0].origin,
        "derived_from_source_context"
    );
}

#[test]
fn a_configured_non_subject_object_is_not_searched() {
    let Some(term) = image_query_policy()
        .non_subject_object_terms
        .first()
        .cloned()
    else {
        return;
    };
    let source = "The value appears on screen.";
    let sources = vec![SceneInput {
        scene_id: "s1".to_string(),
        text: source.to_string(),
    }];
    let mut candidates = vec![entity_candidate("s1", "object_value", &term, 4, 9)];
    enrich_contextual_search_queries(
        "en",
        &sources,
        &[scene("s1", Vec::new())],
        &HashMap::new(),
        &mut candidates,
    );
    assert!(!candidates[0].requires_media);
    assert!(candidates[0].queries.is_empty());
}
