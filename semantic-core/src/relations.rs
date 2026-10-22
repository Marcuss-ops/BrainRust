//! Relations: two distinct kinds that must never share one meaning.
//!
//! `co_mentions` are pairs of identities that occur in the same sentence — a
//! *possible* grouping signal, never a claim. `asserted_relations` are
//! explicitly stated links (collaboration, ownership, contrast) carried with
//! evidence and a negation/uncertainty status. A co-mention must never become
//! an invented relation: the planner may propose `side_by_side` for it, but a
//! directed edge requires an assertion.

use serde::Serialize;

/// Two identities occurring in the same sentence. Emitted for grouping, not
/// as a claim: no direction, no predicate, no invented semantics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoMention {
    pub left_id: String,
    pub right_id: String,
    pub sentence_index: usize,
    /// Verbatim sentence evidence covering both mentions.
    pub evidence: String,
}

/// An explicitly asserted relation with its grounding and epistemic status.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssertedRelation {
    pub source_id: String,
    pub target_id: String,
    /// Normalized predicate family (`collaboration`, `ownership`, `contrast`,
    /// `causal`, `temporal`, ...) when recognized; `None` for bare assertions.
    pub predicate: Option<String>,
    /// `asserted`, `negated` or `uncertain`.
    pub status: RelationStatus,
    pub sentence_index: usize,
    /// Verbatim source evidence of the assertion.
    pub evidence: String,
}

/// Epistemic status of an asserted relation. "X did not work with Y" is a
/// relation with status `negated`; it must not fire as a positive edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RelationStatus {
    Asserted,
    Negated,
    Uncertain,
}

/// An explicit lexical relation marker found inside one source sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitRelationMarker {
    pub predicate: String,
    pub status: RelationStatus,
    pub start_byte: usize,
}

/// Detect a small, multilingual set of explicit relation predicates. This
/// function never upgrades entity co-occurrence to a relation: callers must
/// provide two grounded identities in the same sentence before using it.
/// The marker vocabulary lives in the shared lexicon (`relation_cues.json`).
pub fn explicit_relation_marker(sentence: &str, language: &str) -> Option<ExplicitRelationMarker> {
    let folded = sentence.to_lowercase();
    let mut found = None;
    for group in &crate::lexicon::lexicon().relation.relation_markers {
        for marker in &group.markers {
            if let Some(start_byte) = folded.find(marker.as_str()) {
                if found
                    .as_ref()
                    .is_none_or(|current: &ExplicitRelationMarker| start_byte < current.start_byte)
                {
                    found = Some(ExplicitRelationMarker {
                        predicate: group.predicate.clone(),
                        status: relation_status(&folded, start_byte, language),
                        start_byte,
                    });
                }
            }
        }
    }
    found
}

/// Byte offset where the clause containing `marker_start` begins: right
/// after the last clause separator before it. Shared by every status reader
/// so a negation in an earlier clause can never flip a later assertion.
pub fn clause_start(prefix: &str) -> usize {
    crate::lexicon::lexicon()
        .relation
        .clause_separators
        .iter()
        .filter_map(|separator| {
            prefix
                .rfind(separator.as_str())
                .map(|offset| offset + separator.len())
        })
        .max()
        .unwrap_or(0)
}

/// Status of one clause: negation is read from the named negation
/// vocabularies, uncertainty from the shared cue table. Negation is
/// preserved, never flipped into the opposite claim.
pub fn clause_status(clause: &str, negation_languages: &[&str]) -> RelationStatus {
    let lexicon = crate::lexicon::lexicon();
    if clause.split_whitespace().any(|word| {
        negation_languages
            .iter()
            .any(|language| is_negation_cue(word, language))
    }) {
        RelationStatus::Negated
    } else if lexicon
        .relation
        .uncertainty_cues
        .iter()
        .any(|cue| clause.contains(cue.as_str()))
    {
        RelationStatus::Uncertain
    } else {
        RelationStatus::Asserted
    }
}

fn relation_status(sentence: &str, marker_start: usize, language: &str) -> RelationStatus {
    let clause = &sentence[clause_start(&sentence[..marker_start])..];
    clause_status(clause, &[language])
}

/// A scene's relation view, kept in two separate collections on purpose.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct RelationView {
    pub co_mentions: Vec<CoMention>,
    pub asserted_relations: Vec<AssertedRelation>,
}

/// Negation cue words per supported language, used to mark an assertion as
/// `negated` instead of dropping it (the evidence still exists in the source).
/// The vocabulary lives in the shared lexicon (`relation_cues.json`).
pub fn is_negation_cue(word: &str, language: &str) -> bool {
    let word = word.trim().to_lowercase();
    crate::lexicon::lexicon()
        .relation
        .negation_cues_for(language)
        .iter()
        .any(|cue| cue == &word)
}
