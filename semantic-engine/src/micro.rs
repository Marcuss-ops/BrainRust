//! Fact-to-microsummary projection.
//!
//! Micro phrases are not a second extractor: they are a deterministic
//! projection of the grounded atomic facts. Every phrase keeps the fact ids
//! that justify it, and a fact that is negated or uncertain never becomes a
//! summary. This is the rule that stops "non condannato" from being rewritten
//! as "innocente" and an investigation from becoming a conviction.

use std::collections::BTreeMap;

use semantic_core::lexicon::{self, AttributePredicates, LegalLexicon, MicroLexicon};
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
    // A legal domain pack is optional: its templates are one source of micro
    // phrases, not a precondition. Attribute and metric projections below are
    // language-neutral and run for every supported language.
    let legal = legal_packs::get(language);
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for (index, entity) in entities.iter().enumerate() {
        if let Some(id) = entity_ids.get(index) {
            names.insert(id.as_str(), entity.text.as_str());
        }
    }

    let ontology = &lexicon::lexicon().attribute_predicates;
    let mut summaries: Vec<MicroSummary> = Vec::new();
    // A compact, fact-backed line per person whose scene states at least two
    // canonical attributes. It is a projection of the same facts the card
    // uses, never a second extraction.
    let mut attribute_lines = attribute_summaries(language, source, facts, &names, ontology);
    let attribute_fact_ids: std::collections::BTreeSet<String> = attribute_lines
        .iter()
        .flat_map(|summary| summary.evidence_fact_ids.iter().cloned())
        .collect();
    let mut metric_lines = metric_summaries(language, source, facts, &names, ontology);
    let metric_fact_ids: std::collections::BTreeSet<String> = metric_lines
        .iter()
        .flat_map(|summary| summary.evidence_fact_ids.iter().cloned())
        .collect();
    for fact in facts.iter().filter(|fact| fact.status == "asserted") {
        if attribute_fact_ids.contains(&fact.id) || metric_fact_ids.contains(&fact.id) {
            continue;
        }
        let Some(legal) = legal else {
            break;
        };
        if legal.micro.templates.is_empty() {
            break;
        }
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

    summaries.append(&mut attribute_lines);
    summaries.append(&mut metric_lines);

    // When several asserted facts about one subject are grounded in the same
    // source sentence, retain that complete sentence as a conservative
    // extractive summary candidate. This language-neutral path covers facts
    // without a paraphrase template while preserving every qualifier,
    // attribution, date, and negation verbatim. It never joins distant or
    // differently sourced facts.
    let mut co_evidenced: BTreeMap<(String, usize, usize), Vec<&EntityFact>> = BTreeMap::new();
    for fact in facts.iter().filter(|fact| fact.status == "asserted") {
        co_evidenced
            .entry((
                fact.subject_entity_id.clone(),
                fact.evidence_start_byte,
                fact.evidence_end_byte,
            ))
            .or_default()
            .push(fact);
    }
    for ((subject_id, start_byte, end_byte), grouped_facts) in co_evidenced {
        if grouped_facts.len() < 2 || !names.contains_key(subject_id.as_str()) {
            continue;
        }
        if summaries.iter().any(|summary| {
            summary.subject_entity_id.as_deref() == Some(subject_id.as_str())
                && summary.start_byte == start_byte
                && summary.end_byte == end_byte
        }) {
            continue;
        }
        let Some(evidence) = source.get(start_byte..end_byte) else {
            continue;
        };
        if evidence.chars().count() > 240
            || grouped_facts.iter().any(|fact| fact.evidence != evidence)
        {
            continue;
        }
        let mut fact_ids: Vec<String> = grouped_facts.iter().map(|fact| fact.id.clone()).collect();
        fact_ids.sort();
        fact_ids.dedup();
        if fact_ids.len() < 2 {
            continue;
        }
        let dates: std::collections::BTreeSet<&str> = grouped_facts
            .iter()
            .filter_map(|fact| fact.normalized_date.as_deref())
            .collect();
        summaries.push(MicroSummary {
            id: format!("micro_{:03}", summaries.len() + 1),
            kind: "micro_summary".to_string(),
            text: evidence.to_string(),
            evidence_fact_ids: fact_ids,
            subject_entity_id: Some(subject_id),
            normalized_date: (dates.len() == 1).then(|| dates.iter().next().unwrap().to_string()),
            sentence_index: grouped_facts[0].sentence_index,
            evidence: evidence.to_string(),
            start_byte,
            end_byte,
            score: 0.75,
        });
    }
    summaries.sort_by_key(|summary| (summary.sentence_index, summary.start_byte));
    // Ids are assigned after every source has contributed, so a legal phrase,
    // an attribute projection and an extractive sentence never collide on the
    // same id regardless of which sources exist for a language.
    for (index, summary) in summaries.iter_mut().enumerate() {
        summary.id = format!("micro_{:03}", index + 1);
    }

    let mut groups = Vec::new();
    if let Some(legal) = legal {
        groups.extend(comparison_groups(scene_id, &summaries, legal));
        groups.extend(sequence_groups(scene_id, &summaries, legal));
    }

    MicroBuild { summaries, groups }
}

