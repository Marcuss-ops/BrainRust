use super::*;

pub(crate) fn entity_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<Candidate> {
    entities
        .iter()
        .enumerate()
        .map(|(index, entity)| Candidate {
            id: identities.id_of(index),
            text: entity.text.clone(),
            kind: entity.r#type.clone(),
            start_byte: entity.start,
            end_byte: entity.end,
            sentence_index: sentence_index_at(sentences, entity.start),
            evidence: entity.evidence.clone(),
            score: entity.score,
            provenance: "visualner".to_string(),
        })
        .filter(|candidate| {
            check_span(
                source,
                candidate.start_byte,
                candidate.end_byte,
                &candidate.evidence,
            ) == Grounding::Grounded
        })
        .collect()
}

/// Generates the manifest's bullet candidates with the shared algorithm over
/// the FINAL grounded entity list, so facility mentions added by this engine
/// participate in the same scoring as tokenizer-extracted entities.
pub(crate) fn bullet_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<Candidate> {
    let signals: Vec<semantic_core::bullets::BulletSignal> = entities
        .iter()
        .filter_map(|entity| {
            let kind = semantic_core::bullets::signal_kind_for_entity_type(&entity.r#type)?;
            Some(semantic_core::bullets::BulletSignal {
                start_byte: entity.start,
                end_byte: entity.end,
                kind,
            })
        })
        .collect();
    semantic_core::bullets::generate_with_sentences(source, sentences, &signals)
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| Candidate {
            id: format!("bullet_{:02}", index + 1),
            text: candidate.text.clone(),
            kind: "SENTENCE".to_string(),
            start_byte: candidate.start_byte,
            end_byte: candidate.end_byte,
            sentence_index: candidate.sentence_index,
            evidence: candidate.text,
            score: candidate.score,
            provenance: "semantic_core_bullets".to_string(),
        })
        .collect()
}

/// Index of the sentence that contains `offset`. Spans are sorted, so a
/// binary search replaces a full scan; the linear fallback only runs for an
/// offset outside every span (an invalid caller, never an extracted mention).
pub(crate) fn sentence_index_at(sentences: &[semantic_core::SentenceSpan], offset: usize) -> usize {
    let index = sentences.partition_point(|sentence| sentence.start_byte <= offset);
    if let Some(sentence) = index
        .checked_sub(1)
        .and_then(|previous| sentences.get(previous))
    {
        if offset < sentence.end_byte {
            return index - 1;
        }
    }
    sentences
        .iter()
        .position(|sentence| offset >= sentence.start_byte && offset < sentence.end_byte)
        .unwrap_or(0)
}

pub(crate) fn phrase_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    min_words: usize,
    max_words: usize,
) -> Vec<Candidate> {
    sentences
        .iter()
        .enumerate()
        .filter_map(|(sentence_index, sentence)| {
            let evidence = source.get(sentence.start_byte..sentence.end_byte)?;
            let word_count = evidence.split_whitespace().count();
            if word_count < min_words || word_count >= max_words {
                return None;
            }
            (check_span(source, sentence.start_byte, sentence.end_byte, evidence)
                == Grounding::Grounded)
                .then(|| Candidate {
                    id: format!("sentence_{:04}", sentence_index + 1),
                    text: evidence.to_string(),
                    kind: "PHRASE".to_string(),
                    start_byte: sentence.start_byte,
                    end_byte: sentence.end_byte,
                    sentence_index,
                    evidence: evidence.to_string(),
                    score: (0.5 + (word_count.min(30) as f32 / 60.0)).min(1.0),
                    provenance: "semantic_core_sentence_segmenter".to_string(),
                })
        })
        .collect()
}

