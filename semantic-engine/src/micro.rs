//! Fact-to-microsummary projection.
//!
//! Micro phrases are not a second extractor: they are a deterministic
//! projection of the grounded atomic facts. Every phrase keeps the fact ids
//! that justify it, and a fact that is negated or uncertain never becomes a
//! summary. This is the rule that stops "non condannato" from being rewritten
//! as "innocente" and an investigation from becoming a conviction.

use std::collections::BTreeMap;

use semantic_core::lexicon::{LegalLexicon, MicroLexicon};
use semantic_core::{legal_packs, EntityFact, MicroSummary, SemanticGroup, SemanticGroupMember};
use visualner::VisualEntity;

/// Summary + groups produced from one scene's facts.
#[derive(Debug, Default)]
pub struct MicroBuild {
    pub summaries: Vec<MicroSummary>,
    pub groups: Vec<SemanticGroup>,
}

/// Builds the fact-backed micro phrases and their comparison/sequence groups.
/// `source` is the scene text: a merged summary keeps a verbatim evidence
/// slice across the nearby sentences it covers.
pub fn build(
    language: &str,
    scene_id: &str,
    source: &str,
    facts: &[EntityFact],
    entities: &[VisualEntity],
    entity_ids: &[String],
) -> MicroBuild {
    // Micro phrases are rendered from a language's domain pack. Without one
    // the engine emits no summaries instead of Italian text under a foreign
    // language tag; the facts themselves are still in the manifest.
    let Some(legal) = legal_packs::get(language) else {
        return MicroBuild::default();
    };
    if legal.micro.templates.is_empty() {
        return MicroBuild::default();
    }
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for (index, entity) in entities.iter().enumerate() {
        if let Some(id) = entity_ids.get(index) {
            names.insert(id.as_str(), entity.text.as_str());
        }
    }

    let mut summaries: Vec<MicroSummary> = Vec::new();
    for fact in facts.iter().filter(|fact| fact.status == "asserted") {
        let Some(text) = phrase_for(fact, &names, legal) else {
            continue;
        };
        if text.chars().count() > 120 {
            continue;
        }
        if let Some(existing) = summaries
            .iter_mut()
            .find(|summary| summary.text.eq_ignore_ascii_case(&text))
        {
            // The same claim stated again in a nearby sentence (same or
            // adjacent) merges into one summary: the subject, the date and
            // the asserted-only negation semantics are identical by
            // construction, and the merged summary keeps every evidence id
            // and one verbatim slice over every evidence sentence. A repeat
            // that is not nearby keeps its first deterministic evidence
            // unchanged.
            if existing.subject_entity_id.as_deref() == Some(fact.subject_entity_id.as_str())
                && existing.normalized_date == fact.normalized_date
                && existing.sentence_index.abs_diff(fact.sentence_index) <= 1
            {
                existing.evidence_fact_ids.push(fact.id.clone());
                existing.start_byte = existing.start_byte.min(fact.evidence_start_byte);
                existing.end_byte = existing.end_byte.max(fact.evidence_end_byte);
                if fact.sentence_index < existing.sentence_index {
                    existing.sentence_index = fact.sentence_index;
                }
                if let Some(merged) = source.get(existing.start_byte..existing.end_byte) {
                    existing.evidence = merged.to_string();
                }
            }
            continue;
        }
        summaries.push(MicroSummary {
            id: format!("micro_{:03}", summaries.len() + 1),
            kind: "micro_summary".to_string(),
            text,
            evidence_fact_ids: vec![fact.id.clone()],
            subject_entity_id: Some(fact.subject_entity_id.clone()),
            normalized_date: fact.normalized_date.clone(),
            sentence_index: fact.sentence_index,
            evidence: fact.evidence.clone(),
            start_byte: fact.evidence_start_byte,
            end_byte: fact.evidence_end_byte,
            score: 0.8,
        });
    }
    summaries.sort_by_key(|summary| (summary.sentence_index, summary.start_byte));

    let mut groups = Vec::new();
    groups.extend(comparison_groups(scene_id, &summaries, legal));
    groups.extend(sequence_groups(scene_id, &summaries, legal));

    MicroBuild { summaries, groups }
}

