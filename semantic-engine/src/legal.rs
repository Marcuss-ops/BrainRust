//! Conservative legal/procedural fact extraction.
//!
//! The facility facts in `lib.rs` only cover buildings. Long editorial scripts
//! about investigations and trials need the same atomic-fact treatment for
//! people, courts and documents: who was acquitted, who was convicted, which
//! court confirmed which decision, and on what date.
//!
//! This module is deliberately lexical and conservative:
//!
//! - the subject is always a grounded PERSON/ORG entity in the same sentence;
//! - a date-backed predicate is emitted only when the sentence itself carries
//!   the date, so a nearby sentence cannot supply the wrong year;
//! - negation and uncertainty are preserved as `negated`/`uncertain` instead
//!   of being flipped into the opposite claim;
//! - nothing is inferred beyond the marker (no ownership of seized objects,
//!   no "innocent" from "not convicted", no guilt from an investigation).
//!
//! The output uses the existing [`EntityFact`] contract, so the manifest,
//! grounding checks and the Go adapter do not need a second fact shape.

use std::collections::BTreeSet;

use semantic_core::dates::TemporalFact;
use semantic_core::legal_packs;
use semantic_core::lexicon::{LegalPattern, SubjectKind, ValueKind};
use semantic_core::numbers::NumericFact;
use semantic_core::relations::{clause_start, clause_status, RelationStatus};
use semantic_core::EntityFact;
use visualner::VisualEntity;

use crate::contains_any;

// The marker table lives in the language's domain pack
// (`lexicon/legal/<lang>.json`): markers are matched case-insensitively inside
// one canonical sentence, longer, more specific phrases before their fragments
// so the first match carries the sharper meaning. Extend the domain by editing
// that data file, never this module. A language without a pack yields no legal
// facts at all.

