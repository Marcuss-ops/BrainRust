//! Conservative, clause-local event frames grounded in typed triggers and
//! already-grounded participants, dates, and place mentions.

use semantic_core::{Candidate, Place, TemporalFact};

pub const MAX_EVENT_FRAMES: usize = 200;

pub(crate) struct SceneEventInput<'a> {
    pub scene_index: usize,
    pub scene_id: &'a str,
    pub source: &'a str,
    pub language: &'a str,
    pub candidates: &'a [Candidate],
    pub temporal_facts: &'a [TemporalFact],
    pub places: &'a [Place],
}

mod extract;

mod lexicon;

mod merge;

mod proximity;

pub(crate) use extract::extract_bounded_with_tokens;
pub(crate) use merge::merge_cross_scene;

#[cfg(test)]
use extract::{extract, extract_bounded, trigger_spans};
#[cfg(test)]
use lexicon::{lexicon, TriggerEntry};
#[cfg(test)]
use proximity::nearest_candidate;
#[cfg(test)]
use semantic_core::{EventEvidence, EventFrame, EventParticipant, EventTime};

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