/// Grounded multi-sentence passages for planners that treat a passage as one
/// editorial unit. A window is 2–5 consecutive sentences cut at the canonical
/// segmenter's boundaries (linguistic boundaries, never mid-sentence), and it
/// is anchored when its first and its last sentence each carry at least one
/// grounded signal and every sentence in it is substantive. The score is the
/// shared sentence salience (`semantic_core::bullets`) averaged over the
/// window, so bullets and segments rank with one weight table. Every
/// qualifying window is retained in source order; downstream planners choose
/// what to display. A scene without signals keeps an empty collection.
pub(crate) fn segment_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<ImportantSegment> {
    const MIN_LEN: usize = 2;
    const MAX_LEN: usize = 5;

    let signals: Vec<semantic_core::bullets::BulletSignal> = entities
        .iter()
        .filter_map(|entity| {
            let kind = semantic_core::bullets::signal_kind_for_entity_type(&entity.r#type)?;
            Some(semantic_core::bullets::BulletSignal {
                start_byte: entity.start,
                end_byte: entity.end,
                kind,
            })
        })
        .collect();
    // Per-sentence salience and signal counts come from the shared bullets
    // algorithm: one signal-counting rule and one weight table for both.
    let scored = semantic_core::bullets::generate_with_sentences(source, sentences, &signals);
    let mut salience: Vec<(f32, usize, usize)> = vec![(0.0, 0, 0); sentences.len()];
    for candidate in &scored {
        if let Some(slot) = salience.get_mut(candidate.sentence_index) {
            *slot = (
                candidate.score,
                candidate.visual_mentions,
                candidate.measurement_mentions,
            );
        }
    }

    let substantive = |index: usize| {
        sentences
            .get(index)
            .and_then(|sentence| source.get(sentence.start_byte..sentence.end_byte))
            .is_some_and(semantic_core::bullets::is_substantive)
    };
    let anchored = |index: usize| {
        salience
            .get(index)
            .is_some_and(|(score, _, _)| *score > 0.0)
    };

    let mut windows: Vec<(f32, usize, usize)> = Vec::new(); // (score, start, end inclusive)
    for start in 0..sentences.len() {
        for len in MIN_LEN..=MAX_LEN {
            let end = start + len - 1;
            if end >= sentences.len() {
                break;
            }
            if !anchored(start) || !anchored(end) || !(start..=end).all(substantive) {
                continue;
            }
            let total: f32 = (start..=end).map(|index| salience[index].0).sum();
            windows.push((total / len as f32, start, end));
        }
    }
    windows.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| (b.2 - b.1).cmp(&(a.2 - a.1)))
            .then_with(|| a.1.cmp(&b.1))
    });

    // Keep every complete, grounded window. Overlay selection belongs to
    // PipelineGen; a BrainRust candidate cap would silently discard valid
    // passages before the planner can compare them.
    windows.sort_by_key(|(_, start, end)| (*start, *end));

    windows
        .into_iter()
        .enumerate()
        .filter_map(|(index, (score, start, end))| {
            let first = sentences.get(start)?;
            let last = sentences.get(end)?;
            let evidence = source.get(first.start_byte..last.end_byte)?;
            let visual: usize = (start..=end).map(|i| salience[i].1).sum();
            let measurement: usize = (start..=end).map(|i| salience[i].2).sum();
            (check_span(source, first.start_byte, last.end_byte, evidence) == Grounding::Grounded)
                .then(|| ImportantSegment {
                    id: format!("segment_{:02}", index + 1),
                    kind: "important_segment".to_string(),
                    sentence_index_start: start,
                    sentence_index_end: end,
                    evidence: evidence.to_string(),
                    start_byte: first.start_byte,
                    end_byte: last.end_byte,
                    score,
                    visual_mentions: visual,
                    measurement_mentions: measurement,
                })
        })
        .collect()
}