/// Extracts legal/procedural facts for one scene.
pub fn extract(
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    entity_ids: &[String],
    temporal_facts: &[TemporalFact],
    numeric_facts: &[NumericFact],
) -> Vec<EntityFact> {
    // The legal layer is an optional domain pack: no pack means no legal
    // facts (the generic fact pipeline still runs), never another language's
    // markers applied to this text.
    let Some(legal) = legal_packs::get(language) else {
        return Vec::new();
    };
    let sentences = semantic_core::split_sentences(source, language, None);
    // Editorial legal narration often names the person once and then reports
    // agentless outcomes ("il giudice pronunciò una prima assoluzione"). The
    // fallback below attributes such a sentence to the only person mentioned
    // in the preceding window; two candidate people disable the fallback
    // instead of guessing.
    let persons_per_sentence: Vec<Vec<usize>> = sentences
        .iter()
        .map(|sentence| {
            entities
                .iter()
                .enumerate()
                .filter(|(_, entity)| {
                    entity.r#type == "PERSON"
                        && entity.start >= sentence.start_byte
                        && entity.end <= sentence.end_byte
                })
                .map(|(index, _)| index)
                .collect()
        })
        .collect();
    let mut out: Vec<EntityFact> = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let lower = text.to_lowercase();
        // Byte offsets are only reusable when lowercasing did not change the
        // length (true for the supported Latin scripts). Otherwise the
        // sentence is skipped rather than risk a wrong evidence span.
        if lower.len() != text.len() {
            continue;
        }
        let in_sentence: Vec<usize> = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| {
                entity.start >= sentence.start_byte && entity.end <= sentence.end_byte
            })
            .map(|(index, _)| index)
            .collect();
        let fallback_person = recent_person(&persons_per_sentence, sentence_index, entities);
        // "Questa condanna diventa definitiva" is a finality fact, not a new
        // conviction. The generic "condanna" marker must not duplicate it.
        let finality_sentence = contains_any(&lower, &legal.finality_markers);
        // "Confermò l'assoluzione" is a confirmation, not a second acquittal.
        // The generic marker ("assoluzione") sits inside the specific one, so
        // only that nested occurrence yields; a separate 2014 acquittal in the
        // same sentence as a 2016 confirmation must survive.
        let confirmation_spans = confirmation_spans(&lower, &legal.patterns);
        for pattern in &legal.patterns {
            // "16 anni di reclusione" is a sentence length, not the person's
            // age: the age pattern must not steal the number.
            if pattern.normalized == "age" && contains_any(&lower, &legal.age_guard_markers) {
                continue;
            }
            if pattern.normalized == "convicted" && finality_sentence {
                continue;
            }
            for marker in &pattern.markers {
                let Some(relative) = lower.find(marker.as_str()) else {
                    continue;
                };
                if matches!(pattern.normalized.as_str(), "acquitted" | "convicted")
                    && confirmation_spans
                        .iter()
                        .any(|&(start, end)| relative >= start && relative + marker.len() <= end)
                {
                    continue;
                }
                let marker_start = sentence.start_byte + relative;
                let marker_end = marker_start + marker.len();
                let subject_index = if pattern.normalized == "final" {
                    // The conviction becomes final for the person who was
                    // convicted, not for the court named after the marker
                    // ("la condanna divenne definitiva in Cassazione").
                    finality_subject(entities, &in_sentence, marker_start, fallback_person)
                } else {
                    pick_subject(entities, &in_sentence, marker_start, pattern.subject_kind())
                        .or_else(|| {
                            let index = fallback_person?;
                            subject_matches_kind(entities, index, pattern.subject_kind())
                                .then_some(index)
                        })
                };
                let Some(subject_index) = subject_index else {
                    continue;
                };
                let Some(resolved) = resolve_value(
                    source,
                    sentence.start_byte,
                    sentence.end_byte,
                    marker_start,
                    marker_end,
                    pattern,
                    entities,
                    entity_ids,
                    &in_sentence,
                    subject_index,
                    temporal_facts,
                    numeric_facts,
                ) else {
                    continue;
                };
                let ResolvedValue {
                    value,
                    value_type,
                    normalized,
                    normalized_date,
                    start,
                    end,
                    target,
                } = resolved;
                let status = status_for(text, relative, pattern.normalized.as_str(), language);
                let subject_entity_id = entity_ids[subject_index].clone();
                if out.iter().any(|fact| {
                    fact.subject_entity_id == subject_entity_id
                        && fact.predicate == pattern.predicate
                        && fact.normalized_value.as_deref() == Some(normalized.as_str())
                        && fact.start_byte == start
                }) {
                    continue;
                }
                out.push(EntityFact {
                    id: format!(
                        "{}_{}_{:02}",
                        subject_entity_id,
                        pattern.normalized,
                        out.len() + 1
                    ),
                    subject_entity_id,
                    predicate: pattern.predicate.to_string(),
                    value,
                    value_type,
                    normalized_value: Some(normalized),
                    normalized_date,
                    target_entity_id: target,
                    unit: None,
                    operator: None,
                    status,
                    sentence_index,
                    evidence: text.to_string(),
                    start_byte: start,
                    end_byte: end,
                    evidence_start_byte: sentence.start_byte,
                    evidence_end_byte: sentence.end_byte,
                    importance: 0.0,
                    visual_concept_ids: Vec::new(),
                });
            }
        }
    }
    out
}

/// Byte spans of confirmation markers in the lowercased sentence. A generic
/// `assoluzione`/`condanna` nested inside one of these spans is the same event
/// restated, not a second outcome.
fn confirmation_spans(sentence: &str, patterns: &[LegalPattern]) -> Vec<(usize, usize)> {
    patterns
        .iter()
        .filter(|pattern| {
            matches!(
                pattern.normalized.as_str(),
                "acquittal_confirmed" | "conviction_confirmed"
            )
        })
        .flat_map(|pattern| pattern.markers.iter())
        .filter_map(|marker| {
            sentence
                .find(marker.as_str())
                .map(|start| (start, start + marker.len()))
        })
        .collect()
}

