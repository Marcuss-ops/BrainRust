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

/// Entity candidates for the manifest's identity graph: generic visual
/// concepts are dropped by KIND after each candidate has resolved its own
/// identity id.
///
/// The identity lookup is indexed by the scene's entity list, so filtering the
/// entities BEFORE the lookup renumbers it: every candidate after the first
/// dropped concept would carry the identity of the entity that used to sit in
/// its slot — including concepts the manifest deliberately omits, which the Go
/// side then rejects as an unborn reference. Dropping the finished candidates
/// keeps index and identity aligned.
pub(crate) fn typed_entity_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    identities: &IdentityLookup,
) -> Vec<Candidate> {
    entity_candidates(source, sentences, entities, identities)
        .into_iter()
        .filter(|candidate| candidate.kind != "VISUAL_CONCEPT")
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
/// is grounded when at least one sentence carries a grounded signal and every
/// sentence in it is substantive. A concluding sentence often carries context
/// rather than a new signal ("Il Comune mantenne quindi il monitoraggio…"), so
/// requiring a signal at both ends would drop the complete passage. The score
/// is the shared sentence salience (`semantic_core::bullets`) averaged over the
/// window, so bullets and segments rank with one weight table. Every grounded
/// window is retained in source order without a score threshold: overlay
/// selection belongs to PipelineGen, and a BrainRust cap would silently discard
/// valid passages before the planner can compare them. Every passage remains
/// contiguous and source-grounded. A scene without signals keeps an empty
/// collection.
pub(crate) fn segment_candidates(
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    facts: &[semantic_core::EntityFact],
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
            windows.push((score, start, end));
        }
    }
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

pub(crate) mod contextual;
pub(crate) use contextual::contextual_image_query_candidates;
use contextual::{image_pair_quality, is_image_component_type, is_visual_query_entity};

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

#[cfg(test)]
mod typed_entity_candidate_tests {
    use super::*;

    fn entity(text: &str, kind: &str, source: &str) -> VisualEntity {
        let start = source.find(text).unwrap();
        VisualEntity {
            text: text.into(),
            r#type: kind.into(),
            score: 0.9,
            start,
            end: start + text.len(),
            evidence: text.into(),
        }
    }

    // A dropped visual concept must not shift the identity of the entities that
    // follow it: the manifest has to reference identities it also publishes.
    #[test]
    fn dropping_visual_concepts_keeps_every_identity_reference_aligned() {
        let source = "The crowd roared as Mike Tyson entered the arena.";
        let entities = vec![
            entity("crowd", "VISUAL_CONCEPT", source),
            entity("Mike Tyson", "PERSON", source),
            entity("arena", "VISUAL_CONCEPT", source),
        ];
        let mut registry = semantic_core::IdentityRegistry::new();
        let context = visualner::ScriptContext::default();
        let identities = IdentityLookup::build(&mut registry, "scene", &entities, &context, "en");
        let sentences = semantic_core::split_sentences(source, "en", None);

        let candidates = typed_entity_candidates(source, &sentences, &entities, &identities);

        assert_eq!(
            candidates.len(),
            1,
            "only the typed entity survives: {candidates:?}"
        );
        assert_eq!(candidates[0].text, "Mike Tyson");
        assert_eq!(candidates[0].id, identities.id_of(1));
        assert_ne!(candidates[0].id, identities.id_of(0));
        let published: Vec<&str> = registry
            .identities()
            .iter()
            .filter(|identity| identity.kind != "VISUAL_CONCEPT")
            .map(|identity| identity.id.as_str())
            .collect();
        assert!(
            published.contains(&candidates[0].id.as_str()),
            "candidate {} references an identity the manifest omits ({published:?})",
            candidates[0].id
        );
    }
}