/// A short phrase for one asserted fact. Wording changes; subject, state,
/// date and attribution do not.
fn phrase_for(
    fact: &EntityFact,
    names: &BTreeMap<&str, &str>,
    legal: &LegalLexicon,
) -> Option<String> {
    let name = names.get(fact.subject_entity_id.as_str())?;
    let normalized = fact.normalized_value.as_deref()?;
    let template = legal.micro_template(&fact.predicate, normalized)?;
    let target = fact
        .target_entity_id
        .as_deref()
        .and_then(|id| names.get(id).copied());
    if template.contains("{target}") && target.is_none() {
        return None;
    }
    let date = date_suffix(fact, &legal.micro);
    let mut phrase = template.replace("{name}", name);
    if let Some(target) = target {
        phrase = phrase.replace("{target}", target);
    }
    phrase = phrase.replace("{normalized}", normalized);
    phrase = phrase.replace("{date}", &date);
    Some(phrase)
}

/// ` nel 2009` for a year, ` il 17 dicembre 2009` for a full date.
fn date_suffix(fact: &EntityFact, micro: &MicroLexicon) -> String {
    if fact.value_type != "DATE" {
        return String::new();
    }
    // The normalized date is the only form that is always numeric: a source
    // may spell the year ("duemiladiciannove"), and pasting that surface into
    // a micro phrase produces "condanna il duemiladiciannove".
    if let Some(year) = fact
        .normalized_date
        .as_deref()
        .and_then(|date| date.get(0..4))
    {
        return micro.date_year.replace("{year}", year);
    }
    // Use the verbatim surface, not the ISO form: "17 dicembre 2009" reads
    // correctly, "2009-12-17" does not.
    let surface = fact.value.trim();
    if surface.len() == 4 && surface.chars().all(|ch| ch.is_ascii_digit()) {
        micro.year_surface.replace("{surface}", surface)
    } else if surface.is_empty() {
        String::new()
    } else {
        micro.date_surface.replace("{surface}", surface)
    }
}

fn member(scene_id: &str, summary: &MicroSummary) -> SemanticGroupMember {
    SemanticGroupMember {
        candidate_id: summary.id.clone(),
        scene_id: scene_id.to_string(),
        rank: None,
        sentence_index: summary.sentence_index,
        text: summary.text.clone(),
        evidence: summary.evidence.clone(),
        start_byte: summary.start_byte,
        end_byte: summary.end_byte,
        temporal_surface: None,
        // A step in a dated sequence keeps its ISO date, so a timeline
        // overlay can order it without re-reading the phrase.
        normalized_date: summary.normalized_date.clone(),
    }
}

/// Two different subjects with distinct procedural states in the same scene:
/// a fact comparison, never a ranking of guilt.
fn comparison_groups(
    scene_id: &str,
    summaries: &[MicroSummary],
    legal: &LegalLexicon,
) -> Vec<SemanticGroup> {
    let comparison = &legal.comparison;
    let conviction = summaries.iter().find(|summary| {
        summary.kind == "micro_summary"
            && contains_any(&summary.text, &comparison.conviction_cues)
            && !contains_any(&summary.text, &comparison.conviction_exclusions)
    });
    let investigation = summaries.iter().find(|summary| {
        summary.kind == "micro_summary"
            && contains_any(&summary.text, &comparison.investigation_cues)
    });
    let (Some(conviction), Some(investigation)) = (conviction, investigation) else {
        return Vec::new();
    };
    if conviction.text == investigation.text
        || conviction.subject_entity_id == investigation.subject_entity_id
    {
        return Vec::new();
    }
    vec![SemanticGroup {
        id: format!("fact_comparison_{scene_id}"),
        group_type: "fact_comparison".to_string(),
        title: comparison
            .title
            .replace("{left}", subject_of(&conviction.text))
            .replace("{right}", subject_of(&investigation.text)),
        ranking_basis: Some("explicit_source_facts".to_string()),
        target_count: Some(2),
        complete: Some(true),
        members: vec![
            member(scene_id, conviction),
            member(scene_id, investigation),
        ],
    }]
}

