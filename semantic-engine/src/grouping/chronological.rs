//! Chronology groups, progression/date timeline proposals and standalone date cards.

use super::super::*;

/// Forms a chronology only when at least two dates are fully normalized and
/// each member can be tied to an exact source sentence. Unresolved dates are
/// retained as facts but cannot silently establish temporal order.
pub(crate) fn chronological_groups(
    scene_id: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    facts: &[TemporalFact],
) -> Vec<SemanticGroup> {
    let mut members = Vec::new();
    for (fact_index, fact) in facts.iter().enumerate() {
        let Some(normalized) = fact.normalized.as_deref() else {
            continue;
        };
        let Some((sentence_index, sentence)) =
            sentences.iter().enumerate().find(|(_, sentence)| {
                fact.start_byte >= sentence.start_byte && fact.end_byte <= sentence.end_byte
            })
        else {
            continue;
        };
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if !is_event_sentence(&sentence_text.to_lowercase()) {
            continue;
        }
        // One sentence is one event, however many dates it carries ("nel 2009
        // ... che nel 2011 viene confermata"): two members with the same
        // evidence would draw the same frame twice. The earliest date of the
        // sentence names the event.
        if members
            .iter()
            .any(|(_, existing): &(String, SemanticGroupMember)| {
                existing.sentence_index == sentence_index
            })
        {
            continue;
        }
        members.push((
            normalized.to_string(),
            SemanticGroupMember {
                candidate_id: format!("date_event_{:02}", fact_index + 1),
                scene_id: scene_id.to_string(),
                rank: None,
                sentence_index,
                text: sentence_text.to_string(),
                evidence: sentence_text.to_string(),
                start_byte: sentence.start_byte,
                end_byte: sentence.end_byte,
                temporal_surface: Some(fact.surface.clone()),
                normalized_date: fact.normalized.clone(),
            },
        ));
    }
    if members.len() < 2 {
        return Vec::new();
    }
    members.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.sentence_index.cmp(&b.1.sentence_index))
    });
    vec![SemanticGroup {
        id: "chronology_01".to_string(),
        group_type: "chronological".to_string(),
        title: "Cronologia".to_string(),
        ranking_basis: None,
        target_count: None,
        complete: None,
        members: members.into_iter().map(|(_, member)| member).collect(),
    }]
}

/// Turns a complete `step_sequence` group into one timeline proposal over the
/// group's own evidence, and drops the sentence-level timelines it contains.
/// Without this, a five-step progression is represented by two-step fragments
/// and the last step of the sequence stays uncovered.
pub(crate) fn sequence_timeline_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    groups: &[SemanticGroup],
    temporal_facts: &[TemporalFact],
    entity_facts: &[EntityFact],
    out: &mut Vec<VisualCandidate>,
) {
    // A complete progression wins over a mere chronology of the same evidence:
    // the step sequence is the structure the planner is asked to animate.
    let sequences = groups
        .iter()
        .filter(|group| group.group_type == "step_sequence" && group.members.len() >= 3);
    let chronologies = groups
        .iter()
        .filter(|group| group.group_type == "chronological" && group.members.len() >= 2);
    let mut emitted: Vec<(usize, usize)> = Vec::new();
    for group in sequences.chain(chronologies) {
        let Some(start) = group.members.iter().map(|member| member.start_byte).min() else {
            continue;
        };
        let Some(end) = group.members.iter().map(|member| member.end_byte).max() else {
            continue;
        };
        // The same progression must not be emitted twice (a chronology whose
        // evidence sits inside a step sequence adds nothing).
        if emitted
            .iter()
            .any(|(known_start, known_end)| start >= *known_start && end <= *known_end)
        {
            continue;
        }
        emitted.push((start, end));
        let Some(evidence) = source.get(start..end) else {
            continue;
        };
        let mut surfaces: Vec<String> = group
            .members
            .iter()
            .filter_map(|member| member.normalized_date.clone())
            .collect();
        surfaces.dedup();
        // The fragments of this same progression are the timelines of its own
        // sentences; the complete sequence replaces them instead of repeating
        // them (one timeline per scene).
        out.retain(|candidate| {
            candidate.kind != "timeline" || candidate.start < start || candidate.end > end
        });
        out.push(VisualCandidate {
            kind: "timeline",
            sentence_index: group.members[0].sentence_index,
            entity_ids: Vec::new(),
            entity_indices: Vec::new(),
            entity_texts: Vec::new(),
            chart_items: Vec::new(),
            temporal_surfaces: surfaces,
            suggested_layout: Some("timeline"),
            requires_verified_images: false,
            evidence: evidence.to_string(),
            start,
            end,
        });
    }
    isolated_event_candidates(
        source,
        sentences,
        temporal_facts,
        entity_facts,
        &emitted,
        out,
    );
}

