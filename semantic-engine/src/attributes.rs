//! Grounded person attributes projected from facts the engine already owns.
//!
//! This module is a projection, not a second extractor:
//!
//! * `height` and `weight` come from typed [`Quantity`] values that
//!   [`crate::entity_fact_resolver`] binds to their subject through Universal
//!   Dependencies;
//! * `age` comes from [`PersonAgeFact`];
//! * `profession` comes from [`PersonRole`].
//!
//! Every attribute keeps the exact value span, the verbatim sentence and the
//! canonical fact id that carries the same claim. Negated and uncertain claims
//! are inventoried as facts but never enter a positive card.

use std::collections::BTreeMap;

use semantic_core::lexicon::AttributePredicates;
use semantic_core::{
    EntityAttribute, EntityAttributeList, EntityFact, PersonAgeFact, PersonRole, Quantity,
    RoleStatus, SentenceSpan,
};
use visualner::{AnalyzedToken, VisualEntity};

use crate::entity_fact_resolver::{self, ResolvedAttributeBinding};

/// Attribute lists plus the entity facts that back them.
#[derive(Debug, Default)]
pub(crate) struct AttributeBuild {
    pub(crate) lists: Vec<EntityAttributeList>,
    pub(crate) facts: Vec<EntityFact>,
}