fn subject_of(text: &str) -> &str {
    text.split(':').next().unwrap_or(text)
}

/// Legal outcomes with a date, ordered chronologically: the five-step
/// sequence a planner can animate as one structure.
fn sequence_groups(
    scene_id: &str,
    summaries: &[MicroSummary],
    legal: &LegalLexicon,
) -> Vec<SemanticGroup> {
    let mut steps: Vec<(String, &MicroSummary)> = summaries
        .iter()
        .filter(|summary| summary.kind == "micro_summary")
        .filter_map(|summary| {
            let date = date_key(summary)?;
            if !is_outcome(&summary.text, legal) {
                return None;
            }
            Some((date, summary))
        })
        .collect();
    steps.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.start_byte.cmp(&b.1.start_byte)));
    steps.dedup_by(|a, b| a.0 == b.0);
    if steps.len() < 3 {
        return Vec::new();
    }
    vec![SemanticGroup {
        id: format!("step_sequence_{scene_id}"),
        group_type: "step_sequence".to_string(),
        title: legal
            .comparison
            .sequence_title
            .replace("{count}", &steps.len().to_string()),
        ranking_basis: Some("explicit_source_dates".to_string()),
        target_count: Some(steps.len()),
        complete: Some(true),
        members: steps
            .iter()
            .map(|(_, summary)| member(scene_id, summary))
            .collect(),
    }]
}

