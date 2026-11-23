use super::*;

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
        language,
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
    language: &str,
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
            let query_place =
                semantic_core::lexicon::lexicon().canonical_query_place(&fact.value, language);
            output.push(location_fact_query(source, fact, components, query_place));
            continue;
        };
        let components = vec![
            visual_entity_component(subject),
            visual_entity_component(location),
        ];
        let query_place =
            semantic_core::lexicon::lexicon().canonical_query_place(&fact.value, language);
        output.push(location_fact_query(source, fact, components, query_place));
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
    query_place: Option<&str>,
) -> ContextualImageQuery {
    let last = components.len().saturating_sub(1);
    let query = components
        .iter()
        .enumerate()
        .map(|(index, component)| {
            if index == last {
                query_place.unwrap_or(component.text.trim())
            } else {
                component.text.trim()
            }
        })
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

pub(super) fn image_pair_quality(left_type: &str, right_type: &str) -> usize {
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

mod visual_queries;
use visual_queries::*;
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

pub(crate) mod helpers;
use helpers::*;
pub(super) use helpers::{is_image_component_type, is_visual_query_entity};