/// Canonical attribute behind a fact. The age extractor keeps its historical
/// `personal_data` predicate, so the unit decides it; every other attribute
/// is named by the ontology.
fn canonical_attribute(
    fact: &EntityFact,
    language: &str,
    ontology: &AttributePredicates,
) -> Option<String> {
    // The legal age extractor names the claim `personal_data`; its unit was
    // never carried on the fact, so the predicate itself is the only marker.
    // The attribute projection also emits `age` facts in languages without a
    // legal pack, and both spellings resolve to the same canonical attribute.
    if fact.predicate == "personal_data" && fact.value_type == "NUMBER" {
        return Some("age".to_string());
    }
    ontology
        .label(&fact.predicate, language)
        .map(|_| fact.predicate.clone())
}

fn attribute_unit(
    fact: &EntityFact,
    canonical: &str,
    language: &str,
    ontology: &AttributePredicates,
) -> Option<String> {
    if canonical == "age" {
        let units = ontology.units("age", language);
        let plural = fact
            .normalized_value
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|value| value != 1.0);
        return units
            .get(usize::from(plural) * usize::from(units.len() > 1))
            .cloned()
            .or_else(|| Some("year".to_string()));
    }
    fact.unit.clone()
}

fn attribute_line(
    fact: &EntityFact,
    canonical: &str,
    language: &str,
    ontology: &AttributePredicates,
) -> Option<String> {
    let label = ontology.label(canonical, language)?;
    let value = fact
        .normalized_value
        .as_deref()
        .unwrap_or(fact.value.as_str());
    Some(match attribute_unit(fact, canonical, language, ontology) {
        Some(unit) => format!("{label} {value} {unit}"),
        None => format!("{label} {value}"),
    })
}

/// One compact line per person with at least two canonical attributes, in
/// source order and with the ontology's localized labels. No legal pack is
/// consulted, so the projection works in every supported language.
fn attribute_summaries(
    language: &str,
    source: &str,
    facts: &[EntityFact],
    names: &BTreeMap<&str, &str>,
    ontology: &AttributePredicates,
) -> Vec<MicroSummary> {
    let mut by_subject: BTreeMap<&str, Vec<(&EntityFact, String)>> = BTreeMap::new();
    for fact in facts.iter().filter(|fact| fact.status == "asserted") {
        let Some(canonical) = canonical_attribute(fact, language, ontology) else {
            continue;
        };
        let Some(line) = attribute_line(fact, &canonical, language, ontology) else {
            continue;
        };
        by_subject
            .entry(fact.subject_entity_id.as_str())
            .or_default()
            .push((fact, line));
    }
    let mut out = Vec::new();
    for (subject, mut attributes) in by_subject {
        if attributes.len() < 2 {
            continue;
        }
        let Some(name) = names.get(subject) else {
            continue;
        };
        attributes.sort_by_key(|(fact, _)| fact.start_byte);
        let text = format!(
            "{name}: {}",
            attributes
                .iter()
                .map(|(_, line)| line.as_str())
                .collect::<Vec<_>>()
                .join(" · ")
        );
        if text.chars().count() > 160 {
            continue;
        }
        let start = attributes
            .iter()
            .map(|(fact, _)| fact.evidence_start_byte)
            .min()
            .unwrap_or(0);
        let end = attributes
            .iter()
            .map(|(fact, _)| fact.evidence_end_byte)
            .max()
            .unwrap_or(start);
        let Some(evidence) = source.get(start..end) else {
            continue;
        };
        if evidence.chars().count() > 240 || evidence.trim().is_empty() {
            continue;
        }
        out.push(MicroSummary {
            id: format!("micro_attr_{:03}", out.len() + 1),
            kind: "attribute_summary".to_string(),
            text,
            evidence_fact_ids: attributes.iter().map(|(fact, _)| fact.id.clone()).collect(),
            subject_entity_id: Some(subject.to_string()),
            normalized_date: None,
            sentence_index: attributes
                .iter()
                .map(|(fact, _)| fact.sentence_index)
                .min()
                .unwrap_or(0),
            evidence: evidence.to_string(),
            start_byte: start,
            end_byte: end,
            score: 0.82,
        });
    }
    out
}

