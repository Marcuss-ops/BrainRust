//! Typed event-frame projection shared by the semantic worker and consumers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EventParticipant {
    pub role: String,
    pub entity_id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EventTime {
    pub fact_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_date: Option<String>,
    pub derived: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EventEvidence {
    pub scene_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
}

/// One source-grounded event mention. Cross-scene reconciliation remains a
/// separate pass, so each initial frame carries one scene and its evidence.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EventFrame {
    pub id: String,
    pub event_type: String,
    pub trigger: String,
    pub trigger_start_byte: usize,
    pub trigger_end_byte: usize,
    pub participants: Vec<EventParticipant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<EventTime>,
    pub status: String,
    pub scene_ids: Vec<String>,
    pub evidence: Vec<EventEvidence>,
    pub confidence_basis: Vec<String>,
    /// Entity facts whose source evidence overlaps this event mention.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_fact_ids: Vec<String>,
}