/// Finality belongs to the convicted person, even when the sentence names the
/// court after the marker. Prefer a person in this sentence, then the only
/// recent person; an org is the last resort, not the first match by distance.
fn finality_subject(
    entities: &[VisualEntity],
    in_sentence: &[usize],
    marker_start: usize,
    fallback_person: Option<usize>,
) -> Option<usize> {
    if let Some(person) = pick_subject(entities, in_sentence, marker_start, SubjectKind::Person) {
        return Some(person);
    }
    if let Some(person) = fallback_person {
        if subject_matches_kind(entities, person, SubjectKind::Person) {
            return Some(person);
        }
    }
    pick_subject(
        entities,
        in_sentence,
        marker_start,
        SubjectKind::PersonOrOrg,
    )
}

fn pick_subject(
    entities: &[VisualEntity],
    in_sentence: &[usize],
    marker_start: usize,
    kind: SubjectKind,
) -> Option<usize> {
    in_sentence
        .iter()
        .copied()
        .filter(|&index| subject_matches_kind(entities, index, kind))
        .min_by_key(|&index| {
            let start = entities[index].start;
            marker_start.abs_diff(start)
        })
}

fn subject_matches_kind(entities: &[VisualEntity], index: usize, kind: SubjectKind) -> bool {
    match kind {
        SubjectKind::Person => entities[index].r#type == "PERSON",
        SubjectKind::Org => matches!(entities[index].r#type.as_str(), "ORG" | "ORGANIZATION"),
        SubjectKind::PersonOrOrg => matches!(
            entities[index].r#type.as_str(),
            "PERSON" | "ORG" | "ORGANIZATION"
        ),
        SubjectKind::LegalActor => matches!(
            entities[index].r#type.as_str(),
            "PERSON" | "ORG" | "ORGANIZATION" | "ROLE"
        ),
    }
}

/// The only PERSON mentioned in the six sentences before `index`, or None
/// when the window names zero or several people. Legal narration keeps one
/// subject across several sentences (outcome, appeal, Cassation, finality);
/// two names make the outcome ambiguous, and a wrong attribution is worse
/// than a missing one.
fn recent_person(
    persons_per_sentence: &[Vec<usize>],
    index: usize,
    entities: &[VisualEntity],
) -> Option<usize> {
    let mut people = BTreeSet::new();
    for previous in index.saturating_sub(6)..index {
        people.extend(persons_per_sentence.get(previous)?.iter().copied());
    }
    // "Alberto Stasi" and a later bare "Stasi" are the same person, so the
    // window must not read them as two candidates. Surfaces that share every
    // token collapse into one; the longest surface wins so the fact is
    // attributed to the canonical full name.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for candidate in people {
        let text = entities[candidate].text.to_lowercase();
        let tokens: BTreeSet<&str> = text.split_whitespace().collect();
        let merged = groups.iter_mut().find(|group| {
            group.iter().any(|member| {
                let other = entities[*member].text.to_lowercase();
                let other_tokens: BTreeSet<&str> = other.split_whitespace().collect();
                tokens.is_subset(&other_tokens) || other_tokens.is_subset(&tokens)
            })
        });
        match merged {
            Some(group) => group.push(candidate),
            None => groups.push(vec![candidate]),
        }
    }
    if groups.len() != 1 {
        return None;
    }
    groups.pop().and_then(|group| {
        group.into_iter().max_by_key(|index| {
            (
                entities[*index].text.len(),
                std::cmp::Reverse(entities[*index].start),
            )
        })
    })
}