/// A fact that already carries a unit and a normalized date is a metric (a
/// fine, a capacity, a quoted amount). The projection only reformats it; it
/// never turns a raw chart quantity into a fact, so provenance stays exact.
fn metric_summaries(
    language: &str,
    source: &str,
    facts: &[EntityFact],
    names: &BTreeMap<&str, &str>,
    ontology: &AttributePredicates,
) -> Vec<MicroSummary> {
    let mut out = Vec::new();
    for fact in facts.iter().filter(|fact| fact.status == "asserted") {
        let (Some(unit), Some(date), Some(value)) = (
            fact.unit.as_deref(),
            fact.normalized_date.as_deref(),
            fact.normalized_value.as_deref(),
        ) else {
            continue;
        };
        if canonical_attribute(fact, language, ontology).is_some() {
            continue;
        }
        let Some(name) = names.get(fact.subject_entity_id.as_str()) else {
            continue;
        };
        let Some(year) = date.get(0..4) else {
            continue;
        };
        let text = format!("{year} — {name}: {value} {unit}");
        if text.chars().count() > 160 {
            continue;
        }
        let Some(evidence) = source.get(fact.evidence_start_byte..fact.evidence_end_byte) else {
            continue;
        };
        if evidence.trim().is_empty() {
            continue;
        }
        out.push(MicroSummary {
            id: format!("micro_metric_{:03}", out.len() + 1),
            kind: "metric_summary".to_string(),
            text,
            evidence_fact_ids: vec![fact.id.clone()],
            subject_entity_id: Some(fact.subject_entity_id.clone()),
            normalized_date: Some(date.to_string()),
            sentence_index: fact.sentence_index,
            evidence: evidence.to_string(),
            start_byte: fact.evidence_start_byte,
            end_byte: fact.evidence_end_byte,
            score: 0.78,
        });
    }
    out
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

    // Test-only fixture helper. The production `too_many_arguments` sites were
    // replaced by context structs; this one is not compiled into the binary.
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
        assert_eq!(
            summary.text,
            "Alberto Stasi: nessuna condanna nel procedimento descritto"
        );
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
            fact(
                "f1",
                &id,
                0,
                "legal_outcome",
                "convicted",
                Some("2014-12-17"),
                "asserted",
                "Nel 2014 arrivò la condanna.",
                spans[0],
            ),
            fact(
                "f2",
                &id,
                1,
                "legal_outcome",
                "acquitted",
                Some("2011-10-27"),
                "asserted",
                "Nel 2011 arrivò l'assoluzione.",
                spans[1],
            ),
            fact(
                "f3",
                &id,
                2,
                "legal_finality",
                "final",
                Some("2015-06-11"),
                "asserted",
                "Nel 2015 la condanna divenne definitiva.",
                spans[2],
            ),
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
            fact(
                "f1",
                &id,
                0,
                "legal_outcome",
                outcome,
                Some("2011-10-27"),
                "asserted",
                "Alberto Stasi fu assolto.",
                spans[0],
            ),
            fact(
                "f2",
                &id,
                1,
                "legal_outcome",
                outcome,
                Some("2011-10-27"),
                "asserted",
                "Alberto Stasi fu assolto.",
                spans[1],
            ),
            // The same claim far apart in the scene is not a nearby repeat:
            // it keeps the first deterministic evidence instead of merging.
            fact(
                "f3",
                &id,
                3,
                "legal_outcome",
                outcome,
                Some("2011-10-27"),
                "asserted",
                "Alberto Stasi fu assolto.",
                spans[3],
            ),
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
        let (source, spans) = spans_of(&["Alberto Stasi è indagato.", "Alberto Stasi è indagato."]);
        let facts = vec![
            fact(
                "f1",
                &id,
                0,
                "investigation_status",
                "under_investigation",
                Some("2015"),
                "asserted",
                "Alberto Stasi è indagato.",
                spans[0],
            ),
            fact(
                "f2",
                &id,
                1,
                "investigation_status",
                "under_investigation",
                Some("2016"),
                "asserted",
                "Alberto Stasi è indagato.",
                spans[1],
            ),
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
        let (source, spans) = spans_of(&["Alberto Stasi è indagato.", "Alberto Stasi è indagato."]);
        let facts = vec![
            fact(
                "f1",
                &id_a,
                0,
                "investigation_status",
                "under_investigation",
                Some("2015"),
                "asserted",
                "Alberto Stasi è indagato.",
                spans[0],
            ),
            fact(
                "f2",
                &id_b,
                1,
                "investigation_status",
                "under_investigation",
                Some("2015"),
                "asserted",
                "Alberto Stasi è indagato.",
                spans[1],
            ),
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

    #[test]
    fn a_fact_with_a_unit_and_a_date_becomes_one_metric_summary() {
        let (entity, id) = person();
        let (source, spans) = spans_of(&["Alberto Stasi fu multato di 5.000 euro nel 2019."]);
        let mut metric = fact(
            "f_fine",
            &id,
            0,
            "fine",
            "5000",
            Some("2019-03-01"),
            "asserted",
            &source,
            spans[0],
        );
        metric.value = "5.000 euro".to_string();
        metric.value_type = "MONEY".to_string();
        metric.unit = Some("EUR".to_string());
        let built = build("xx", "scena", &source, &[metric], &[entity], &[id]);
        assert_eq!(built.summaries.len(), 1, "{:?}", built.summaries);
        assert_eq!(built.summaries[0].kind, "metric_summary");
        assert_eq!(built.summaries[0].text, "2019 — Alberto Stasi: 5000 EUR");
        assert_eq!(built.summaries[0].evidence_fact_ids, vec!["f_fine"]);
        assert_eq!(built.summaries[0].evidence, source);
    }
}
