//! Typed planner decisions (ROADMAP 2026-11-19 §4).
//!
//! A candidate that does not reach the sealed plan is recorded here with a
//! stable reason code, so an operator can answer "why is there no overlay at
//! 00:42?" without re-running the planner. The log is additive: an empty log is
//! omitted from the wire.

use super::super::request::EntityOccurrenceV2;
use super::super::response::{PlanDecisionV2, PlanItemV2};

pub(super) const STAGE_CANDIDATES: &str = "candidates";
pub(super) const STAGE_ADMISSION: &str = "admission";
pub(super) const STAGE_OVERLAP: &str = "overlap";
pub(super) const STAGE_CLIP: &str = "clip";
pub(super) const OUTCOME_DROPPED: &str = "dropped";

// Candidate lane: the entity card was never built.
pub(super) const REASON_ENTITY_KIND_UNSUPPORTED: &str = "ENTITY_KIND_UNSUPPORTED";
pub(super) const REASON_MAP_COVERS_ENTITY: &str = "MAP_COVERS_ENTITY";
pub(super) const REASON_DUPLICATE_IDENTITY: &str = "DUPLICATE_IDENTITY";
pub(super) const REASON_TEXT_CARD_LIMIT: &str = "TEXT_CARD_LIMIT";
pub(super) const REASON_NO_TIMING_WINDOW: &str = "NO_TIMING_WINDOW";

// Admission lane: the candidate lost a finite resource.
pub(super) const REASON_MAPS_ONLY: &str = "MAPS_ONLY";
pub(super) const REASON_CATEGORY_DISABLED: &str = "CATEGORY_DISABLED";
pub(super) const REASON_GROUP_LIMIT: &str = "GROUP_LIMIT";
pub(super) const REASON_BUDGET_EXCEEDED: &str = "BUDGET_EXCEEDED";
pub(super) const REASON_PER_SCENE_LIMIT: &str = "PER_SCENE_LIMIT";

// Layout lanes: the item existed but could not be placed.
pub(super) const REASON_OVERLAP_LOST: &str = "OVERLAP_LOST";
pub(super) const REASON_CLIPPED_OUT: &str = "CLIPPED_OUT";
/// A phrase card sat entirely inside an entity-card window and there was no
/// free position for the card that owned the window, so the phrase was
/// removed instead of rendering an invisible card.
pub(super) const REASON_OCCLUDED_BY_ENTITY_CARD: &str = "OCCLUDED_BY_ENTITY_CARD";
/// A phrase duplicates a named entity or organization card in the same window.
pub(super) const REASON_DUPLICATE_NAMED_PHRASE: &str = "DUPLICATE_NAMED_PHRASE";
/// The candidate carried no certified word-timing anchor, so the gap pass may
/// not promote it into an uncovered interval: it would place copy at a guessed
/// instant. The phrase lane reports this as `unanchored_in_transcript`.
pub(super) const REASON_UNANCHORED_IN_TRANSCRIPT: &str = "UNANCHORED_IN_TRANSCRIPT";

/// Records one dropped plan item. The identity fields are copied so the
/// decision can be joined to the entity timeline without the item.
pub(super) fn push_decision(
    decisions: &mut Vec<PlanDecisionV2>,
    stage: &str,
    reason_code: &str,
    item: &PlanItemV2,
) {
    decisions.push(PlanDecisionV2 {
        candidate_id: item.id.clone(),
        kind: item.kind.clone(),
        start_us: item.start_us,
        scene_id: item.scene_id.clone(),
        entity_id: item.entity_id.clone(),
        canonical_entity_id: item
            .entity_ref
            .as_ref()
            .map(|reference| reference.canonical_entity_id.clone())
            .unwrap_or_default(),
        stage: stage.to_string(),
        outcome: OUTCOME_DROPPED.to_string(),
        reason_code: reason_code.to_string(),
    });
}

/// Records a candidate that never became a plan item: the entity card lane
/// skips an occurrence before `scene_item` exists.
pub(super) fn push_entity_decision(
    decisions: &mut Vec<PlanDecisionV2>,
    reason_code: &str,
    candidate_id: &str,
    scene_id: &str,
    entity: &EntityOccurrenceV2,
) {
    decisions.push(PlanDecisionV2 {
        candidate_id: candidate_id.to_string(),
        kind: String::new(),
        start_us: 0,
        scene_id: scene_id.to_string(),
        entity_id: entity.entity_id.trim().to_string(),
        canonical_entity_id: entity.canonical_entity_id.trim().to_string(),
        stage: STAGE_CANDIDATES.to_string(),
        outcome: OUTCOME_DROPPED.to_string(),
        reason_code: reason_code.to_string(),
    });
}

/// Maps the admission accounting spelling to the public typed reason.
pub(super) fn admission_reason_code(reason: &str) -> &'static str {
    match reason {
        "maps_only" => REASON_MAPS_ONLY,
        "category" => REASON_CATEGORY_DISABLED,
        "group" => REASON_GROUP_LIMIT,
        "budget" => REASON_BUDGET_EXCEEDED,
        "per_scene" => REASON_PER_SCENE_LIMIT,
        _ => REASON_BUDGET_EXCEEDED,
    }
}
