//! Fact-to-microsummary projection.
//!
//! Micro phrases are not a second extractor: they are a deterministic
//! projection of the grounded atomic facts. Every phrase keeps the fact ids
//! that justify it, and a fact that is negated or uncertain never becomes a
//! summary. This is the rule that stops "non condannato" from being rewritten
//! as "innocente" and an investigation from becoming a conviction.

use std::collections::BTreeMap;

use semantic_core::{EntityFact, MicroSummary, SemanticGroup, SemanticGroupMember};
use visualner::VisualEntity;

/// Summary + groups produced from one scene's facts.
#[derive(Debug, Default)]
pub struct MicroBuild {
    pub summaries: Vec<MicroSummary>,
    pub groups: Vec<SemanticGroup>,
}

/// Builds the fact-backed micro phrases and their comparison/sequence groups.
pub fn build(
    scene_id: &str,
    facts: &[EntityFact],
    entities: &[VisualEntity],
    entity_ids: &[String],
) -> MicroBuild {
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for (index, entity) in entities.iter().enumerate() {
        if let Some(id) = entity_ids.get(index) {
            names.insert(id.as_str(), entity.text.as_str());
        }
    }

    let mut summaries: Vec<MicroSummary> = Vec::new();
    for fact in facts.iter().filter(|fact| fact.status == "asserted") {
        let Some(text) = phrase_for(fact, &names) else {
            continue;
        };
        if text.chars().count() > 120 {
            continue;
        }
        if summaries
            .iter()
            .any(|summary| summary.text.eq_ignore_ascii_case(&text))
        {
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
    groups.extend(comparison_groups(scene_id, &summaries));
    groups.extend(sequence_groups(scene_id, &summaries));
    summaries.extend(entity_fact_summaries(&summaries));

    MicroBuild { summaries, groups }
}

/// A short phrase for one asserted fact. Wording changes; subject, state,
/// date and attribution do not.
fn phrase_for(fact: &EntityFact, names: &BTreeMap<&str, &str>) -> Option<String> {
    let name = names.get(fact.subject_entity_id.as_str())?;
    let normalized = fact.normalized_value.as_deref()?;
    let date = date_suffix(fact);
    let target = fact
        .target_entity_id
        .as_deref()
        .and_then(|id| names.get(id).copied());
    let phrase = match (fact.predicate.as_str(), normalized) {
        ("legal_outcome", "acquitted") => format!("{name}: assoluzione{date}"),
        ("legal_outcome", "convicted") => format!("{name}: condanna{date}"),
        ("legal_outcome", "conviction_confirmed") => {
            format!("{name}: conferma della condanna{date}")
        }
        ("legal_outcome", "acquittal_confirmed") => {
            format!("{name}: conferma dell'assoluzione{date}")
        }
        ("legal_outcome", "annulled_with_remand") => {
            format!("{name}: annullamento con rinvio{date}")
        }
        ("legal_finality", "final") => format!("{name}: condanna definitiva{date}"),
        ("investigation_status", "under_investigation") => format!("{name}: indagato"),
        ("investigation_status", "denies_accusations") => format!("{name}: respinge le accuse"),
        ("legal_outcome", "not_convicted") => {
            format!("{name}: nessuna condanna nel procedimento descritto")
        }
        ("procedural_act", "received_warrant") => {
            format!("{name}: avviso di garanzia{date}")
        }
        ("procedural_act", "preliminary_hearing") => {
            format!("{name}: udienza preliminare{date}")
        }
        ("investigation_status", "investigation_concluded") => {
            format!("{name}: conclusione delle indagini{date}")
        }
        ("sentence_length", "years") => {
            format!("{name}: {normalized} anni di reclusione{date}")
        }
        ("personal_data", "age") => format!("{name}: {normalized} anni"),
        ("relationship", "partner_of") => {
            format!("{name}: fidanzato di {}", target?)
        }
        _ => return None,
    };
    Some(phrase)
}

/// ` nel 2009` for a year, ` il 17 dicembre 2009` for a full date.
fn date_suffix(fact: &EntityFact) -> String {
    if fact.value_type != "DATE" {
        return String::new();
    }
    // Use the verbatim surface, not the ISO form: "17 dicembre 2009" reads
    // correctly, "2009-12-17" does not.
    let surface = fact.value.trim();
    if surface.len() == 4 && surface.chars().all(|ch| ch.is_ascii_digit()) {
        format!(" nel {surface}")
    } else if surface.is_empty() {
        String::new()
    } else {
        format!(" il {surface}")
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
    }
}

/// Two different subjects with distinct procedural states in the same scene:
/// a fact comparison, never a ranking of guilt.
fn comparison_groups(scene_id: &str, summaries: &[MicroSummary]) -> Vec<SemanticGroup> {
    let conviction = summaries.iter().find(|summary| {
        summary.kind == "micro_summary"
            && (summary.text.contains("condanna definitiva")
                || (summary.text.contains("condanna")
                    && !summary.text.contains("nessuna condanna")
                    && !summary.text.contains("conferma")))
    });
    let investigation = summaries.iter().find(|summary| {
        summary.kind == "micro_summary"
            && (summary.text.contains("indagato")
                || summary.text.contains("respinge le accuse")
                || summary.text.contains("nessuna condanna"))
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
        title: format!(
            "Confronto: {}",
            pair_title(&conviction.text, &investigation.text)
        ),
        ranking_basis: Some("explicit_source_facts".to_string()),
        target_count: Some(2),
        complete: Some(true),
        members: vec![
            member(scene_id, conviction),
            member(scene_id, investigation),
        ],
    }]
}

fn pair_title(left: &str, right: &str) -> String {
    let left = left.split(':').next().unwrap_or(left);
    let right = right.split(':').next().unwrap_or(right);
    format!("{left} vs {right}")
}

/// Legal outcomes with a date, ordered chronologically: the five-step
/// sequence a planner can animate as one structure.
fn sequence_groups(scene_id: &str, summaries: &[MicroSummary]) -> Vec<SemanticGroup> {
    let mut steps: Vec<(String, &MicroSummary)> = summaries
        .iter()
        .filter(|summary| summary.kind == "micro_summary")
        .filter_map(|summary| {
            let date = date_key(summary)?;
            if !is_outcome(&summary.text) {
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
        title: format!("Sequenza: {} tappe", steps.len()),
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

fn is_outcome(text: &str) -> bool {
    ["assoluzione", "condanna", "annullamento", "conferma"]
        .iter()
        .any(|cue| text.contains(cue))
}

/// A compact entity card: the same facts, grouped under their subject.
fn entity_fact_summaries(summaries: &[MicroSummary]) -> Vec<MicroSummary> {
    let mut by_subject: BTreeMap<&str, Vec<&MicroSummary>> = BTreeMap::new();
    for summary in summaries {
        if summary.kind != "micro_summary" {
            continue;
        }
        let Some(subject) = summary.subject_entity_id.as_deref() else {
            continue;
        };
        by_subject.entry(subject).or_default().push(summary);
    }
    by_subject
        .into_iter()
        .enumerate()
        .filter_map(|(index, (subject, facts))| {
            if facts.len() < 2 {
                return None;
            }
            let display = facts[0]
                .text
                .split_once(':')
                .map(|(name, _)| name)
                .unwrap_or(subject);
            let text = format!(
                "{display}: {}",
                facts
                    .iter()
                    .filter_map(|summary| summary.text.split_once(':').map(|(_, detail)| detail))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            if text.chars().count() > 180 {
                return None;
            }
            Some(MicroSummary {
                id: format!("entity_facts_{:03}", index + 1),
                kind: "entity_facts".to_string(),
                text,
                evidence_fact_ids: facts
                    .iter()
                    .flat_map(|summary| summary.evidence_fact_ids.clone())
                    .collect(),
                subject_entity_id: Some(subject.to_string()),
                normalized_date: None,
                sentence_index: facts[0].sentence_index,
                evidence: facts[0].evidence.clone(),
                start_byte: facts[0].start_byte,
                end_byte: facts[0].end_byte,
                score: 0.7,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(predicate: &str, normalized: &str, value: &str, status: &str) -> EntityFact {
        EntityFact {
            id: format!("fact_{normalized}"),
            subject_entity_id: "person_01".to_string(),
            predicate: predicate.to_string(),
            value: value.to_string(),
            value_type: "DATE".to_string(),
            normalized_value: Some(normalized.to_string()),
            normalized_date: Some(value.to_string()),
            target_entity_id: None,
            unit: None,
            operator: None,
            status: status.to_string(),
            sentence_index: 0,
            evidence: format!("Evidence for {normalized}."),
            start_byte: 0,
            end_byte: 4,
            evidence_start_byte: 0,
            evidence_end_byte: 24,
            importance: 0.0,
            visual_concept_ids: Vec::new(),
        }
    }

    fn entities() -> (Vec<VisualEntity>, Vec<String>) {
        (
            vec![VisualEntity {
                text: "Alberto Stasi".to_string(),
                r#type: "PERSON".to_string(),
                score: 1.0,
                start: 0,
                end: 13,
                evidence: "Alberto Stasi".to_string(),
            }],
            vec!["person_01".to_string()],
        )
    }

    #[test]
    fn negated_facts_never_become_a_summary() {
        let (entities, ids) = entities();
        let build = super::build(
            "scene",
            &[fact("legal_outcome", "acquitted", "2009", "negated")],
            &entities,
            &ids,
        );
        assert!(build.summaries.is_empty());
    }

    #[test]
    fn outcome_keeps_the_year_and_the_fact_id() {
        let (entities, ids) = entities();
        let build = super::build(
            "scene",
            &[fact("legal_outcome", "convicted", "2014", "asserted")],
            &entities,
            &ids,
        );
        assert_eq!(build.summaries.len(), 1);
        assert_eq!(build.summaries[0].text, "Alberto Stasi: condanna nel 2014");
        assert_eq!(
            build.summaries[0].evidence_fact_ids,
            vec!["fact_convicted".to_string()]
        );
    }

    #[test]
    fn three_dated_outcomes_form_one_ordered_sequence() {
        let (entities, ids) = entities();
        let mut acquired = fact("legal_outcome", "acquitted", "2009", "asserted");
        acquired.id = "a".into();
        let mut convicted = fact("legal_outcome", "convicted", "2014", "asserted");
        convicted.id = "b".into();
        let mut final_fact = fact("legal_finality", "final", "2015", "asserted");
        final_fact.id = "c".into();
        let build = super::build("scene", &[final_fact, acquired, convicted], &entities, &ids);
        let sequence = build
            .groups
            .iter()
            .find(|group| group.group_type == "step_sequence")
            .expect("sequence group");
        assert_eq!(sequence.members.len(), 3);
        assert!(sequence.members[0].text.contains("2009"));
        assert!(sequence.members[2].text.contains("2015"));
    }

    #[test]
    fn not_convicted_is_never_rewritten_as_innocent() {
        let (entities, ids) = entities();
        let mut not_convicted = fact("legal_outcome", "not_convicted", "n/a", "asserted");
        not_convicted.value_type = "EVENT".into();
        let build = super::build("scene", &[not_convicted], &entities, &ids);
        assert_eq!(build.summaries.len(), 1);
        assert!(build.summaries[0].text.contains("nessuna condanna"));
        assert!(!build.summaries[0].text.contains("innocente"));
    }
}
