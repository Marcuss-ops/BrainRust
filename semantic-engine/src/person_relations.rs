//! Clause-local person relations from explicit English/Italian markers.

use std::collections::BTreeMap;

use semantic_core::{PersonRelation, RelationStatus, SentenceSpan};
use serde::Deserialize;
use visualner::VisualEntity;

const RELATIONS: &str = include_str!("../../lexicon/family_relations.json");

#[derive(Deserialize)]
struct RelationFile {
    languages: BTreeMap<String, Vec<RelationEntry>>,
}

#[derive(Deserialize)]
struct RelationEntry {
    surface: String,
    kind: String,
    direction: String,
}

pub fn extract(
    source: &str,
    language: &str,
    sentences: &[SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
) -> Vec<PersonRelation> {
    if entities.len() != entity_ids.len() {
        return Vec::new();
    }
    let language = language.split('-').next().unwrap_or(language);
    let Ok(file) = serde_json::from_str::<RelationFile>(RELATIONS) else {
        return Vec::new();
    };
    let Some(markers) = file.languages.get(language) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        for marker in markers {
            for (local_start, local_end) in spans(text, &marker.surface) {
                let left_end = sentence.start_byte + local_start;
                let right_start = sentence.start_byte + local_end;
                let left = nearest_person(entities, entity_ids, sentence, left_end, true);
                let right = nearest_person(entities, entity_ids, sentence, right_start, false);
                let (Some(left), Some(right)) = (left, right) else {
                    continue;
                };
                if left.0 == right.0 || left.1 > 80 || right.1 > 80 {
                    continue;
                }
                let (from, to) = if marker.direction == "right_to_left" {
                    (right.0, left.0)
                } else {
                    (left.0, right.0)
                };
                let clause_start =
                    semantic_core::relations::clause_start(text.get(..local_start).unwrap_or(""));
                let clause_end = semantic_core::lexicon::lexicon()
                    .relation
                    .clause_separators
                    .iter()
                    .filter_map(|separator| {
                        text.get(local_end..)?
                            .find(separator)
                            .map(|offset| local_end + offset)
                    })
                    .min()
                    .unwrap_or(text.len());
                let clause = text.get(clause_start..clause_end).unwrap_or(text);
                let status = match semantic_core::relations::clause_status(clause, &[language]) {
                    RelationStatus::Asserted => "asserted",
                    RelationStatus::Negated => "negated",
                    RelationStatus::Uncertain => "uncertain",
                };
                output.push(PersonRelation {
                    from_id: entity_ids[from].clone(),
                    to_id: entity_ids[to].clone(),
                    kind: marker.kind.clone(),
                    status: status.to_string(),
                    start_byte: left_end,
                    end_byte: right_start,
                    sentence_index,
                    evidence: text.to_string(),
                });
            }
        }
    }
    output.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then(a.kind.cmp(&b.kind))
            .then(a.from_id.cmp(&b.from_id))
    });
    output.dedup_by(|a, b| {
        a.start_byte == b.start_byte
            && a.kind == b.kind
            && a.from_id == b.from_id
            && a.to_id == b.to_id
    });
    output
}

fn nearest_person(
    entities: &[VisualEntity],
    ids: &[String],
    sentence: &SentenceSpan,
    boundary: usize,
    before: bool,
) -> Option<(usize, usize)> {
    let mut candidates: Vec<(usize, usize)> = entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| {
            entity.r#type == "PERSON"
                && entity.start >= sentence.start_byte
                && entity.end <= sentence.end_byte
                && (if before {
                    entity.end <= boundary
                } else {
                    entity.start >= boundary
                })
        })
        .filter_map(|(index, entity)| {
            ids.get(index)?;
            let distance = if before {
                boundary.saturating_sub(entity.end)
            } else {
                entity.start.saturating_sub(boundary)
            };
            Some((index, distance))
        })
        .collect();
    candidates.sort_by_key(|(index, distance)| (*distance, entities[*index].start));
    if candidates
        .get(1)
        .is_some_and(|(_, next)| *next == candidates[0].1)
    {
        return None;
    }
    candidates.first().copied()
}

