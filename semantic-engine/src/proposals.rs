use super::*;

pub(crate) fn to_visual_proposal(
    candidate: VisualCandidate,
    scene_id: &str,
    index: usize,
) -> (VisualProposal, Option<SearchFeedbackEntry>) {
    let score = match candidate.kind {
        "chart" => 0.9,
        "timeline" => 0.8,
        "multiple_entities" => 0.7,
        _ => 0.6,
    };
    // Search exact identities independently. A multi-entity layout does not
    // imply that the named subjects belong in one image-search query; a joint
    // query is emitted only by the relation-grounded contextual-query path.
    let mut visual_query_suggestions: Vec<String> = Vec::new();
    let mut queries: Vec<SearchQuery> = Vec::new();
    if !candidate.entity_texts.is_empty() && candidate.kind != "entity_card" {
        for text in &candidate.entity_texts {
            if text.is_empty() || is_noise_query(text) {
                continue;
            }
            if !visual_query_suggestions.iter().any(|query| query == text) {
                visual_query_suggestions.push(text.clone());
            }
            queries.push(SearchQuery::literal(text.clone()));
        }
    }
    let proposal_id = format!("{scene_id}_visual_{:03}", index + 1);
    let feedback = candidate
        .requires_verified_images
        .then(|| SearchFeedbackEntry {
            proposal_id: proposal_id.clone(),
            scene_id: scene_id.to_string(),
            queries: queries.clone(),
            status: "pending".to_string(),
            retry: None,
        })
        .filter(|entry| !entry.queries.is_empty());
    let proposal = VisualProposal {
        id: proposal_id,
        proposal_type: candidate.kind.to_string(),
        entity_ids: candidate.entity_ids,
        visual_query_suggestions,
        suggested_visual: candidate
            .suggested_layout
            .unwrap_or(candidate.kind)
            .to_string(),
        status: "requires_semantic_validation".to_string(),
        score,
        chart_items: candidate.chart_items,
        temporal_surfaces: candidate.temporal_surfaces,
        evidence: candidate.evidence,
        start_byte: candidate.start,
        end_byte: candidate.end,
    };
    (proposal, feedback)
}

/// Noise surfaces never become search proposals. A curated filler phrase
/// ("non è così", "ancora oggi", "per questo") or an editorial heading is
/// discourse scaffolding, not a visual subject: its text stays available in
/// the scene inventory (phrase candidates), it just never becomes a query.
fn is_noise_query(text: &str) -> bool {
    let normalized = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    normalized.is_empty()
        || visualner::is_heading_surface(&normalized)
        || semantic_core::lexicon::lexicon()
            .stopwords
            .contains_filler_phrase(&normalized)
}

pub(crate) fn retain_grounded_entities(
    source: &str,
    entities: Vec<VisualEntity>,
) -> (Vec<VisualEntity>, usize) {
    let total = entities.len();
    // One grounding gate for every projection: the shared `retain_grounded`
    // owns the byte-span check, and entity surfaces additionally require their
    // display text to be the verbatim evidence slice.
    let (kept, _) = retain_grounded(
        source,
        entities,
        |entity| entity.start,
        |entity| entity.end,
        |entity| entity.evidence.as_str(),
    );
    let kept: Vec<VisualEntity> = kept
        .into_iter()
        .filter(|entity| entity.text == entity.evidence)
        .collect();
    let dropped = total.saturating_sub(kept.len());
    (kept, dropped)
}

pub(crate) fn retain_grounded_temporal(
    source: &str,
    facts: Vec<TemporalFact>,
) -> Vec<TemporalFact> {
    retain_grounded(
        source,
        facts,
        |fact| fact.start_byte,
        |fact| fact.end_byte,
        |fact| fact.evidence.as_str(),
    )
    .0
}

pub(crate) fn retain_grounded_candidates(
    source: &str,
    candidates: Vec<VisualCandidate>,
) -> (Vec<VisualCandidate>, usize) {
    retain_grounded(
        source,
        candidates,
        |candidate| candidate.start,
        |candidate| candidate.end,
        |candidate| candidate.evidence.as_str(),
    )
}
