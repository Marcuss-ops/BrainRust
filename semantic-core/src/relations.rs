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
pub fn explicit_relation_marker(sentence: &str, language: &str) -> Option<ExplicitRelationMarker> {
    let folded = sentence.to_lowercase();
    let markers: &[(&str, &[&str])] = &[
        (
            "ownership",
            &["owned by", "owns", "possiede", "proprietà di"],
        ),
        ("funding", &["funds", "finanzia", "financed", "finanziert"]),
        (
            "collaboration",
            &[
                "collaborates with",
                "collaborated with",
                "collabora con",
                "lavora con",
                "partners with",
                "partnered with",
            ],
        ),
        (
            "competition",
            &[
                "competes with",
                "competed with",
                "compete con",
                "rivalizza con",
            ],
        ),
        (
            "contrast",
            &[
                "whereas",
                "while",
                "mentre",
                "invece",
                "mientras",
                "tandis que",
            ],
        ),
        (
            "causal",
            &[
                "causes",
                "caused",
                "causa",
                "provoca",
                "leads to",
                "contributes to",
                "contribuisce a",
            ],
        ),
        ("temporal", &["preceded", "precede", "prima di", "before"]),
    ];
    let mut found = None;
    for (predicate, variants) in markers {
        for marker in *variants {
            if let Some(start_byte) = folded.find(marker) {
                if found
                    .as_ref()
                    .is_none_or(|current: &ExplicitRelationMarker| start_byte < current.start_byte)
                {
                    found = Some(ExplicitRelationMarker {
                        predicate: (*predicate).to_string(),
                        status: relation_status(&folded, start_byte, language),
                        start_byte,
                    });
                }
            }
        }
    }
    found
}

fn relation_status(sentence: &str, marker_start: usize, language: &str) -> RelationStatus {
    let prefix = &sentence[..marker_start];
    let clause_start = [" e ", " and ", " y ", " et ", " und ", ";"]
        .iter()
        .filter_map(|separator| {
            prefix
                .rfind(separator)
                .map(|offset| offset + separator.len())
        })
        .max()
        .unwrap_or(0);
    let clause = &sentence[clause_start..];
    if clause
        .split_whitespace()
        .any(|word| is_negation_cue(word, language))
    {
        RelationStatus::Negated
    } else if [
        "forse",
        "perhaps",
        "allegedly",
        "presumibilmente",
        "maybe",
        "possibly",
    ]
    .iter()
    .any(|cue| clause.contains(cue))
    {
        RelationStatus::Uncertain
    } else {
        RelationStatus::Asserted
    }
}

/// A scene's relation view, kept in two separate collections on purpose.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct RelationView {
    pub co_mentions: Vec<CoMention>,
    pub asserted_relations: Vec<AssertedRelation>,
}

/// Negation cue words per supported language, used to mark an assertion as
/// `negated` instead of dropping it (the evidence still exists in the source).
pub fn is_negation_cue(word: &str, language: &str) -> bool {
    let word = word.trim().to_lowercase();
    let lang = language.split(['-', '_']).next().unwrap_or("en");
    match lang {
        "it" => matches!(
            word.as_str(),
            "non" | "mai" | "senza" | "niente" | "nessuno"
        ),
        "en" => matches!(
            word.as_str(),
            "not" | "never" | "no" | "without" | "denies" | "denied"
        ),
        "es" => matches!(word.as_str(), "no" | "nunca" | "sin" | "nada" | "niega"),
        "fr" => matches!(word.as_str(), "ne" | "pas" | "jamais" | "sans" | "nie"),
        "de" => matches!(
            word.as_str(),
            "nicht" | "nie" | "ohne" | "kein" | "bestreitet"
        ),
        _ => matches!(word.as_str(), "not" | "never" | "no" | "without"),
    }
}