fn spans(text: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
    boundaries.push(text.len());
    let needle_chars = needle.chars().count();
    let folded_needle = needle.to_lowercase();
    (0..boundaries.len().saturating_sub(needle_chars))
        .filter_map(|index| {
            let start = boundaries[index];
            let end = boundaries[index + needle_chars];
            if text.get(start..end)?.to_lowercase() != folded_needle {
                return None;
            }
            let before = text.get(..start)?.chars().next_back();
            let after = text.get(end..)?.chars().next();
            let word = |ch: Option<char>| ch.is_some_and(char::is_alphanumeric);
            (!word(before) && !word(after)).then_some((start, end))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(source: &str, text: &str) -> VisualEntity {
        let start = source.find(text).unwrap();
        VisualEntity {
            text: text.into(),
            r#type: "PERSON".into(),
            score: 1.0,
            start,
            end: start + text.len(),
            evidence: text.into(),
        }
    }

    #[test]
    fn emits_directional_parent_relation_only_between_grounded_people() {
        let source = "Maya Chen is the daughter of John Chen.";
        let people = vec![person(source, "Maya Chen"), person(source, "John Chen")];
        let facts = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &people,
            &["maya".into(), "john".into()],
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(
            (&*facts[0].from_id, &*facts[0].to_id, &*facts[0].kind),
            ("maya", "john", "child_of")
        );
        assert_eq!(facts[0].status, "asserted");
        assert_eq!(facts[0].evidence, source);
    }

    #[test]
    fn preserves_negation_and_requires_both_people() {
        let source = "Maya Chen is not the daughter of John Chen.";
        let people = vec![person(source, "Maya Chen"), person(source, "John Chen")];
        let facts = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &people,
            &["maya".into(), "john".into()],
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].status, "negated");
        let single = extract(
            source,
            "en",
            &semantic_core::split_sentences(source, "en", None),
            &people[..1],
            &["maya".into()],
        );
        assert!(single.is_empty());
    }

    #[test]
    fn localized_marker_keeps_utf8_byte_span_and_direction() {
        let source = "John Chen est le Père de Maya Chen.";
        let people = vec![person(source, "John Chen"), person(source, "Maya Chen")];
        let facts = extract(
            source,
            "fr",
            &semantic_core::split_sentences(source, "fr", None),
            &people,
            &["john".into(), "maya".into()],
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].from_id, "john");
        assert_eq!(facts[0].to_id, "maya");
        assert_eq!(facts[0].kind, "parent_of");
        assert_eq!(&source[facts[0].start_byte..facts[0].end_byte], "Père de");
    }

    #[test]
    fn oblique_polish_and_russian_family_markers_resolve_directionally() {
        let examples = [
            (
                "pl",
                "Jan Kowalski jest ojcem Marii Nowak.",
                "Jan Kowalski",
                "Marii Nowak",
                "ojcem",
            ),
            (
                "ru",
                "Иван Петров является отцом Марии Ивановой.",
                "Иван Петров",
                "Марии Ивановой",
                "отцом",
            ),
        ];

        for (language, source, left_name, right_name, marker) in examples {
            let people = vec![person(source, left_name), person(source, right_name)];
            let relations = extract(
                source,
                language,
                &semantic_core::split_sentences(source, language, None),
                &people,
                &["parent".into(), "child".into()],
            );
            assert_eq!(relations.len(), 1, "{language}: {relations:?}");
            assert_eq!(relations[0].from_id, "parent");
            assert_eq!(relations[0].to_id, "child");
            assert_eq!(relations[0].kind, "parent_of");
            assert_eq!(
                &source[relations[0].start_byte..relations[0].end_byte],
                marker
            );
        }
    }
}