/// The value side of one legal fact: the verbatim surface, its canonical
/// conclusion, and the ISO date backing a dated outcome.
struct ResolvedValue {
    value: String,
    value_type: String,
    normalized: String,
    normalized_date: Option<String>,
    start: usize,
    end: usize,
    target: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn resolve_value(
    source: &str,
    sentence_start: usize,
    sentence_end: usize,
    marker_start: usize,
    marker_end: usize,
    pattern: &LegalPattern,
    entities: &[VisualEntity],
    entity_ids: &[String],
    in_sentence: &[usize],
    subject_index: usize,
    temporal_facts: &[TemporalFact],
    numeric_facts: &[NumericFact],
) -> Option<ResolvedValue> {
    match pattern.value_kind() {
        ValueKind::Date => {
            let in_sentence: Vec<&TemporalFact> = temporal_facts
                .iter()
                .filter(|fact| fact.start_byte >= sentence_start && fact.end_byte <= sentence_end)
                .collect();
            // A sentence can state several dates: "Nel 2014 ... una prima
            // assoluzione, che nel 2016 viene confermata". The outcome marker
            // belongs to the date stated before it; only a sentence with no
            // preceding date may use a following one.
            let before = in_sentence
                .iter()
                .copied()
                .filter(|fact| fact.end_byte <= marker_start)
                .max_by_key(|fact| fact.end_byte);
            let after = in_sentence
                .iter()
                .copied()
                .filter(|fact| fact.start_byte >= marker_end)
                .min_by_key(|fact| fact.start_byte);
            let fact = before.or(after)?;
            let date = fact
                .normalized
                .clone()
                .unwrap_or_else(|| fact.surface.clone());
            Some(ResolvedValue {
                value: fact.surface.clone(),
                value_type: "DATE".to_string(),
                normalized: pattern.normalized.clone(),
                normalized_date: Some(date),
                start: fact.start_byte,
                end: fact.end_byte,
                target: None,
            })
        }
        ValueKind::Number => {
            let fact = numeric_facts
                .iter()
                .filter(|fact| fact.start_byte >= sentence_start && fact.end_byte <= sentence_end)
                .min_by_key(|fact| marker_start.abs_diff(fact.start_byte))?;
            let normalized = fact
                .value
                .map(|value| value.to_string())
                .unwrap_or_else(|| fact.surface.clone());
            Some(ResolvedValue {
                value: fact.surface.clone(),
                value_type: fact.kind.clone(),
                normalized,
                normalized_date: None,
                start: fact.start_byte,
                end: fact.end_byte,
                target: None,
            })
        }
        ValueKind::Entity => {
            let target = in_sentence
                .iter()
                .copied()
                .filter(|&index| index != subject_index)
                .filter(|&index| {
                    matches!(
                        entities[index].r#type.as_str(),
                        "PERSON" | "ORG" | "ORGANIZATION"
                    )
                })
                .filter(|&index| entities[index].start >= marker_end)
                .min_by_key(|&index| entities[index].start)?;
            let entity = &entities[target];
            Some(ResolvedValue {
                value: entity.text.clone(),
                value_type: "ENTITY".to_string(),
                normalized: pattern.normalized.clone(),
                normalized_date: None,
                start: entity.start,
                end: entity.end,
                target: Some(entity_ids[target].clone()),
            })
        }
        ValueKind::Marker => {
            let value = source.get(marker_start..marker_end)?.to_string();
            Some(ResolvedValue {
                value,
                value_type: "EVENT".to_string(),
                normalized: pattern.normalized.clone(),
                normalized_date: None,
                start: marker_start,
                end: marker_end,
                target: None,
            })
        }
    }
}

/// Negation is read from the clause before the marker, so a negation in an
/// earlier clause cannot flip a later assertion. `not_convicted` and
/// `denies_accusations` describe a negative state directly, so they keep the
/// `asserted` status: the text asserts that absence.
/// Negation and uncertainty are read from the clause before the marker via
/// the shared status helpers, so a negation in an earlier clause cannot flip
/// a later assertion. `not_convicted` and `denies_accusations` describe a
/// negative state directly, so they keep the `asserted` status: the text
/// asserts that absence. Legal facts consult the Italian and English
/// negation vocabularies regardless of scene language.
fn status_for(sentence: &str, marker_relative: usize, normalized: &str, language: &str) -> String {
    if matches!(normalized, "not_convicted" | "denies_accusations") {
        return "asserted".to_string();
    }
    let prefix = &sentence[..marker_relative];
    let clause = &sentence[clause_start(prefix)..marker_relative];
    // The pack guarantees this language has legal negation vocabulary.
    match clause_status(clause, &[language]) {
        RelationStatus::Negated => "negated".to_string(),
        RelationStatus::Uncertain => "uncertain".to_string(),
        RelationStatus::Asserted => "asserted".to_string(),
    }
}

/// Curated legal actors that a general tokenizer cannot type reliably.
/// The spans are verbatim and only the semantic type is added; no fact is
/// inferred from the presence of an organization alone.
pub fn collect_entities(source: &str, language: &str) -> Vec<VisualEntity> {
    let mut out: Vec<VisualEntity> = Vec::new();
    let Some(legal) = legal_packs::get(language) else {
        return out;
    };
    for cue in &legal.entity_cues {
        collect_cue(source, cue.cue.as_str(), cue.kind.as_str(), &mut out);
    }
    // The bare institution is a valid alias only where a longer span was not
    // already captured ("la Cassazione", "la Procura").
    for cue in &legal.entity_alias_cues {
        for (start, _) in source.match_indices(cue.cue.as_str()) {
            if out
                .iter()
                .any(|entity| entity.start <= start && start < entity.end)
            {
                continue;
            }
            push(
                &mut out,
                cue.cue.clone(),
                cue.kind.as_str(),
                start,
                start + cue.cue.len(),
            );
        }
    }
    out.sort_by_key(|entity| (entity.start, entity.end));
    out
}

fn collect_cue(source: &str, cue: &str, kind: &str, out: &mut Vec<VisualEntity>) {
    let trailing_space = cue.ends_with(' ');
    for (start, _) in source.match_indices(cue) {
        if start > 0 {
            let previous = source[..start].chars().next_back();
            if previous.is_some_and(|ch| ch.is_alphanumeric()) {
                continue;
            }
        }
        let mut end = start + cue.len();
        if trailing_space {
            // "Procura di Pavia": capture the following title-cased place.
            let tail = &source[end..];
            let mut cursor = 0usize;
            for token in tail.split_whitespace().take(2) {
                let core = token.trim_end_matches(|ch: char| ch.is_ascii_punctuation());
                let Some(first) = core.chars().next() else {
                    break;
                };
                if !first.is_uppercase() {
                    break;
                }
                let Some(relative) = tail[cursor..].find(token) else {
                    break;
                };
                end = end + relative + core.len();
                cursor += relative + token.len();
                if core.len() != token.len() {
                    break;
                }
            }
            if end == start + cue.len() {
                continue;
            }
        }
        push(out, source[start..end].to_string(), kind, start, end);
    }
}

fn push(out: &mut Vec<VisualEntity>, text: String, kind: &str, start: usize, end: usize) {
    if let Some(existing) = out
        .iter_mut()
        .find(|entity| entity.start < end && start < entity.end)
    {
        // Keep the widest span; a longer, more specific span wins.
        if end - start > existing.end - existing.start {
            existing.text = text;
            existing.r#type = kind.to_string();
            existing.start = start;
            existing.end = end;
            existing.evidence = existing.text.clone();
        }
        return;
    }
    out.push(VisualEntity {
        text: text.clone(),
        r#type: kind.to_string(),
        score: 0.9,
        start,
        end,
        evidence: text,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use visualner::ExtractOptions;

    fn facts(text: &str) -> (Vec<EntityFact>, Vec<String>) {
        let analysis = visualner::analyze(
            text,
            &ExtractOptions {
                language: "it".to_string(),
                entity_count: 100,
            },
        )
        .expect("analysis");
        let ids: Vec<String> = analysis
            .entities
            .iter()
            .enumerate()
            .map(|(index, entity)| format!("e{}:{}", index, entity.text))
            .collect();
        let facts = extract(
            text,
            "it",
            &analysis.entities,
            &ids,
            &analysis.temporal_facts,
            &analysis.numeric_facts,
        );
        (facts, ids)
    }

    #[test]
    fn acquittal_keeps_the_subject_and_the_same_sentence_date() {
        let (facts, _) = facts("Il 17 dicembre 2009 Alberto Stasi fu assolto.");
        let fact = facts
            .iter()
            .find(|fact| fact.normalized_value.as_deref() == Some("acquitted"))
            .unwrap_or_else(|| panic!("acquittal fact: {facts:?}"));
        assert_eq!(fact.predicate, "legal_outcome");
        assert_eq!(fact.status, "asserted");
        assert_eq!(fact.value, "17 dicembre 2009");
        assert!(fact.id.contains("acquitted"));
    }

    #[test]
    fn negated_conviction_is_preserved_as_negated() {
        let (facts, _) = facts("Alberto Stasi non è stato condannato in quel procedimento.");
        let fact = facts
            .iter()
            .find(|fact| fact.normalized_value.as_deref() == Some("not_convicted"))
            .expect("not-convicted fact");
        assert_eq!(fact.status, "asserted");
        assert!(!facts.iter().any(|fact| {
            fact.normalized_value.as_deref() == Some("convicted") && fact.status == "asserted"
        }));
    }

    #[test]
    fn a_second_date_in_the_sentence_does_not_override_the_outcome_date() {
        let (facts, _) = facts(
            "Alberto Stasi fu rinviato a giudizio. Nel 2014 il giudice pronunciò una prima assoluzione, che nel 2016 fu confermata in appello.",
        );
        let acquitted = facts
            .iter()
            .find(|fact| fact.normalized_value.as_deref() == Some("acquitted"))
            .expect("acquittal fact");
        assert_eq!(acquitted.normalized_date.as_deref(), Some("2014"));
    }

    #[test]
    fn finality_prefers_the_convicted_person_over_the_court_after_the_marker() {
        let (facts, ids) = facts(
            "Il 17 dicembre 2014 la Corte d'Assise d'Appello di Milano pronunciò una condanna a sedici anni per Alberto Stasi. Il 12 dicembre 2015 la condanna divenne definitiva in Cassazione.",
        );
        let person = ids
            .iter()
            .find(|id| id.contains("Alberto Stasi"))
            .expect("person id");
        let final_fact = facts
            .iter()
            .find(|fact| fact.normalized_value.as_deref() == Some("final"))
            .unwrap_or_else(|| panic!("final fact: {facts:?}"));
        assert_eq!(&final_fact.subject_entity_id, person);
        assert_eq!(final_fact.normalized_date.as_deref(), Some("2015-12-12"));
    }

    #[test]
    fn confirmed_acquittal_does_not_also_emit_a_plain_acquittal() {
        let (facts, _) = facts(
            "Nel 2009 Alberto Stasi fu assolto. Il 6 dicembre 2011 la Corte d'Appello di Milano confermò l'assoluzione.",
        );
        assert!(
            facts.iter().any(|fact| {
                fact.normalized_value.as_deref() == Some("acquittal_confirmed")
                    && fact.normalized_date.as_deref() == Some("2011-12-06")
            }),
            "confirmation fact: {facts:?}"
        );
        assert!(
            !facts.iter().any(|fact| {
                fact.normalized_value.as_deref() == Some("acquitted")
                    && fact.normalized_date.as_deref() == Some("2011-12-06")
            }),
            "the generic marker duplicated the confirmation: {facts:?}"
        );
    }

    #[test]
    fn final_outcome_is_attributed_to_the_recent_person() {
        let (facts, _) = facts(
            "Nel 2019 Alberto Stasi fu condannato a sedici anni. Nel 2020 questa condanna diventa definitiva.",
        );
        let final_fact = facts
            .iter()
            .find(|fact| fact.predicate == "legal_finality")
            .unwrap_or_else(|| panic!("final fact: {facts:?}"));
        assert_eq!(final_fact.normalized_value.as_deref(), Some("final"));
        assert_eq!(final_fact.normalized_date.as_deref(), Some("2020"));
    }

    #[test]
    fn agentless_outcome_uses_the_only_recent_person() {
        let (facts, ids) = facts(
            "Nel 2009 Alberto Stasi fu assolto. Nel 2014 il giudice pronunciò una prima condanna.",
        );
        let person = ids
            .iter()
            .find(|id| id.contains("Alberto Stasi"))
            .expect("person id");
        let convicted = facts
            .iter()
            .find(|fact| fact.normalized_value.as_deref() == Some("convicted"))
            .unwrap_or_else(|| panic!("convicted fact: {facts:?}"));
        assert_eq!(&convicted.subject_entity_id, person);
        assert_eq!(convicted.normalized_date.as_deref(), Some("2014"));
    }

    #[test]
    fn two_recent_people_disable_the_agentless_fallback() {
        let (facts, _) = facts(
            "Alberto Stasi e Andrea Sempio furono assolti nel 2009. Nel 2014 il giudice pronunciò una condanna.",
        );
        assert!(!facts.iter().any(|fact| {
            fact.normalized_value.as_deref() == Some("convicted")
                && fact.normalized_date.as_deref() == Some("2014")
        }));
    }

    #[test]
    fn negation_never_flips_into_the_opposite_claim() {
        let (facts, _) = facts("Alberto Stasi non fu assolto il 17 dicembre 2009.");
        let fact = facts
            .iter()
            .find(|fact| fact.normalized_value.as_deref() == Some("acquitted"))
            .expect("negated acquittal is still evidence");
        assert_eq!(fact.status, "negated");
    }

    #[test]
    fn under_investigation_and_denial_stay_distinct() {
        let (facts, _) =
            facts("Andrea Sempio è indagato nel distinto procedimento e respinge le accuse.");
        let statuses: Vec<(&str, &str)> = facts
            .iter()
            .filter(|fact| fact.predicate == "investigation_status")
            .filter_map(|fact| Some((fact.normalized_value.as_deref()?, fact.status.as_str())))
            .collect();
        assert!(statuses.contains(&("under_investigation", "asserted")));
        assert!(statuses.contains(&("denies_accusations", "asserted")));
    }

    #[test]
    fn a_language_without_a_legal_pack_degrades_instead_of_borrowing_markers() {
        let text = "Il 17 dicembre 2009 Alberto Stasi fu assolto.";
        let analysis = visualner::analyze(
            text,
            &ExtractOptions {
                language: "de".to_string(),
                entity_count: 100,
            },
        )
        .expect("analysis");
        let ids: Vec<String> = analysis
            .entities
            .iter()
            .enumerate()
            .map(|(index, entity)| format!("e{}:{}", index, entity.text))
            .collect();
        let facts = extract(
            text,
            "de",
            &analysis.entities,
            &ids,
            &analysis.temporal_facts,
            &analysis.numeric_facts,
        );
        assert!(
            facts.is_empty(),
            "German has no legal pack, so Italian markers must not fire: {facts:?}"
        );
    }

    #[test]
    fn a_date_backed_fact_requires_the_date_in_the_same_sentence() {
        let (facts, _) =
            facts("Alberto Stasi fu assolto. Il 17 dicembre 2009 arrivò la decisione.");
        assert!(
            !facts
                .iter()
                .any(|fact| fact.normalized_value.as_deref() == Some("acquitted")),
            "a neighbouring sentence cannot supply the date: {facts:?}"
        );
    }
}
