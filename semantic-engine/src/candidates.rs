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

/// Builds compact, source-grounded provider queries from the visual roles in
/// each clause and from explicit anaphoric references between scene sentences.
/// Queries stay within evidence spans, are budgeted per scene, and never
/// promote a retrieval composition into a verified fact.
/// The two consumers of contextual queries require different contracts.
/// `SceneManifest::contextual_image_queries` is contract-pure: only queries
/// composed from an asserted subject-to-location fact (with its exact
/// sentence evidence) may appear there. The visual-context families (clause
/// slots, linked clauses, inherited scene context) are retrieval
/// compositions, never relation assertions, so they may only seed
/// `contextual_search` production candidates alongside the grounded queries.
pub(crate) struct ContextualQueries {
    /// Fact-grounded relation queries: the only form the manifest contract
    /// allows. Numbered `contextual_NNN` in sentence order.
    pub manifest_queries: Vec<ContextualImageQuery>,
    /// Grounded and visual-context queries together, in sentence order, as
    /// production-candidate seeds. Numbered `contextual_NNN` in that order.
    pub production_seeds: Vec<ContextualImageQuery>,
}

pub(crate) fn contextual_image_query_candidates(
    language: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
    entity_facts: &[semantic_core::EntityFact],
) -> ContextualQueries {
    let max_contextual_queries =
        crate::consolidate::image_query_policy().max_contextual_queries_per_scene;
    // The boolean records the origin: `true` for a fact-grounded relation
    // query that satisfies the manifest contract, `false` for a visual-context
    // family that stays out of the manifest.
    let mut combined: Vec<(bool, ContextualImageQuery)> = fact_grounded_location_queries(
        source,
        sentences,
        entities,
        entity_ids,
        entity_facts,
    )
    .into_iter()
    .map(|query| (true, query))
    .collect();
    for candidate in linked_clause_visual_queries(language, source, sentences, entities) {
        if combined.len() >= max_contextual_queries {
            break;
        }
        if !combined
            .iter()
            .any(|(_, existing)| existing.query.eq_ignore_ascii_case(&candidate.query))
        {
            combined.push((false, candidate));
        }
    }
    for candidate in sentence_visual_slot_queries(language, source, sentences, entities) {
        if combined.len() >= max_contextual_queries {
            break;
        }
        if !combined
            .iter()
            .any(|(_, existing)| existing.query.eq_ignore_ascii_case(&candidate.query))
        {
            combined.push((false, candidate));
        }
    }
    for candidate in inherited_scene_visual_queries(language, source, sentences, entities) {
        if combined.len() >= max_contextual_queries {
            break;
        }
        if !combined
            .iter()
            .any(|(_, existing)| existing.query.eq_ignore_ascii_case(&candidate.query))
        {
            combined.push((false, candidate));
        }
    }
    combined.truncate(max_contextual_queries);
    combined.sort_by_key(|(_, candidate)| candidate.sentence_index);
    let mut production_seeds: Vec<ContextualImageQuery> =
        combined.iter().map(|(_, query)| query.clone()).collect();
    for (index, candidate) in production_seeds.iter_mut().enumerate() {
        candidate.id = format!("contextual_{:03}", index + 1);
    }
    let mut manifest_queries: Vec<ContextualImageQuery> = combined
        .into_iter()
        .filter(|(grounded, _)| *grounded)
        .map(|(_, query)| query)
        .collect();
    for (index, candidate) in manifest_queries.iter_mut().enumerate() {
        candidate.id = format!("contextual_{:03}", index + 1);
    }
    ContextualQueries {
        manifest_queries,
        production_seeds,
    }
}

