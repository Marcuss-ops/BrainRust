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
pub fn build(
    language: &str,
    scene_id: &str,
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
