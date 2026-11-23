use super::*;

/// Joins two adjacent clauses only when a configured simultaneous-action
/// marker links them, the first clause names an actor, and both clauses have
/// affirmative visual predicates. This preserves scene context without
/// treating arbitrary same-sentence entities as related.
pub(super) fn linked_clause_visual_queries(
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
pub(super) fn inherited_scene_visual_queries(
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