fn fact_grounded_location_queries(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    entity_ids: &[String],
    facts: &[semantic_core::EntityFact],
) -> Vec<ContextualImageQuery> {
    let mut output = Vec::new();
    for fact in facts {
        if fact.predicate != "located_in"
            || fact.status != "asserted"
            || fact.target_entity_id.is_none()
            || fact.value.trim().is_empty()
        {
            continue;
        }
        let Some(sentence) = sentences.get(fact.sentence_index) else {
            continue;
        };
        let Some(subject) = entities.iter().enumerate().find_map(|(index, entity)| {
            (entity_ids
                .get(index)
                .is_some_and(|id| id == &fact.subject_entity_id)
                && entity.start >= sentence.start_byte
                && entity.end <= sentence.end_byte
                && is_visual_query_entity(&entity.r#type))
            .then_some(entity)
        }) else {
            continue;
        };
        if fact.start_byte >= fact.end_byte
            || source.get(fact.start_byte..fact.end_byte) != Some(fact.value.as_str())
            || fact.evidence_start_byte >= fact.evidence_end_byte
            || source.get(fact.evidence_start_byte..fact.evidence_end_byte)
                != Some(fact.evidence.as_str())
        {
            continue;
        }
        let Some(location) = entities.iter().enumerate().find_map(|(index, entity)| {
            (entity_ids.get(index).is_some_and(|id| {
                fact.target_entity_id
                    .as_ref()
                    .is_some_and(|target| id == target)
            }) && entity.r#type == "LOCATION"
                && entity.text == fact.value
                && entity.start == fact.start_byte
                && entity.end == fact.end_byte)
                .then_some(entity)
        }) else {
            // The same canonical place may have been extracted at a different
            // mention. The fact's own value span is the grounded target here.
            let components = vec![
                visual_entity_component(subject),
                semantic_core::QueryComponent {
                    kind: "location".to_string(),
                    component_type: "LOCATION".to_string(),
                    text: fact.value.clone(),
                    start_byte: fact.start_byte,
                    end_byte: fact.end_byte,
                },
            ];
            output.push(location_fact_query(source, fact, components));
            continue;
        };
        let components = vec![
            visual_entity_component(subject),
            visual_entity_component(location),
        ];
        output.push(location_fact_query(source, fact, components));
    }
    output
}

fn visual_entity_component(entity: &VisualEntity) -> semantic_core::QueryComponent {
    semantic_core::QueryComponent {
        kind: visual_component_kind(&entity.r#type).to_string(),
        component_type: entity.r#type.clone(),
        text: entity.text.clone(),
        start_byte: entity.start,
        end_byte: entity.end,
    }
}

fn location_fact_query(
    source: &str,
    fact: &semantic_core::EntityFact,
    components: Vec<semantic_core::QueryComponent>,
) -> ContextualImageQuery {
    let query = components
        .iter()
        .map(|component| component.text.trim())
        .collect::<Vec<_>>()
        .join(" ");
    ContextualImageQuery {
        id: String::new(),
        kind: "contextual_image_query".to_string(),
        query,
        evidence: source
            .get(fact.evidence_start_byte..fact.evidence_end_byte)
            .unwrap_or_default()
            .to_string(),
        evidence_start_byte: fact.evidence_start_byte,
        evidence_end_byte: fact.evidence_end_byte,
        sentence_index: fact.sentence_index,
        components,
    }
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

/// Builds one image search composition from the visual slots in each source
/// clause. It intentionally does not form the Cartesian product of entities:
/// an object/product is the anchor, then only its nearby brand, actor, place,
/// and one source-backed action cue can join it.
fn sentence_visual_slot_queries(
    language: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<ContextualImageQuery> {
    let mut output = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        for (clause_start, clause_end) in visual_clause_spans(sentence_text) {
            let absolute_start = sentence.start_byte + clause_start;
            let absolute_end = sentence.start_byte + clause_end;
            let Some(evidence) = source.get(absolute_start..absolute_end) else {
                continue;
            };
            let comparison = is_explicit_visual_comparison(evidence, language);
            let lower_evidence = evidence.to_lowercase();
            let future_proposal = crate::consolidate::image_query_policy()
                .irrealis_visual_cues
                .iter()
                .any(|cue| lower_evidence.contains(cue.to_lowercase().as_str()));
            if (has_explicit_link_disclaimer(evidence, language) && !comparison)
                || (semantic_core::relations::clause_status(evidence, &[language])
                    != semantic_core::RelationStatus::Asserted
                    && !comparison
                    && !future_proposal)
            {
                continue;
            }
            let mut rows: Vec<&VisualEntity> = entities
                .iter()
                .filter(|entity| {
                    entity.start >= absolute_start
                        && entity.end <= absolute_end
                        && source.get(entity.start..entity.end) == Some(entity.text.as_str())
                        && is_visual_query_entity(&entity.r#type)
                })
                .collect();
            rows.sort_by(|left, right| {
                visual_slot_rank(&left.r#type)
                    .cmp(&visual_slot_rank(&right.r#type))
                    .then(left.start.cmp(&right.start))
                    .then(right.end.cmp(&left.end))
            });
            // A typed compound owns its nested brand span. Keeping both would
            // repeat the same product in the query.
            rows.dedup_by(|right, left| left.start == right.start && left.end == right.end);
            let all_rows = rows.clone();
            rows.retain(|entity| {
                !all_rows.iter().any(|other| {
                    other.start <= entity.start
                        && entity.end <= other.end
                        && (other.start < entity.start || entity.end < other.end)
                        && matches!(
                            other.r#type.as_str(),
                            "PRODUCT" | "OBJECT" | "FACILITY" | "BRAND" | "ORG" | "ORGANIZATION"
                        )
                })
            });

            let objects: Vec<&VisualEntity> = rows
                .iter()
                .copied()
                .filter(|entity| {
                    matches!(
                        entity.r#type.as_str(),
                        "PRODUCT" | "OBJECT" | "FACILITY" | "STRUCTURE"
                    ) && !crate::consolidate::image_query_policy()
                        .non_subject_object_terms
                        .iter()
                        .any(|term| entity.text.eq_ignore_ascii_case(term))
                })
                .collect();
            let brands: Vec<&VisualEntity> = rows
                .iter()
                .copied()
                .filter(|entity| matches!(entity.r#type.as_str(), "BRAND" | "ORG" | "ORGANIZATION"))
                .collect();
            let people: Vec<&VisualEntity> = rows
                .iter()
                .copied()
                .filter(|entity| entity.r#type == "PERSON")
                .filter(|entity| !is_passive_visual_owner(evidence, entity))
                .collect();
            let locations: Vec<&VisualEntity> = rows
                .iter()
                .copied()
                .filter(|entity| entity.r#type == "LOCATION")
                .collect();
            let action = visual_action_component(source, absolute_start, absolute_end, evidence);
            let mut anchors: Vec<&VisualEntity> = Vec::new();
            if comparison && brands.len() >= 2 {
                // A comparison can join two named makers, but a denial of
                // collaboration never changes the comparison into a claim.
                if let Some(object) = objects.first() {
                    anchors.push(object);
                }
                anchors.extend(brands.iter().take(2).copied());
            } else if let Some(object) = objects.first() {
                if action.is_some() {
                    if let Some(person) = people.first() {
                        anchors.push(person);
                    }
                }
                anchors.push(object);
                if let Some(brand) = closest_related_brand(object, &brands) {
                    anchors.push(brand);
                }
            } else if !brands.is_empty() {
                // Brand/place and organization/place scenes still form useful
                // visual searches when the text names no person.
                anchors.push(brands[0]);
                if let Some(person) = people.first().filter(|_| action.is_some()) {
                    anchors.insert(0, person);
                }
            } else if locations.len() >= 2 {
                // Only join a landmark to a location explicitly introduced
                // with a location phrase; two landmarks listed together do
                // not become a single scene query.
                if let Some((landmark, place)) = locations.iter().copied().find_map(|place| {
                    locations
                        .iter()
                        .copied()
                        .find(|landmark| {
                            landmark.start != place.start
                                && explicit_location_link(source, landmark, place)
                        })
                        .map(|landmark| (landmark, place))
                }) {
                    anchors.push(landmark);
                    anchors.push(place);
                }
            } else if let Some(person) = people.first() {
                anchors.push(person);
            }
            if let Some(location) = locations
                .iter()
                .copied()
                .filter(|location| {
                    anchors
                        .last()
                        .is_some_and(|anchor| explicit_location_link(source, anchor, location))
                })
                .min_by_key(|entity| {
                    anchors
                        .last()
                        .map_or(usize::MAX, |anchor| anchor.start.abs_diff(entity.start))
                })
            {
                if !anchors
                    .iter()
                    .any(|anchor| anchor.start == location.start && anchor.end == location.end)
                {
                    anchors.push(location);
                }
            }
            anchors.dedup_by(|right, left| left.start == right.start && left.end == right.end);
            if anchors.is_empty() || (anchors.len() < 2 && action.is_none()) {
                continue;
            }
            let mut components: Vec<semantic_core::QueryComponent> = anchors
                .iter()
                .map(|entity| semantic_core::QueryComponent {
                    kind: visual_component_kind(&entity.r#type).to_string(),
                    component_type: entity.r#type.clone(),
                    text: entity.text.clone(),
                    start_byte: entity.start,
                    end_byte: entity.end,
                })
                .collect();
            if let Some(action) = action {
                components.push(action);
            }
            let query = components
                .iter()
                .map(|component| component.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            if query.split_whitespace().count() > 7 || query.chars().count() > 64 {
                continue;
            }
            output.push(ContextualImageQuery {
                id: String::new(),
                kind: "contextual_image_query".to_string(),
                query,
                evidence: evidence.to_string(),
                evidence_start_byte: absolute_start,
                evidence_end_byte: absolute_end,
                sentence_index,
                components,
            });
            if output.len() >= 10 {
                return output;
            }
        }
    }
    output
}

/// Joins two adjacent clauses only when a configured simultaneous-action
/// marker links them, the first clause names an actor, and both clauses have
/// affirmative visual predicates. This preserves scene context without
/// treating arbitrary same-sentence entities as related.
fn linked_clause_visual_queries(
    language: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<ContextualImageQuery> {
    let policy = crate::consolidate::image_query_policy();
    let mut output = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let lower_sentence = sentence_text.to_lowercase();
        if !policy
            .linked_clause_visual_cues
            .iter()
            .any(|cue| lower_sentence.contains(cue.to_lowercase().as_str()))
        {
            continue;
        }
        let clauses = visual_clause_spans(sentence_text);
        for pair in clauses.windows(2) {
            let (left_start, left_end) = pair[0];
            let (right_start, right_end) = pair[1];
            let left_absolute_start = sentence.start_byte + left_start;
            let left_absolute_end = sentence.start_byte + left_end;
            let right_absolute_start = sentence.start_byte + right_start;
            let right_absolute_end = sentence.start_byte + right_end;
            let left_text = &sentence_text[left_start..left_end];
            let right_text = &sentence_text[right_start..right_end];
            if semantic_core::relations::clause_status(left_text, &[language])
                != semantic_core::RelationStatus::Asserted
                || semantic_core::relations::clause_status(right_text, &[language])
                    != semantic_core::RelationStatus::Asserted
                || visual_action_component(
                    source,
                    left_absolute_start,
                    left_absolute_end,
                    left_text,
                )
                .is_none()
                || visual_action_component(
                    source,
                    right_absolute_start,
                    right_absolute_end,
                    right_text,
                )
                .is_none()
            {
                continue;
            }
            let left_entities: Vec<&VisualEntity> = entities
                .iter()
                .filter(|entity| {
                    entity.start >= left_absolute_start && entity.end <= left_absolute_end
                })
                .collect();
            let right_entities: Vec<&VisualEntity> = entities
                .iter()
                .filter(|entity| {
                    entity.start >= right_absolute_start && entity.end <= right_absolute_end
                })
                .collect();
            if !left_entities.iter().any(|entity| entity.r#type == "PERSON") {
                continue;
            }
            let left_object = left_entities.iter().copied().find(|entity| {
                matches!(entity.r#type.as_str(), "PRODUCT" | "OBJECT" | "FACILITY")
                    && !policy
                        .non_subject_object_terms
                        .iter()
                        .any(|term| entity.text.eq_ignore_ascii_case(term))
            });
            let right_objects: Vec<&VisualEntity> = right_entities
                .iter()
                .copied()
                .filter(|entity| {
                    matches!(
                        entity.r#type.as_str(),
                        "PRODUCT" | "OBJECT" | "FACILITY" | "STRUCTURE"
                    ) && !policy
                        .non_subject_object_terms
                        .iter()
                        .any(|term| entity.text.eq_ignore_ascii_case(term))
                })
                .collect();
            let (Some(left_object), Some(right_object)) =
                (left_object, right_objects.first().copied())
            else {
                continue;
            };
            let mut selected = vec![left_object];
            if let Some(brand) = left_entities
                .iter()
                .copied()
                .filter(|entity| matches!(entity.r#type.as_str(), "BRAND" | "ORG" | "ORGANIZATION"))
                .min_by_key(|brand| left_object.start.abs_diff(brand.start))
            {
                selected.push(brand);
            }
            selected.push(right_object);
            if let Some(additional_object) = right_objects.iter().copied().find(|entity| {
                !selected
                    .iter()
                    .any(|item| item.start <= entity.start && entity.end <= item.end)
            }) {
                selected.push(additional_object);
            }
            if let Some(right_brand) = right_entities.iter().copied().find(|entity| {
                matches!(
                    entity.r#type.as_str(),
                    "BRAND" | "ORG" | "ORGANIZATION" | "FACILITY"
                ) && !selected
                    .iter()
                    .any(|item| item.start <= entity.start && entity.end <= item.end)
            }) {
                selected.push(right_brand);
            }
            if let Some(location) = right_entities
                .iter()
                .copied()
                .find(|entity| entity.r#type == "LOCATION")
            {
                selected.push(location);
            }
            let components: Vec<semantic_core::QueryComponent> = selected
                .iter()
                .map(|entity| semantic_core::QueryComponent {
                    kind: visual_component_kind(&entity.r#type).to_string(),
                    component_type: entity.r#type.clone(),
                    text: entity.text.clone(),
                    start_byte: entity.start,
                    end_byte: entity.end,
                })
                .collect();
            let query = components
                .iter()
                .map(|component| component.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            if query.split_whitespace().count() > policy.max_query_words
                || query.chars().count() > policy.max_query_chars
            {
                continue;
            }
            output.push(ContextualImageQuery {
                id: String::new(),
                kind: "contextual_image_query".to_string(),
                query,
                evidence: sentence_text.to_string(),
                evidence_start_byte: sentence.start_byte,
                evidence_end_byte: sentence.end_byte,
                sentence_index,
                components,
            });
            break;
        }
    }
    output
}

/// Carries only clearly referenced scene context across adjacent sentences:
/// an event's named product/brand to "durante l'evento" and a city to an
/// explicit "suoi monumenti" list. It does not merge arbitrary co-mentions.
fn inherited_scene_visual_queries(
    language: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<ContextualImageQuery> {
    let mut output = Vec::new();
    let policy = crate::consolidate::image_query_policy();
    for (index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let lower = text.to_lowercase();
        let current_entities: Vec<&VisualEntity> = entities
            .iter()
            .filter(|entity| entity.start >= sentence.start_byte && entity.end <= sentence.end_byte)
            .collect();
        let has_scene_reference = policy
            .scene_reference_cues
            .iter()
            .any(|cue| lower.contains(cue.to_lowercase().as_str()));
        let has_anaphoric_object = current_entities.iter().any(|entity| {
            policy
                .anaphoric_object_terms
                .iter()
                .any(|term| entity.text.eq_ignore_ascii_case(term))
        });
        if (has_scene_reference || has_anaphoric_object)
            && visual_action_component(source, sentence.start_byte, sentence.end_byte, text)
                .is_some()
        {
            let object = current_entities
                .iter()
                .copied()
                .min_by_key(|entity| match entity.r#type.as_str() {
                    "PRODUCT" => 0,
                    "OBJECT" | "FACILITY" => 1,
                    _ => 2,
                })
                .filter(|entity| {
                    matches!(entity.r#type.as_str(), "PRODUCT" | "OBJECT" | "FACILITY")
                });
            if let Some(object) = object {
                // Search backwards for the nearest asserted event that named
                // both an actor/organization and a visual product. The
                // anaphor in this sentence is what licenses inheritance.
                for prior_index in (0..index).rev().take(4) {
                    let prior = &sentences[prior_index];
                    let prior_text = source.get(prior.start_byte..prior.end_byte).unwrap_or("");
                    if semantic_core::relations::clause_status(prior_text, &[language])
                        != semantic_core::RelationStatus::Asserted
                    {
                        continue;
                    }
                    let prior_entities: Vec<&VisualEntity> = entities
                        .iter()
                        .filter(|entity| {
                            entity.start >= prior.start_byte && entity.end <= prior.end_byte
                        })
                        .collect();
                    let product = prior_entities.iter().copied().find(|entity| {
                        matches!(entity.r#type.as_str(), "PRODUCT" | "OBJECT" | "FACILITY")
                    });
                    let brand = prior_entities.iter().copied().find(|entity| {
                        matches!(entity.r#type.as_str(), "BRAND" | "ORG" | "ORGANIZATION")
                    });
                    let actor = prior_entities
                        .iter()
                        .copied()
                        .find(|entity| entity.r#type == "PERSON");
                    if (product.is_some() && (brand.is_some() || actor.is_some()))
                        || (has_scene_reference && brand.is_some())
                    {
                        let mut components = Vec::new();
                        if has_scene_reference && !has_anaphoric_object {
                            if let Some(brand) = brand {
                                components.push(brand);
                            }
                        } else if let Some(actor) = actor {
                            components.push(actor);
                        }
                        if let Some(product) = product {
                            components.push(product);
                        }
                        if let Some(brand) = brand.filter(|brand| {
                            !components.iter().any(|entity| {
                                entity.start <= brand.start && brand.end <= entity.end
                            })
                        }) {
                            components.push(brand);
                        }
                        if let Some(location) = prior_entities
                            .iter()
                            .copied()
                            .find(|entity| entity.r#type == "LOCATION")
                        {
                            components.push(location);
                        }
                        components.push(object);
                        output.push(make_inherited_visual_query(
                            source,
                            prior.start_byte,
                            sentence.end_byte,
                            index,
                            components,
                        ));
                        break;
                    }
                }
            }
        }

        let has_location_reference = policy
            .location_inheritance_cues
            .iter()
            .any(|cue| lower.contains(cue.to_lowercase().as_str()));
        if has_location_reference {
            let city = entities
                .iter()
                .filter(|entity| {
                    entity.r#type == "LOCATION"
                        && entity.end <= sentence.start_byte
                        && sentence.start_byte.saturating_sub(entity.end) < 1200
                })
                .max_by_key(|entity| entity.end);
            if let Some(city) = city {
                for landmark in current_entities
                    .iter()
                    .filter(|entity| entity.r#type == "LOCATION")
                {
                    output.push(make_inherited_visual_query(
                        source,
                        city.start,
                        sentence.end_byte,
                        index,
                        vec![landmark, city],
                    ));
                }
            }
        }
    }
    output
}

fn make_inherited_visual_query(
    source: &str,
    evidence_start: usize,
    evidence_end: usize,
    sentence_index: usize,
    entities: Vec<&VisualEntity>,
) -> ContextualImageQuery {
    let components: Vec<semantic_core::QueryComponent> = entities
        .into_iter()
        .map(|entity| semantic_core::QueryComponent {
            kind: visual_component_kind(&entity.r#type).to_string(),
            component_type: entity.r#type.clone(),
            text: entity.text.clone(),
            start_byte: entity.start,
            end_byte: entity.end,
        })
        .collect();
    ContextualImageQuery {
        id: String::new(),
        kind: "contextual_image_query".to_string(),
        query: components
            .iter()
            .map(|component| component.text.trim())
            .collect::<Vec<_>>()
            .join(" "),
        evidence: source
            .get(evidence_start..evidence_end)
            .unwrap_or("")
            .to_string(),
        evidence_start_byte: evidence_start,
        evidence_end_byte: evidence_end,
        sentence_index,
        components,
    }
}

fn visual_slot_rank(entity_type: &str) -> u8 {
    match entity_type {
        "PRODUCT" => 0,
        "OBJECT" => 1,
        "FACILITY" | "STRUCTURE" => 2,
        "BRAND" | "ORG" | "ORGANIZATION" => 3,
        "PERSON" => 4,
        "LOCATION" => 5,
        _ => 6,
    }
}

fn visual_component_kind(entity_type: &str) -> &'static str {
    match entity_type {
        "PERSON" => "entity",
        "LOCATION" => "location",
        "ORG" | "ORGANIZATION" | "BRAND" => "related_entity",
        "FACILITY" | "STRUCTURE" => "facility",
        _ => "object",
    }
}

fn closest_related_brand<'a>(
    object: &VisualEntity,
    brands: &[&'a VisualEntity],
) -> Option<&'a VisualEntity> {
    brands
        .iter()
        .copied()
        .min_by_key(|brand| object.start.abs_diff(brand.start))
}

fn is_passive_visual_owner(clause: &str, entity: &VisualEntity) -> bool {
    let Some(relative) = clause.find(entity.text.as_str()) else {
        return false;
    };
    let prefix = clause[..relative].to_lowercase();
    crate::consolidate::image_query_policy()
        .passive_owner_cues
        .iter()
        .any(|cue| prefix.contains(cue.to_lowercase().as_str()))
}

fn explicit_location_link(source: &str, subject: &VisualEntity, location: &VisualEntity) -> bool {
    let policy = crate::consolidate::image_query_policy();
    if subject.end <= location.start {
        let Some(gap) = source.get(subject.end..location.start) else {
            return false;
        };
        let lower = gap.to_lowercase();
        let padded_gap = format!(" {} ", lower.trim());
        return policy
            .location_anchor_cues
            .iter()
            .any(|cue| padded_gap.contains(&format!(" {} ", cue.trim().to_lowercase())));
    }
    if location.end <= subject.start {
        let Some(gap) = source.get(location.end..subject.start) else {
            return false;
        };
        let lower = gap.to_lowercase();
        let padded_gap = format!(" {} ", lower.trim());
        return policy
            .location_preceding_subject_cues
            .iter()
            .any(|cue| padded_gap.contains(&format!(" {} ", cue.trim().to_lowercase())));
    }
    false
}

fn is_explicit_visual_comparison(clause: &str, _language: &str) -> bool {
    let lower = clause.to_lowercase();
    crate::consolidate::image_query_policy()
        .comparison_cues
        .iter()
        .any(|cue| lower.contains(cue.to_lowercase().as_str()))
}

fn has_explicit_link_disclaimer(clause: &str, _language: &str) -> bool {
    let lower = clause.to_lowercase();
    crate::consolidate::image_query_policy()
        .link_disclaimer_cues
        .iter()
        .any(|cue| lower.contains(cue.to_lowercase().as_str()))
}

fn visual_clause_spans(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_lowercase();
    let separators = &crate::consolidate::image_query_policy().visual_clause_separators;
    let mut cuts = Vec::new();
    for separator in separators {
        for (start, matched) in lower.match_indices(separator.as_str()) {
            let end = start + matched.len();
            let left_boundary = start == 0
                || lower[..start]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
            let right_boundary =
                end == lower.len() || lower[end..].chars().next().is_some_and(char::is_whitespace);
            if left_boundary && right_boundary {
                cuts.push((start, end));
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut spans = Vec::new();
    let mut start = 0;
    for (cut_start, cut_end) in cuts {
        if start < cut_start {
            let raw = &text[start..cut_start];
            let left = raw.len()
                - raw
                    .trim_start_matches(|ch: char| ch.is_whitespace() || ch == ',')
                    .len();
            let right = raw
                .trim_end_matches(|ch: char| ch.is_whitespace() || ch == ',')
                .len();
            if left < right {
                spans.push((start + left, start + right));
            }
        }
        start = cut_end;
    }
    if start < text.len() {
        let raw = &text[start..];
        let left = raw.len()
            - raw
                .trim_start_matches(|ch: char| ch.is_whitespace() || ch == ',')
                .len();
        let right = raw
            .trim_end_matches(|ch: char| ch.is_whitespace() || ch == ',')
            .len();
        if left < right {
            spans.push((start + left, start + right));
        }
    }
    if spans.is_empty() {
        spans.push((0, text.len()));
    }
    spans
}

fn visual_action_component(
    source: &str,
    start: usize,
    end: usize,
    clause: &str,
) -> Option<semantic_core::QueryComponent> {
    let lower = clause.to_lowercase();
    let policy = crate::consolidate::image_query_policy();
    // A lexical cue can describe a document action rather than an imageable
    // event (for example, "ricorso presentato in Cassazione"). In that case
    // retain the entities as independent candidates without adding a visual
    // action to their composed query.
    if policy
        .nonvisual_action_context_phrases
        .iter()
        .any(|phrase| lower.contains(&phrase.to_lowercase()))
    {
        return None;
    }
    // Policy order ranks useful visual frames above incidental verbs. If two
    // cues start at the same position, keep the longest configured phrase so
    // "annullando con rinvio" stays intact in both query and evidence.
    let mut selected_cue: Option<(&str, usize)> = None;
    for cue in &policy.visual_action_cues {
        let Some(offset) = lower.find(&cue.to_lowercase()) else {
            continue;
        };
        match selected_cue {
            None => selected_cue = Some((cue, offset)),
            Some((selected, selected_offset))
                if offset == selected_offset && cue.len() > selected.len() =>
            {
                selected_cue = Some((cue, offset));
            }
            _ => {}
        }
    }
    let cue = selected_cue?;
    let absolute_start = start + cue.1;
    let absolute_end = absolute_start + cue.0.len();
    if absolute_end > end {
        return None;
    }
    let text = source.get(absolute_start..absolute_end)?.to_string();
    Some(semantic_core::QueryComponent {
        kind: "action".to_string(),
        component_type: "ACTION".to_string(),
        text,
        start_byte: absolute_start,
        end_byte: absolute_end,
    })
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
        let sentences = semantic_core::split_sentences(source, "en", None);
        let queries =
            contextual_image_query_candidates("en", source, &sentences, &entities, &[], &[]);
        // Without an asserted fact nothing may enter the manifest collection:
        // the visual-context family is a production seed only.
        assert!(queries.manifest_queries.is_empty());
        assert_eq!(queries.production_seeds.len(), 1);
        let query = &queries.production_seeds[0];
        assert_eq!(query.query, "Aurora Stadium Torino");
        for component in &query.components {
            assert!(component.start_byte >= evidence_start);
            assert!(component.end_byte <= evidence_end);
            assert_eq!(
                &source[component.start_byte..component.end_byte],
                component.text
            );
        }
    }
}
