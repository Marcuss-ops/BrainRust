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
    entities: &[VisualEntity],
    min_words: usize,
    max_words: usize,
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
    let mut salience = vec![0.0; sentences.len()];
    for bullet in semantic_core::bullets::generate_with_sentences(source, sentences, &signals) {
        if let Some(score) = salience.get_mut(bullet.sentence_index) {
            *score = bullet.score;
        }
    }
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
                    // Keep all complete sentences in the phrase inventory;
                    // rank their editorial value from grounded semantic
                    // signals rather than rewarding length alone.
                    score: salience.get(sentence_index).copied().unwrap_or_default(),
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
/// window, so bullets and segments rank with one weight table. A bounded
/// greedy interval selection keeps at most three non-overlapping passages per
/// scene; every selected passage remains contiguous and source-grounded.
/// A scene without signals keeps an empty collection.
pub(crate) fn segment_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    facts: &[semantic_core::EntityFact],
) -> Vec<ImportantSegment> {
    const MIN_LEN: usize = 2;
    const MAX_LEN: usize = 5;
    const MIN_EDITORIAL_SCORE: f32 = 0.4;

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

    // Dated, asserted events are editorial anchors. Reward a contiguous
    // passage that preserves several events together, so a timeline is not
    // fragmented into individually high scoring sentences that omit its
    // beginning or conclusion.
    let event_counts: Vec<usize> = sentences
        .iter()
        .enumerate()
        .map(|(index, _)| {
            facts
                .iter()
                .filter(|fact| {
                    fact.status == "asserted"
                        && fact.normalized_date.is_some()
                        && fact.sentence_index == index
                })
                .count()
        })
        .collect();
    let total_events: usize = event_counts.iter().sum();

    let mut windows: Vec<(f32, usize, usize)> = Vec::new(); // (score, start, end inclusive)
    for start in 0..sentences.len() {
        for len in MIN_LEN..=MAX_LEN {
            let end = start + len - 1;
            if end >= sentences.len() {
                break;
            }
            if !(start..=end).any(anchored) || !(start..=end).all(substantive) {
                continue;
            }
            let total: f32 = (start..=end).map(|index| salience[index].0).sum();
            let editorial_score = total / len as f32;
            let covered_events: usize = (start..=end).map(|index| event_counts[index]).sum();
            let sequence_bonus = if total_events > 1 {
                0.32 * (covered_events as f32 / total_events as f32)
            } else {
                0.0
            };
            let score = (editorial_score + sequence_bonus).min(1.0);
            if score >= MIN_EDITORIAL_SCORE {
                windows.push((score, start, end));
            }
        }
    }
    windows.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| (b.2 - b.1).cmp(&(a.2 - a.1)))
            .then_with(|| a.1.cmp(&b.1))
    });
    const MAX_SEGMENTS_PER_SCENE: usize = 3;
    let mut selected: Vec<(f32, usize, usize)> = Vec::new();
    for window in windows {
        if selected.len() == MAX_SEGMENTS_PER_SCENE {
            break;
        }
        let (_, start, end) = window;
        if selected
            .iter()
            .any(|(_, prior_start, prior_end)| start <= *prior_end && *prior_start <= end)
        {
            continue;
        }
        selected.push(window);
    }
    selected.sort_by_key(|(_, start, end)| (*start, *end));

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