struct DraftAttribute {
    entity_index: usize,
    predicate: String,
    /// An existing fact with the same claim (the age extractor already
    /// registers one). Reusing its id prevents a duplicate fact and lets the
    /// grouped card represent that same canonical fact.
    reuse_fact_id: Option<String>,
    /// Display value (the bare number for an age).
    value: String,
    /// Exact source slice the fact's `start_byte..end_byte` covers; the Go
    /// boundary requires `source[start..end] == fact.value`.
    fact_value: String,
    normalized_value: Option<f64>,
    normalized_text: Option<String>,
    value_type: &'static str,
    unit: Option<String>,
    unit_surface: Option<String>,
    qualifier: String,
    status: String,
    start_byte: usize,
    end_byte: usize,
    evidence_start: usize,
    evidence_end: usize,
    sentence_index: usize,
    evidence: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn extract(
    language: &str,
    source: &str,
    sentences: &[SentenceSpan],
    tokens: &[AnalyzedToken],
    entities: &[VisualEntity],
    entity_ids: &[String],
    quantities: &[Quantity],
    age_facts: &[PersonAgeFact],
    roles: &[PersonRole],
    existing_facts: &[EntityFact],
    ontology: &AttributePredicates,
) -> AttributeBuild {
    if entities.len() != entity_ids.len() {
        return AttributeBuild::default();
    }
    let bindings =
        entity_fact_resolver::resolve(language, sentences, tokens, entities, quantities, ontology);
    let mut drafts = quantity_drafts(source, sentences, quantities, &bindings);
    drafts.extend(age_drafts(
        language,
        source,
        sentences,
        entity_ids,
        age_facts,
        existing_facts,
        ontology,
    ));
    drafts.extend(role_drafts(source, sentences, entity_ids, roles));
    if drafts.is_empty() {
        return AttributeBuild::default();
    }
    drafts.sort_by_key(|draft| {
        (
            draft.entity_index,
            draft.start_byte,
            draft.predicate.clone(),
        )
    });

    let mut facts = Vec::with_capacity(drafts.len());
    for draft in &drafts {
        if draft.reuse_fact_id.is_some() {
            continue;
        }
        let Some(entity_id) = entity_ids.get(draft.entity_index) else {
            continue;
        };
        facts.push(EntityFact {
            id: format!(
                "{}_attribute_{}_{}",
                entity_id, draft.predicate, draft.start_byte
            ),
            subject_entity_id: entity_id.clone(),
            predicate: draft.predicate.clone(),
            value: draft.fact_value.clone(),
            value_type: draft.value_type.to_string(),
            normalized_value: draft
                .normalized_text
                .clone()
                .or_else(|| draft.normalized_value.map(|value| value.to_string())),
            normalized_date: None,
            target_entity_id: None,
            unit: draft.unit.clone(),
            operator: None,
            status: draft.status.clone(),
            sentence_index: draft.sentence_index,
            evidence: draft.evidence.clone(),
            start_byte: draft.start_byte,
            end_byte: draft.end_byte,
            evidence_start_byte: draft.evidence_start,
            evidence_end_byte: draft.evidence_end,
            importance: 0.0,
            visual_concept_ids: Vec::new(),
        });
    }

    AttributeBuild {
        lists: attribute_lists(source, entities, entity_ids, &drafts),
        facts,
    }
}

fn quantity_drafts(
    source: &str,
    sentences: &[SentenceSpan],
    quantities: &[Quantity],
    bindings: &[ResolvedAttributeBinding],
) -> Vec<DraftAttribute> {
    let mut out = Vec::new();
    for binding in bindings {
        let Some(quantity) = quantities.get(binding.quantity_index) else {
            continue;
        };
        let Some(sentence) = sentences.get(quantity.sentence_index) else {
            continue;
        };
        let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let unit_surface = if quantity.unit_surface.is_empty() {
            quantity.unit.clone()
        } else {
            quantity.unit_surface.clone()
        };
        out.push(DraftAttribute {
            entity_index: binding.entity_index,
            predicate: binding.predicate.clone(),
            reuse_fact_id: None,
            value: quantity.evidence.clone(),
            fact_value: quantity.evidence.clone(),
            normalized_value: quantity.raw_value,
            normalized_text: None,
            value_type: "NUMBER",
            unit: Some(quantity.unit.clone()),
            unit_surface: Some(unit_surface),
            qualifier: quantity.qualifier.clone(),
            status: quantity.status.clone(),
            start_byte: quantity.start_byte,
            end_byte: quantity.end_byte,
            evidence_start: sentence.start_byte,
            evidence_end: sentence.end_byte,
            sentence_index: quantity.sentence_index,
            evidence: evidence.to_string(),
        });
    }
    out
}

fn age_drafts(
    language: &str,
    source: &str,
    sentences: &[SentenceSpan],
    entity_ids: &[String],
    age_facts: &[PersonAgeFact],
    existing_facts: &[EntityFact],
    ontology: &AttributePredicates,
) -> Vec<DraftAttribute> {
    let mut out = Vec::new();
    for age in age_facts {
        let Some(entity_index) = entity_ids.iter().position(|id| id == &age.person_id) else {
            continue;
        };
        let Some(sentence) = sentences.get(age.sentence_index) else {
            continue;
        };
        let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let units = ontology.units("age", language);
        let unit_surface = if age.age == 1 {
            units.first()
        } else {
            units.get(1).or_else(|| units.first())
        }
        .cloned()
        .unwrap_or_else(|| "year".to_string());
        let reuse_fact_id = existing_facts
            .iter()
            .find(|fact| {
                fact.subject_entity_id == age.person_id
                    && fact.start_byte == age.start_byte
                    && fact.end_byte == age.end_byte
                    && matches!(fact.predicate.as_str(), "personal_data" | "age")
            })
            .map(|fact| fact.id.clone());
        out.push(DraftAttribute {
            entity_index,
            predicate: "age".to_string(),
            reuse_fact_id,
            value: age.age.to_string(),
            fact_value: source
                .get(age.start_byte..age.end_byte)
                .filter(|slice| !slice.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| age.age.to_string()),
            normalized_value: Some(f64::from(age.age)),
            normalized_text: Some(age.age.to_string()),
            value_type: "NUMBER",
            unit: Some("year".to_string()),
            unit_surface: Some(unit_surface),
            qualifier: "exact".to_string(),
            status: sentence_status(source, sentence).to_string(),
            start_byte: age.start_byte,
            end_byte: age.end_byte,
            evidence_start: sentence.start_byte,
            evidence_end: sentence.end_byte,
            sentence_index: age.sentence_index,
            evidence: evidence.to_string(),
        });
    }
    out
}

fn role_drafts(
    source: &str,
    sentences: &[SentenceSpan],
    entity_ids: &[String],
    roles: &[PersonRole],
) -> Vec<DraftAttribute> {
    let mut out = Vec::new();
    for role in roles {
        let Some(entity_index) = entity_ids.iter().position(|id| id == &role.person_id) else {
            continue;
        };
        let Some(sentence) = sentences.get(role.sentence_index) else {
            continue;
        };
        let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        out.push(DraftAttribute {
            entity_index,
            predicate: "profession".to_string(),
            reuse_fact_id: None,
            value: role.role_surface.clone(),
            fact_value: role.role_surface.clone(),
            normalized_value: None,
            normalized_text: Some(role.role_code.clone()),
            value_type: "TEXT",
            unit: None,
            unit_surface: None,
            qualifier: "exact".to_string(),
            status: match role.status {
                RoleStatus::Asserted => "asserted",
                RoleStatus::Negated => "negated",
                RoleStatus::Uncertain => "uncertain",
            }
            .to_string(),
            start_byte: role.start_byte,
            end_byte: role.end_byte,
            evidence_start: sentence.start_byte,
            evidence_end: sentence.end_byte,
            sentence_index: role.sentence_index,
            evidence: evidence.to_string(),
        });
    }
    out
}

fn sentence_status(source: &str, sentence: &SentenceSpan) -> &'static str {
    let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
        return "asserted";
    };
    crate::facts::fact_status_lower(&text.to_lowercase())
}