/// Composed image queries: the verbatim source slice that spans two or more
/// grounded components (entities, measurements, dates) stated together in one
/// sentence, with every component's own span as provenance. The literal-
/// substring rule is kept: the query is a source slice, never a synthetic
/// join of surfaces the source never wrote. One query per sentence, in source
/// order; a sentence without two grounded components stays out.
pub(crate) fn composed_query_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    numeric_facts: &[semantic_core::NumericFact],
    temporal_facts: &[TemporalFact],
) -> Vec<ComposedImageQuery> {
    let mut components: Vec<QueryComponent> = Vec::new();
    components.extend(
        entities
            .iter()
            .filter(|entity| is_image_component_type(&entity.r#type))
            .map(|entity| QueryComponent {
                kind: "entity".to_string(),
                component_type: entity.r#type.clone(),
                text: entity.text.clone(),
                start_byte: entity.start,
                end_byte: entity.end,
            }),
    );
    components.extend(numeric_facts.iter().map(|fact| QueryComponent {
        kind: "measurement".to_string(),
        component_type: fact.kind.clone(),
        text: fact.surface.clone(),
        start_byte: fact.start_byte,
        end_byte: fact.end_byte,
    }));
    components.extend(temporal_facts.iter().map(|fact| QueryComponent {
        kind: "date".to_string(),
        component_type: fact.kind.clone(),
        text: fact.surface.clone(),
        start_byte: fact.start_byte,
        end_byte: fact.end_byte,
    }));
    // One collection per sentence; identical spans are one component (an
    // entity and its measurement projection share the surface).
    let mut by_sentence: Vec<Vec<QueryComponent>> = vec![Vec::new(); sentences.len()];
    for component in components {
        let index = sentence_index_at(sentences, component.start_byte);
        if let Some(bucket) = by_sentence.get_mut(index) {
            if !bucket.iter().any(|other| {
                other.start_byte == component.start_byte && other.end_byte == component.end_byte
            }) {
                bucket.push(component);
            }
        }
    }

    by_sentence
        .into_iter()
        .enumerate()
        .filter_map(|(sentence_index, mut components)| {
            if components.len() < 2
                || !components
                    .iter()
                    .any(|component| is_visual_query_entity(&component.component_type))
            {
                return None;
            }
            components.sort_by_key(|component| (component.start_byte, component.end_byte));
            let start_byte = components.first()?.start_byte;
            let end_byte = components
                .iter()
                .map(|component| component.end_byte)
                .max()?;
            let query = source.get(start_byte..end_byte)?;
            if query.split_whitespace().count() > 12 || query.chars().count() > 80 {
                return None;
            }
            (check_span(source, start_byte, end_byte, query) == Grounding::Grounded).then(|| {
                ComposedImageQuery {
                    id: format!("composed_{:03}", sentence_index + 1),
                    kind: "composed_image_query".to_string(),
                    query: query.to_string(),
                    start_byte,
                    end_byte,
                    sentence_index,
                    components,
                }
            })
        })
        .collect()
}

/// Builds non-literal provider queries only when the engine already has an
/// asserted, source-grounded relation between two visual entities. The query
/// joins the exact entity surfaces; sentence evidence and both component
/// spans preserve why the pair belongs together. Location facts can also use
/// their grounded value when the location did not survive entity extraction.
pub(crate) fn contextual_image_query_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
    facts: &[EntityFact],
) -> Vec<ContextualImageQuery> {
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    for fact in facts {
        if fact.status != "asserted" {
            continue;
        }
        let Some(target_id) = fact.target_entity_id.as_deref() else {
            continue;
        };
        let Some(subject_index) = entity_ids.iter().enumerate().find_map(|(index, identity)| {
            let entity = entities.get(index)?;
            (identity == &fact.subject_entity_id
                && entity.start >= fact.evidence_start_byte
                && entity.end <= fact.evidence_end_byte)
                .then_some(index)
        }) else {
            continue;
        };
        let target_index = entity_ids.iter().enumerate().find_map(|(index, identity)| {
            let entity = entities.get(index)?;
            (identity == target_id
                && entity.start >= fact.evidence_start_byte
                && entity.end <= fact.evidence_end_byte)
                .then_some(index)
        });
        let Some(subject) = entities.get(subject_index) else {
            continue;
        };
        let target = target_index.and_then(|index| entities.get(index));
        let target_span = if let Some(target) = target {
            if !is_visual_query_entity(&target.r#type)
                || (fact.predicate == "located_in" && target.r#type != "LOCATION")
            {
                continue;
            }
            (
                target.text.as_str(),
                target.start,
                target.end,
                target.r#type.as_str(),
                if fact.predicate == "located_in" {
                    "location"
                } else {
                    "related_entity"
                },
            )
        } else if fact.predicate == "located_in"
            && source.get(fact.start_byte..fact.end_byte) == Some(fact.value.as_str())
            && fact.start_byte >= fact.evidence_start_byte
            && fact.end_byte <= fact.evidence_end_byte
        {
            (
                fact.value.as_str(),
                fact.start_byte,
                fact.end_byte,
                "LOCATION",
                "location",
            )
        } else {
            continue;
        };
        if subject.start == target_span.1 && subject.end == target_span.2
            || !is_visual_query_entity(&subject.r#type)
            || !seen.insert(relation_query_key(
                fact.sentence_index,
                fact.subject_entity_id.as_str(),
                target_id,
            ))
        {
            continue;
        }
        let Some(sentence) = sentences.get(fact.sentence_index) else {
            continue;
        };
        if fact.evidence_start_byte < sentence.start_byte
            || fact.evidence_end_byte > sentence.end_byte
        {
            continue;
        }
        let Some(evidence) = source.get(fact.evidence_start_byte..fact.evidence_end_byte) else {
            continue;
        };
        if evidence != fact.evidence {
            continue;
        }
        let query = format!("{} {}", subject.text.trim(), target_span.0.trim());
        if query.trim().is_empty() {
            continue;
        }
        output.push(ContextualImageQuery {
            id: format!("contextual_{:03}", output.len() + 1),
            kind: "contextual_image_query".to_string(),
            query,
            evidence: evidence.to_string(),
            evidence_start_byte: fact.evidence_start_byte,
            evidence_end_byte: fact.evidence_end_byte,
            sentence_index: fact.sentence_index,
            components: vec![
                QueryComponent {
                    kind: "entity".to_string(),
                    component_type: subject.r#type.clone(),
                    text: subject.text.clone(),
                    start_byte: subject.start,
                    end_byte: subject.end,
                },
                QueryComponent {
                    kind: target_span.4.to_string(),
                    component_type: target_span.3.to_string(),
                    text: target_span.0.to_string(),
                    start_byte: target_span.1,
                    end_byte: target_span.2,
                },
            ],
        });
    }
    // Co-mention alone does not establish that two entities form a useful
    // image-search subject. Pair queries are emitted only above when a
    // grounded asserted fact links the identities; each identity still gets
    // its own exact/context search ladder in the proposal stage.
    output
}

fn relation_query_key(sentence_index: usize, left: &str, right: &str) -> (usize, String, String) {
    if left <= right {
        (sentence_index, left.to_string(), right.to_string())
    } else {
        (sentence_index, right.to_string(), left.to_string())
    }
}

fn is_visual_query_entity(entity_type: &str) -> bool {
    matches!(
        entity_type,
        "PERSON"
            | "LOCATION"
            | "ORG"
            | "ORGANIZATION"
            | "FACILITY"
            | "BRAND"
            | "PRODUCT"
            | "OBJECT"
            | "WORK"
            | "STRUCTURE"
    )
}

fn is_image_component_type(entity_type: &str) -> bool {
    is_visual_query_entity(entity_type)
        || matches!(entity_type, "DATE" | "NUMBER" | "PERCENT" | "MONEY")
}

/// Extracts complete, explicitly delimited quotations. The delimiters remain
/// part of the candidate so the text and offsets are an exact source slice.
/// Apostrophes and single quotes are deliberately excluded because they are
/// common orthographic characters and cannot safely delimit a quote alone.
pub(crate) fn quote_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
) -> Vec<Candidate> {
    let delimiters = [("“", "”"), ("„", "“"), ("«", "»"), ("\"", "\"")];
    let mut spans = Vec::new();
    for (opening, closing) in delimiters {
        let mut cursor = 0;
        while let Some(relative_start) = source[cursor..].find(opening) {
            let start = cursor + relative_start;
            let content_start = start + opening.len();
            let Some(relative_end) = source[content_start..].find(closing) else {
                break;
            };
            let close_start = content_start + relative_end;
            let end = close_start + closing.len();
            if !source[content_start..close_start].trim().is_empty() {
                spans.push((start, end));
            }
            cursor = end;
        }
    }
    spans.sort_unstable();
    spans.dedup();
    spans
        .into_iter()
        .enumerate()
        .filter_map(|(index, (start, end))| {
            let evidence = source.get(start..end)?;
            (check_span(source, start, end, evidence) == Grounding::Grounded).then(|| Candidate {
                id: format!("quote_{:04}", index + 1),
                text: evidence.to_string(),
                kind: "QUOTE".to_string(),
                start_byte: start,
                end_byte: end,
                sentence_index: sentence_index_at(sentences, start),
                evidence: evidence.to_string(),
                score: 1.0,
                provenance: "semantic-core".to_string(),
            })
        })
        .collect()
}

/// Projects only typed, picturable entities into the provider keyword surface.
/// Generic noun phrases remain available in the phrase inventory; they do not
/// independently become search terms. Duplicates are removed per scene so a
/// repeated mention does not spend several planner keyword slots.
pub(crate) fn important_word_candidates(entity_candidates: &[Candidate]) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    entity_candidates
        .iter()
        .filter(|candidate| is_visual_query_entity(&candidate.kind))
        .filter(|candidate| candidate.text.split_whitespace().count() <= 5)
        .filter(|candidate| seen.insert(candidate.text.to_lowercase()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod contextual_image_query_tests {
    use super::*;

    #[test]
    fn chooses_repeated_entity_mentions_inside_the_supporting_sentence() {
        let source = "Torino first. Aurora Stadium is located in Torino.";
        let evidence_start = source.find("Aurora Stadium").unwrap();
        let evidence_end = source.len();
        let subject_start = evidence_start;
        let subject_end = subject_start + "Aurora Stadium".len();
        let location_start = source.rfind("Torino").unwrap();
        let entities = vec![
            VisualEntity {
                text: "Torino".into(),
                r#type: "LOCATION".into(),
                score: 0.9,
                start: 0,
                end: 6,
                evidence: "Torino".into(),
            },
            VisualEntity {
                text: "Aurora Stadium".into(),
                r#type: "FACILITY".into(),
                score: 0.9,
                start: subject_start,
                end: subject_end,
                evidence: "Aurora Stadium".into(),
            },
            VisualEntity {
                text: "Torino".into(),
                r#type: "LOCATION".into(),
                score: 0.9,
                start: location_start,
                end: location_start + "Torino".len(),
                evidence: "Torino".into(),
            },
        ];
        let ids = vec!["location".into(), "facility".into(), "location".into()];
        let evidence = source[evidence_start..evidence_end].to_string();
        let facts = vec![EntityFact {
            id: "located".into(),
            subject_entity_id: "facility".into(),
            predicate: "located_in".into(),
            value: "Torino".into(),
            value_type: "ENTITY".into(),
            normalized_value: None,
            normalized_date: None,
            target_entity_id: Some("location".into()),
            unit: None,
            operator: None,
            status: "asserted".into(),
            sentence_index: 1,
            evidence,
            start_byte: location_start,
            end_byte: location_start + "Torino".len(),
            evidence_start_byte: evidence_start,
            evidence_end_byte: evidence_end,
            importance: 0.0,
            visual_concept_ids: Vec::new(),
        }];

        let sentences = semantic_core::split_sentences(source, "en", None);
        let queries =
            contextual_image_query_candidates(source, &sentences, &entities, &ids, &facts);
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].query, "Aurora Stadium Torino");
        for component in &queries[0].components {
            assert!(component.start_byte >= evidence_start);
            assert!(component.end_byte <= evidence_end);
            assert_eq!(
                &source[component.start_byte..component.end_byte],
                component.text
            );
        }
    }
}