/// Composed image queries are a small source-grounded shortlist. They need two
/// picturable entities in the same sentence, stay short enough for an image
/// provider, and keep the exact literal span as provenance.
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

    let mut candidates: Vec<(usize, usize, ComposedImageQuery)> = by_sentence
        .into_iter()
        .enumerate()
        .filter_map(|(sentence_index, mut components)| {
            if components.len() < 2
                || components
                    .iter()
                    .filter(|component| is_visual_query_entity(&component.component_type))
                    .count()
                    < 2
            {
                return None;
            }
            components.sort_by_key(|component| {
                (component.start_byte, std::cmp::Reverse(component.end_byte))
            });
            let mut non_overlapping = Vec::with_capacity(components.len());
            for component in components {
                if non_overlapping.iter().any(|prior: &QueryComponent| {
                    component.start_byte < prior.end_byte && prior.start_byte < component.end_byte
                }) {
                    continue;
                }
                non_overlapping.push(component);
            }
            components = non_overlapping;
            if components
                .iter()
                .filter(|component| is_visual_query_entity(&component.component_type))
                .count()
                < 2
                || !components
                    .iter()
                    .filter(|component| is_visual_query_entity(&component.component_type))
                    .enumerate()
                    .any(|(index, left)| {
                        components
                            .iter()
                            .filter(|component| is_visual_query_entity(&component.component_type))
                            .skip(index + 1)
                            .any(|right| {
                                image_pair_quality(&left.component_type, &right.component_type) >= 3
                            })
                    })
            {
                return None;
            }
            let start_byte = components.first()?.start_byte;
            let end_byte = components
                .iter()
                .map(|component| component.end_byte)
                .max()?;
            let query = source.get(start_byte..end_byte)?;
            if query.split_whitespace().count() > 9 || query.chars().count() > 68 {
                return None;
            }
            (check_span(source, start_byte, end_byte, query) == Grounding::Grounded).then(|| {
                let visual_count = components
                    .iter()
                    .filter(|component| is_visual_query_entity(&component.component_type))
                    .count();
                let named_count = components
                    .iter()
                    .filter(|component| {
                        matches!(
                            component.component_type.as_str(),
                            "PERSON" | "LOCATION" | "ORG" | "ORGANIZATION" | "FACILITY"
                        )
                    })
                    .count();
                let quality = named_count * 10 + visual_count;
                (
                    quality,
                    sentence_index,
                    ComposedImageQuery {
                        id: format!("composed_{:03}", sentence_index + 1),
                        kind: "composed_image_query".to_string(),
                        query: query.to_string(),
                        start_byte,
                        end_byte,
                        sentence_index,
                        components,
                    },
                )
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.2.query.len().cmp(&right.2.query.len()))
            .then_with(|| left.1.cmp(&right.1))
    });
    candidates.truncate(3);
    candidates.sort_by_key(|(_, sentence_index, _)| *sentence_index);
    candidates
        .into_iter()
        .enumerate()
        .map(|(index, (_, _, mut candidate))| {
            candidate.id = format!("composed_{:03}", index + 1);
            candidate
        })
        .collect()
}