fn attribute_lists(
    source: &str,
    entities: &[VisualEntity],
    entity_ids: &[String],
    drafts: &[DraftAttribute],
) -> Vec<EntityAttributeList> {
    let mut by_entity: BTreeMap<usize, Vec<&DraftAttribute>> = BTreeMap::new();
    for draft in drafts.iter().filter(|draft| draft.status == "asserted") {
        by_entity.entry(draft.entity_index).or_default().push(draft);
    }
    let mut lists = Vec::new();
    for (entity_index, mut attributes) in by_entity {
        attributes.sort_by_key(|draft| (draft.start_byte, draft.predicate.clone()));
        // One value per predicate keeps the card a summary. A second
        // measurement of the same property stays in the fact inventory.
        let mut seen = std::collections::HashSet::new();
        attributes.retain(|draft| seen.insert(draft.predicate.clone()));
        let Some(first) = attributes.first() else {
            continue;
        };
        let evidence_start = first.evidence_start;
        let evidence_end = attributes.last().map_or(first.evidence_end, |draft| {
            draft.evidence_end.max(first.evidence_end)
        });
        let Some(evidence) = source.get(evidence_start..evidence_end) else {
            continue;
        };
        let Some(entity_id) = entity_ids.get(entity_index) else {
            continue;
        };
        lists.push(EntityAttributeList {
            entity_id: entity_id.clone(),
            label: entity_label(entities, entity_ids, entity_id, entity_index),
            attributes: attributes
                .iter()
                .map(|draft| EntityAttribute {
                    predicate: draft.predicate.clone(),
                    value: draft.value.clone(),
                    normalized_value: draft.normalized_value,
                    unit: draft.unit.clone(),
                    unit_surface: draft.unit_surface.clone(),
                    qualifier: draft.qualifier.clone(),
                    status: draft.status.clone(),
                    start_byte: draft.start_byte,
                    end_byte: draft.end_byte,
                    sentence_index: draft.sentence_index,
                    evidence: draft.evidence.clone(),
                    source_fact_id: draft.reuse_fact_id.clone().unwrap_or_else(|| {
                        format!(
                            "{}_attribute_{}_{}",
                            entity_id, draft.predicate, draft.start_byte
                        )
                    }),
                })
                .collect(),
            start_byte: evidence_start,
            end_byte: evidence_end,
            sentence_index: first.sentence_index,
            evidence: evidence.to_string(),
        });
    }
    lists
}

fn entity_label(
    entities: &[VisualEntity],
    entity_ids: &[String],
    entity_id: &str,
    entity_index: usize,
) -> String {
    entities
        .iter()
        .enumerate()
        .filter(|(index, _)| entity_ids.get(*index).is_some_and(|id| id == entity_id))
        .map(|(index, entity)| (entity.text.chars().count(), index, entity))
        .max_by_key(|(length, index, _)| (*length, std::cmp::Reverse(*index)))
        .map(|(_, _, entity)| entity.text.clone())
        .or_else(|| entities.get(entity_index).map(|entity| entity.text.clone()))
        .unwrap_or_default()
}