/// The ISO date is embedded in the evidence sentence; read it from the fact
/// evidence through the date suffix already encoded in the phrase is fragile,
/// so the summary keeps the year only as a sort key.
fn date_key(summary: &MicroSummary) -> Option<String> {
    summary
        .normalized_date
        .clone()
        .filter(|date| date.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
}

fn is_outcome(text: &str, legal: &LegalLexicon) -> bool {
    contains_any(text, &legal.outcome_markers)
}

fn contains_any(text: &str, cues: &[String]) -> bool {
    cues.iter().any(|cue| text.contains(cue.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "Alberto Stasi";

    fn person() -> (VisualEntity, String) {
        (
            VisualEntity {
                text: NAME.to_string(),
                r#type: "PERSON".to_string(),
                score: 0.9,
                start: 0,
                end: NAME.len(),
                evidence: NAME.to_string(),
            },
            "e_stasi".to_string(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn fact(
        id: &str,
        subject: &str,
        sentence_index: usize,
        predicate: &str,
        normalized_value: &str,
        normalized_date: Option<&str>,
        status: &str,
        evidence: &str,
        span: (usize, usize),
    ) -> EntityFact {
        EntityFact {
            id: id.to_string(),
            subject_entity_id: subject.to_string(),
            predicate: predicate.to_string(),
            value: evidence.to_string(),
            value_type: if normalized_date.is_some() {
                "DATE".to_string()
            } else {
                "TEXT".to_string()
            },
            normalized_value: Some(normalized_value.to_string()),
            normalized_date: normalized_date.map(str::to_string),
            target_entity_id: None,
            unit: None,
            operator: None,
            status: status.to_string(),
            sentence_index,
            evidence: evidence.to_string(),
            start_byte: span.0,
            end_byte: span.1,
            evidence_start_byte: span.0,
            evidence_end_byte: span.1,
            importance: 0.9,
            visual_concept_ids: Vec::new(),
        }
    }

    fn spans_of(sentences: &[&str]) -> (String, Vec<(usize, usize)>) {
        let mut source = String::new();
        let mut spans = Vec::new();
        for sentence in sentences {
            if !source.is_empty() {
                source.push(' ');
            }
            let start = source.len();
            source.push_str(sentence);
            spans.push((start, source.len()));
        }
        (source, spans)
    }

    #[test]
    fn negated_facts_never_become_a_summary() {
        let (entity, id) = person();
        let (source, spans) = spans_of(&["Alberto Stasi fu assolto."]);
        let facts = vec![fact(
            "f1",
            &id,
            0,
            "legal_outcome",
            "convicted",
            Some("2009-12-17"),
            "negated",
            "Alberto Stasi fu assolto.",
            spans[0],
        )];
        let built = build("it", "scena", &source, &facts, &[entity], &[id]);
        assert!(
            built.summaries.is_empty(),
            "a negated fact is never a summary: {:?}",
            built.summaries
        );
    }

    #[test]
    fn not_convicted_is_never_rewritten_as_innocent() {
        let (entity, id) = person();
        let (source, spans) = spans_of(&["Alberto Stasi non fu condannato."]);
        let facts = vec![fact(
            "f1",
            &id,
            0,
            "legal_outcome",
            "not_convicted",
            None,
            "asserted",
            "Alberto Stasi non fu condannato.",
            spans[0],
        )];
        let built = build("it", "scena", &source, &facts, &[entity], &[id]);
        let summary = built
            .summaries
            .first()
            .unwrap_or_else(|| panic!("summary expected: {:?}", built.summaries));
        assert_eq!(summary.text, "Alberto Stasi: nessuna condanna nel procedimento descritto");
        assert!(
            !summary.text.contains("assolto") && !summary.text.contains("innocente"),
            "negation must never become innocence: {}",
            summary.text
        );
    }

    #[test]
    fn outcome_keeps_the_year_and_the_fact_id() {
        let (entity, id) = person();
        let (source, spans) = spans_of(&["Il 17 dicembre 2009 arrivò la condanna."]);
        let facts = vec![fact(
            "f_condanna_2009",
            &id,
            0,
            "legal_outcome",
            "convicted",
            Some("2009-12-17"),
            "asserted",
            "Il 17 dicembre 2009 arrivò la condanna.",
            spans[0],
        )];
        let built = build("it", "scena", &source, &facts, &[entity], &[id]);
        let summary = &built.summaries[0];
        assert_eq!(summary.text, "Alberto Stasi: condanna nel 2009");
        assert_eq!(summary.evidence_fact_ids, vec!["f_condanna_2009"]);
        assert_eq!(summary.normalized_date.as_deref(), Some("2009-12-17"));
        assert_eq!(summary.subject_entity_id.as_deref(), Some("e_stasi"));
    }

    #[test]
    fn three_dated_outcomes_form_one_ordered_sequence() {
        let (entity, id) = person();
        let (source, spans) = spans_of(&[
            "Nel 2014 arrivò la condanna.",
            "Nel 2011 arrivò l'assoluzione.",
            "Nel 2015 la condanna divenne definitiva.",
        ]);
        let facts = vec![
            fact("f1", &id, 0, "legal_outcome", "convicted", Some("2014-12-17"), "asserted", "Nel 2014 arrivò la condanna.", spans[0]),
            fact("f2", &id, 1, "legal_outcome", "acquitted", Some("2011-10-27"), "asserted", "Nel 2011 arrivò l'assoluzione.", spans[1]),
            fact("f3", &id, 2, "legal_finality", "final", Some("2015-06-11"), "asserted", "Nel 2015 la condanna divenne definitiva.", spans[2]),
        ];
        let built = build("it", "scena", &source, &facts, &[entity], &[id]);
        assert_eq!(built.summaries.len(), 3);
        let sequence = built
            .groups
            .iter()
            .find(|group| group.group_type == "step_sequence")
            .unwrap_or_else(|| panic!("sequence expected: {:?}", built.groups));
        assert_eq!(sequence.members.len(), 3);
        let dates: Vec<Option<&str>> = sequence
            .members
            .iter()
            .map(|member| member.normalized_date.as_deref())
            .collect();
        assert_eq!(
            dates,
            vec![Some("2011-10-27"), Some("2014-12-17"), Some("2015-06-11")],
            "the sequence is chronological"
        );
    }

    #[test]
    fn nearby_repeated_claims_merge_with_every_evidence_id() {
        let (entity, id) = person();
        let (source, spans) = spans_of(&[
            "Alberto Stasi fu assolto.",
            "Alberto Stasi fu assolto.",
            "Il caso continua.",
            "Alberto Stasi fu assolto.",
        ]);
        let outcome = "acquitted";
        let facts = vec![
            fact("f1", &id, 0, "legal_outcome", outcome, Some("2011-10-27"), "asserted", "Alberto Stasi fu assolto.", spans[0]),
            fact("f2", &id, 1, "legal_outcome", outcome, Some("2011-10-27"), "asserted", "Alberto Stasi fu assolto.", spans[1]),
            // The same claim far apart in the scene is not a nearby repeat:
            // it keeps the first deterministic evidence instead of merging.
            fact("f3", &id, 3, "legal_outcome", outcome, Some("2011-10-27"), "asserted", "Alberto Stasi fu assolto.", spans[3]),
        ];
        let built = build("it", "scena", &source, &facts, &[entity], &[id]);
        assert_eq!(built.summaries.len(), 1, "one claim, one summary");
        let summary = &built.summaries[0];
        assert_eq!(summary.evidence_fact_ids, vec!["f1", "f2"]);
        assert_eq!(summary.sentence_index, 0);
        // The merged evidence is one verbatim slice over both sentences.
        assert_eq!(summary.start_byte, spans[0].0);
        assert_eq!(summary.end_byte, spans[1].1);
        assert_eq!(
            summary.evidence,
            source[spans[0].0..spans[1].1],
            "merged evidence is a verbatim source slice"
        );
    }

    #[test]
    fn merging_never_crosses_a_date() {
        let (entity, id) = person();
        let (source, spans) = spans_of(&[
            "Alberto Stasi è indagato.",
            "Alberto Stasi è indagato.",
        ]);
        let facts = vec![
            fact("f1", &id, 0, "investigation_status", "under_investigation", Some("2015"), "asserted", "Alberto Stasi è indagato.", spans[0]),
            fact("f2", &id, 1, "investigation_status", "under_investigation", Some("2016"), "asserted", "Alberto Stasi è indagato.", spans[1]),
        ];
        let built = build("it", "scena", &source, &facts, &[entity], &[id]);
        assert_eq!(built.summaries.len(), 1);
        assert_eq!(
            built.summaries[0].evidence_fact_ids,
            vec!["f1"],
            "different dates must never merge into one summary"
        );
    }

    #[test]
    fn merging_never_crosses_a_subject() {
        let (entity_a, id_a) = person();
        let mut entity_b = entity_a.clone();
        entity_b.start = 100;
        entity_b.end = 100 + NAME.len();
        let id_b = "e_stasi_omonimo".to_string();
        let (source, spans) = spans_of(&[
            "Alberto Stasi è indagato.",
            "Alberto Stasi è indagato.",
        ]);
        let facts = vec![
            fact("f1", &id_a, 0, "investigation_status", "under_investigation", Some("2015"), "asserted", "Alberto Stasi è indagato.", spans[0]),
            fact("f2", &id_b, 1, "investigation_status", "under_investigation", Some("2015"), "asserted", "Alberto Stasi è indagato.", spans[1]),
        ];
        let built = build(
            "it",
            "scena",
            &source,
            &facts,
            &[entity_a, entity_b],
            &[id_a, id_b],
        );
        assert_eq!(built.summaries.len(), 1);
        assert_eq!(
            built.summaries[0].evidence_fact_ids,
            vec!["f1"],
            "different subjects must never merge into one summary"
        );
    }
}
