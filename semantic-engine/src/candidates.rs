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
        .map(|entity| semantic_core::bullets::BulletSignal {
            start_byte: entity.start,
            end_byte: entity.end,
            kind: if is_visual_subject(entity) {
                semantic_core::bullets::SignalKind::Visual
            } else {
                semantic_core::bullets::SignalKind::Measurement
            },
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
/// window, so bullets and segments rank with one weight table. The best
/// non-overlapping windows win in stable order; a scene without signals keeps
/// an empty collection.
pub(crate) fn segment_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<ImportantSegment> {
    const MIN_LEN: usize = 2;
    const MAX_LEN: usize = 5;
    const MAX_SEGMENTS: usize = 3;

    let signals: Vec<semantic_core::bullets::BulletSignal> = entities
        .iter()
        .map(|entity| semantic_core::bullets::BulletSignal {
            start_byte: entity.start,
            end_byte: entity.end,
            kind: if is_visual_subject(entity) {
                semantic_core::bullets::SignalKind::Visual
            } else {
                semantic_core::bullets::SignalKind::Measurement
            },
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
    let anchored = |index: usize| salience.get(index).is_some_and(|(score, _, _)| *score > 0.0);

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

    let mut selected: Vec<(f32, usize, usize)> = Vec::new();
    for (score, start, end) in windows {
        if selected.len() >= MAX_SEGMENTS {
            break;
        }
        let overlaps = selected
            .iter()
            .any(|(_, other_start, other_end)| start <= *other_end && *other_start <= end);
        if !overlaps {
            selected.push((score, start, end));
        }
    }
    selected.sort_by_key(|(_, start, _)| *start);

    selected
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
    components.extend(entities.iter().map(|entity| QueryComponent {
        kind: "entity".to_string(),
        component_type: entity.r#type.clone(),
        text: entity.text.clone(),
        start_byte: entity.start,
        end_byte: entity.end,
    }));
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
            if !bucket
                .iter()
                .any(|other| other.start_byte == component.start_byte && other.end_byte == component.end_byte)
            {
                bucket.push(component);
            }
        }
    }

    by_sentence
        .into_iter()
        .enumerate()
        .filter_map(|(sentence_index, mut components)| {
            if components.len() < 2 {
                return None;
            }
            components.sort_by_key(|component| (component.start_byte, component.end_byte));
            let start_byte = components.first()?.start_byte;
            let end_byte = components.iter().map(|component| component.end_byte).max()?;
            let query = source.get(start_byte..end_byte)?;
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

/// Projects the already-built entity candidates into the keyword surface:
/// keywords must be usable as image queries, so they are a curated subset of
/// the entities, not a second extraction. Duplicates are removed per scene so
/// the same name does not spend several planner keyword slots.
pub(crate) fn important_word_candidates(entity_candidates: &[Candidate]) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    entity_candidates
        .iter()
        .filter(|candidate| {
            !matches!(
                candidate.kind.as_str(),
                "DATE" | "NUMBER" | "PERCENT" | "MONEY"
            )
        })
        .filter(|candidate| {
            candidate.kind != "VISUAL_CONCEPT" || candidate.text.split_whitespace().count() >= 2
        })
        .filter(|candidate| candidate.text.split_whitespace().count() <= 5)
        .filter(|candidate| seen.insert(candidate.text.to_lowercase()))
        .cloned()
        .collect()
}
