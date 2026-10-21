//! Grounding gate: nothing leaves the engine unless its evidence is the exact
//! UTF-8 slice of the source text at the reported byte offsets.

use visualner::{TemporalFact, VisualEntity};

use crate::groups::VisualCandidate;

/// True when `source[start..end]` is exactly `evidence` and it is not empty.
pub fn span_matches(source: &str, start: usize, end: usize, evidence: &str) -> bool {
    !evidence.is_empty() && source.get(start..end) == Some(evidence)
}

pub fn entity_is_grounded(source: &str, entity: &VisualEntity) -> bool {
    entity.text == entity.evidence && span_matches(source, entity.start, entity.end, &entity.evidence)
}

/// Returns the grounded entities and how many were dropped. Entity indices in
/// proposals refer to the returned list, so this runs before grouping.
pub fn retain_grounded_entities(source: &str, entities: Vec<VisualEntity>) -> (Vec<VisualEntity>, usize) {
    let total = entities.len();
    let kept: Vec<VisualEntity> = entities
        .into_iter()
        .filter(|entity| entity_is_grounded(source, entity))
        .collect();
    let dropped = total - kept.len();
    (kept, dropped)
}

pub fn retain_grounded_temporal(source: &str, facts: Vec<TemporalFact>) -> Vec<TemporalFact> {
    facts
        .into_iter()
        .filter(|fact| span_matches(source, fact.start_byte, fact.end_byte, &fact.evidence))
        .collect()
}

/// Returns the grounded proposals and how many were dropped.
pub fn retain_grounded_candidates(source: &str, candidates: Vec<VisualCandidate>) -> (Vec<VisualCandidate>, usize) {
    let total = candidates.len();
    let kept: Vec<VisualCandidate> = candidates
        .into_iter()
        .filter(|candidate| span_matches(source, candidate.start, candidate.end, &candidate.evidence))
        .collect();
    let dropped = total - kept.len();
    (kept, dropped)
}