/// A dated event that stands alone is presented on its own card, not dropped.
/// Without it, a fact stated in one sentence has no proposal at all and the
/// coverage report lists it as uncovered; the planner still decides whether to
/// animate it. A sentence qualifies when it carries an explicit event predicate
/// or when the extractor already grounded a dated fact in it, so a stray date
/// in background prose never becomes a card.
pub(crate) fn isolated_event_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    temporal_facts: &[TemporalFact],
    entity_facts: &[EntityFact],
    covered: &[(usize, usize)],
    out: &mut Vec<VisualCandidate>,
) {
    let mut emitted: Vec<(usize, usize)> = covered.to_vec();
    for fact in temporal_facts {
        if fact.normalized.is_none() {
            continue;
        }
        if emitted
            .iter()
            .any(|(start, end)| *start <= fact.start_byte && fact.end_byte <= *end)
        {
            continue;
        }
        let Some((sentence_index, sentence)) =
            sentences.iter().enumerate().find(|(_, sentence)| {
                sentence.start_byte <= fact.start_byte && fact.end_byte <= sentence.end_byte
            })
        else {
            continue;
        };
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if !is_event_sentence(&text.to_lowercase()) && !asserts_dated_fact(entity_facts, sentence) {
            continue;
        }
        // A timeline this scene already draws over the sentence (two dates in
        // one bullet) presents the event already; a card would repeat it.
        if out.iter().any(|candidate| {
            candidate.kind == "timeline"
                && candidate.start <= sentence.start_byte
                && sentence.end_byte <= candidate.end
        }) {
            continue;
        }
        emitted.push((sentence.start_byte, sentence.end_byte));
        out.push(VisualCandidate {
            kind: "timeline",
            sentence_index,
            entity_ids: Vec::new(),
            entity_indices: Vec::new(),
            entity_texts: Vec::new(),
            chart_items: Vec::new(),
            // A date card shows the surface the script uses, verbatim.
            temporal_surfaces: vec![fact.surface.clone()],
            suggested_layout: Some("date_card"),
            requires_verified_images: false,
            evidence: text.to_string(),
            start: sentence.start_byte,
            end: sentence.end_byte,
        });
    }
}

/// True when a fact with a normalized date was grounded inside `sentence`.
/// That is an asserted dated claim, which the event-predicate list cannot see
/// across domains: "il GUP emise la sua decisione di assoluzione" is an event
/// its stems (built for construction and outcomes) do not name.
pub(crate) fn asserts_dated_fact(
    facts: &[EntityFact],
    sentence: &semantic_core::SentenceSpan,
) -> bool {
    facts.iter().any(|fact| {
        fact.normalized_date.is_some()
            && sentence.start_byte <= fact.start_byte
            && fact.end_byte <= sentence.end_byte
    })
}

/// Require an explicit event/process predicate before dates are grouped into
/// a visual timeline. Merely placing unrelated dates in a scene is not enough.
pub(crate) fn is_event_sentence(sentence_lower: &str) -> bool {
    lexicon::lexicon()
        .process_phases
        .event_predicate_markers
        .iter()
        .any(|marker| sentence_lower.contains(marker.as_str()))
}