/// Builds compact provider queries from asserted relations first, then from
/// source-grounded same-sentence co-mentions. A co-mention is explicitly a
/// retrieval cue, not an assertion that the entities are related. Queries are
/// bounded per scene and every component retains its exact source span.
pub(crate) fn contextual_image_query_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
    facts: &[EntityFact],
    co_mentions: &[semantic_core::CoMention],
) -> Vec<ContextualImageQuery> {
    let max_contextual_queries =
        crate::consolidate::image_query_policy().max_contextual_queries_per_scene;
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    let explicitly_located_entities: HashSet<String> = facts
        .iter()
        .filter(|fact| fact.status == "asserted" && fact.predicate == "located_in")
        .flat_map(|fact| {
            std::iter::once(fact.subject_entity_id.clone())
                .chain(fact.target_entity_id.iter().cloned())
        })
        .collect();
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
            // A place already contained in an institution's source name is
            // not useful as a second query component ("Comune di Torino
            // Torino"). Keep the entity span and relation evidence intact,
            // but do not emit a redundant image query.
            || (subject.start < target_span.2 && target_span.1 < subject.end)
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
        if query.trim().is_empty()
            || query.split_whitespace().count() > 7
            || query.chars().count() > 64
        {
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
    let mut co_mention_candidates: Vec<(usize, usize, String, ContextualImageQuery)> = Vec::new();
    for mention in co_mentions {
        // Prefer the explicit location relation already extracted for an
        // entity. A second, merely co-mentioned person/place pair in another
        // clause of the same scene is too easy to misread as a location link.
        if explicitly_located_entities.contains(&mention.left_id)
            || explicitly_located_entities.contains(&mention.right_id)
        {
            continue;
        }
        let Some(sentence) = sentences.get(mention.sentence_index) else {
            continue;
        };
        let Some(evidence) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        if evidence != mention.evidence {
            continue;
        }
        let left_indices: Vec<usize> = entity_ids
            .iter()
            .enumerate()
            .filter_map(|(index, identity)| {
                let entity = entities.get(index)?;
                (identity == &mention.left_id
                    && entity.start >= sentence.start_byte
                    && entity.end <= sentence.end_byte
                    && is_visual_query_entity(&entity.r#type)
                    && source.get(entity.start..entity.end) == Some(entity.text.as_str()))
                .then_some(index)
            })
            .collect();
        let right_indices: Vec<usize> = entity_ids
            .iter()
            .enumerate()
            .filter_map(|(index, identity)| {
                let entity = entities.get(index)?;
                (identity == &mention.right_id
                    && entity.start >= sentence.start_byte
                    && entity.end <= sentence.end_byte
                    && is_visual_query_entity(&entity.r#type)
                    && source.get(entity.start..entity.end) == Some(entity.text.as_str()))
                .then_some(index)
            })
            .collect();
        let Some((left, right)) = left_indices
            .iter()
            .flat_map(|left| {
                right_indices.iter().filter_map(move |right| {
                    let a = entities.get(*left)?;
                    let b = entities.get(*right)?;
                    let distinct = a.start != b.start || a.end != b.end;
                    let disjoint = a.end <= b.start || b.end <= a.start;
                    (distinct && disjoint).then_some((*left, *right))
                })
            })
            .min_by_key(|(left, right)| {
                let a = &entities[*left];
                let b = &entities[*right];
                a.start.abs_diff(b.start)
            })
        else {
            continue;
        };
        let a = &entities[left];
        let b = &entities[right];
        let (first, second) = contextual_entity_order(a, b);
        let query = format!("{} {}", first.text.trim(), second.text.trim());
        if query.split_whitespace().count() > 7 || query.chars().count() > 64 {
            continue;
        }
        let key = relation_query_key(mention.sentence_index, &mention.left_id, &mention.right_id);
        if !seen.insert(key) {
            continue;
        }
        let pair_quality = image_pair_quality(&first.r#type, &second.r#type);
        if pair_quality < 3 {
            continue;
        }
        co_mention_candidates.push((
            pair_quality,
            first.start.abs_diff(second.start),
            query.clone(),
            ContextualImageQuery {
                id: String::new(),
                kind: "contextual_image_query".to_string(),
                query,
                evidence: evidence.to_string(),
                evidence_start_byte: sentence.start_byte,
                evidence_end_byte: sentence.end_byte,
                sentence_index: mention.sentence_index,
                components: vec![first, second]
                    .into_iter()
                    .map(|entity| QueryComponent {
                        kind: "co_mentioned_entity".to_string(),
                        component_type: entity.r#type.clone(),
                        text: entity.text.clone(),
                        start_byte: entity.start,
                        end_byte: entity.end,
                    })
                    .collect(),
            },
        ));
    }
    co_mention_candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.sentence_index.cmp(&right.3.sentence_index))
    });
    for (_, _, _, candidate) in co_mention_candidates {
        if output.len() >= max_contextual_queries {
            break;
        }
        if output
            .iter()
            .any(|existing| existing.query.eq_ignore_ascii_case(&candidate.query))
        {
            continue;
        }
        output.push(candidate);
    }
    output.truncate(max_contextual_queries);
    output.sort_by_key(|candidate| candidate.sentence_index);
    for (index, candidate) in output.iter_mut().enumerate() {
        candidate.id = format!("contextual_{:03}", index + 1);
    }
    output
}

fn image_pair_quality(left_type: &str, right_type: &str) -> usize {
    let has_object = left_type == "OBJECT" || right_type == "OBJECT";
    let has_person = left_type == "PERSON" || right_type == "PERSON";
    let has_location = left_type == "LOCATION" || right_type == "LOCATION";
    let has_org =
        matches!(left_type, "ORG" | "ORGANIZATION") || matches!(right_type, "ORG" | "ORGANIZATION");
    if (left_type == "OBJECT" && right_type == "OBJECT")
        || (has_person && has_location)
        || (has_object && has_location)
    {
        5
    } else if (has_person && has_object) || (has_org && has_location) {
        4
    } else if has_person && has_org {
        // Co-mention alone is too weak to make a useful joint image query
        // for a person and an organization. Asserted relation facts can still
        // produce that pair through the fact-grounded path above.
        2
    } else if has_person && left_type == "PERSON" && right_type == "PERSON" {
        2
    } else {
        0
    }
}

fn contextual_entity_order<'a>(
    left: &'a VisualEntity,
    right: &'a VisualEntity,
) -> (&'a VisualEntity, &'a VisualEntity) {
    let rank = |entity_type: &str| match entity_type {
        "PERSON" => 0,
        "OBJECT" => 1,
        "ORG" | "ORGANIZATION" => 2,
        "FACILITY" => 3,
        "LOCATION" => 4,
        _ => 5,
    };
    if rank(&left.r#type) < rank(&right.r#type)
        || (rank(&left.r#type) == rank(&right.r#type) && left.start <= right.start)
    {
        (left, right)
    } else {
        (right, left)
    }
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
            contextual_image_query_candidates(source, &sentences, &entities, &ids, &facts, &[]);
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
