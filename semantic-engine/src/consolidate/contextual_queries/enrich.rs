//! Attaches source-grounded contextual image queries to entity searches.

use std::collections::{HashMap, HashSet};

use semantic_core::{
    Candidate, ContextualImageQuery, ProductionCandidate, QueryComponent, SceneInput,
    SceneManifest, SearchQuery,
};

use super::{image_query_policy, visual_query_owner_rank, ImageQueryPolicy};

/// Per-script lookups shared by every candidate: entity spans, identity spans,
/// the contexts each scene carries, and which identities can issue a search.
struct EnrichmentIndex<'a> {
    sources_by_scene: HashMap<&'a str, &'a SceneInput>,
    entities_by_span: HashMap<(String, String, usize, usize), &'a Candidate>,
    identity_by_span: HashMap<(String, usize, usize), String>,
    entities_by_scene: HashMap<&'a str, Vec<&'a Candidate>>,
    contexts_by_scene: HashMap<&'a str, Vec<&'a ContextualImageQuery>>,
    searchable_entity_ids: HashSet<(String, String)>,
}

impl<'a> EnrichmentIndex<'a> {
    fn build(
        sources: &'a [SceneInput],
        scenes: &'a [SceneManifest],
        query_seeds: &'a HashMap<&'a str, &'a [ContextualImageQuery]>,
        candidates: &[ProductionCandidate],
    ) -> Self {
        let sources_by_scene = sources
            .iter()
            .map(|source| (source.scene_id.as_str(), source))
            .collect();
        let mut entities_by_span = HashMap::new();
        let mut identity_by_span = HashMap::new();
        let mut entities_by_scene: HashMap<&str, Vec<&Candidate>> = HashMap::new();
        let mut contexts_by_scene = HashMap::new();
        for scene in scenes {
            for entity in &scene.entities {
                entities_by_scene
                    .entry(scene.scene_id.as_str())
                    .or_default()
                    .push(entity);
                entities_by_span.insert(
                    (
                        scene.scene_id.clone(),
                        entity.id.clone(),
                        entity.start_byte,
                        entity.end_byte,
                    ),
                    entity,
                );
                identity_by_span.insert(
                    (scene.scene_id.clone(), entity.start_byte, entity.end_byte),
                    entity.id.clone(),
                );
            }
            let contextual_queries = query_seeds
                .get(scene.scene_id.as_str())
                .copied()
                .unwrap_or(scene.contextual_image_queries.as_slice());
            contexts_by_scene.insert(scene.scene_id.as_str(), contextual_queries.iter().collect());
        }
        let searchable_entity_ids = candidates
            .iter()
            .filter(|candidate| {
                candidate.recipe == "entity_search" && candidate.entity_ids.len() == 1
            })
            .map(|candidate| (candidate.scene_id.clone(), candidate.entity_ids[0].clone()))
            .collect();
        Self {
            sources_by_scene,
            entities_by_span,
            identity_by_span,
            entities_by_scene,
            contexts_by_scene,
            searchable_entity_ids,
        }
    }

    /// The best visual owner of a context by entity type (product/object before
    /// people and places), regardless of whether it can issue a search.
    fn context_owner(&self, scene_id: &str, context: &ContextualImageQuery) -> Option<&str> {
        context
            .components
            .iter()
            .filter_map(|component| {
                let id = self.identity_by_span.get(&(
                    scene_id.to_string(),
                    component.start_byte,
                    component.end_byte,
                ))?;
                let entity = self.entities_by_span.get(&(
                    scene_id.to_string(),
                    id.clone(),
                    component.start_byte,
                    component.end_byte,
                ))?;
                Some((visual_query_owner_rank(&entity.kind), id.as_str()))
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, id)| id)
    }

    /// The owner that actually competes for a media slot: prefer the typed
    /// owner, and fall back to any searchable component of the same context.
    fn effective_context_owner(
        &self,
        scene_id: &str,
        context: &ContextualImageQuery,
    ) -> Option<&str> {
        let preferred = self.context_owner(scene_id, context);
        if preferred.is_some_and(|id| {
            self.searchable_entity_ids
                .contains(&(scene_id.to_string(), id.to_string()))
        }) {
            return preferred;
        }
        context
            .components
            .iter()
            .filter_map(|component| {
                let id = self.identity_by_span.get(&(
                    scene_id.to_string(),
                    component.start_byte,
                    component.end_byte,
                ))?;
                if !self
                    .searchable_entity_ids
                    .contains(&(scene_id.to_string(), id.clone()))
                {
                    return None;
                }
                let entity = self.entities_by_span.get(&(
                    scene_id.to_string(),
                    id.clone(),
                    component.start_byte,
                    component.end_byte,
                ))?;
                Some((visual_query_owner_rank(&entity.kind), id.as_str()))
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, id)| id)
    }
}

/// The lower-cased sentence around the candidate anchor.
fn candidate_sentence_text(source: &SceneInput, candidate: &ProductionCandidate) -> String {
    let anchor_start = candidate.start_byte.min(source.text.len());
    let anchor_end = candidate.end_byte.min(source.text.len());
    let sentence_start = source.text[..anchor_start]
        .rfind(['.', '!', '?', '\n'])
        .map_or(0, |index| index + 1);
    let sentence_end = source.text[anchor_end..]
        .find(['.', '!', '?', '\n'])
        .map_or(source.text.len(), |index| anchor_end + index + 1);
    source
        .text
        .get(sentence_start..sentence_end)
        .unwrap_or("")
        .to_lowercase()
}

/// True when the context covers the candidate span but another identity owns
/// it.
fn context_covers_candidate(
    index: &EnrichmentIndex<'_>,
    scene_id: &str,
    context: &ContextualImageQuery,
    candidate: &ProductionCandidate,
) -> bool {
    let identity = candidate.entity_ids[0].as_str();
    context.components.iter().any(|component| {
        let component_identity = index.identity_by_span.get(&(
            scene_id.to_string(),
            component.start_byte,
            component.end_byte,
        ));
        component_identity.is_some_and(|id| id == identity)
            || (component.start_byte <= candidate.start_byte
                && candidate.end_byte <= component.end_byte)
    })
}

/// Applies the three suppression gates that do not depend on query shape.
fn should_disable(
    index: &EnrichmentIndex<'_>,
    candidate: &ProductionCandidate,
    source: &SceneInput,
    contexts: &[&ContextualImageQuery],
    policy: &ImageQueryPolicy,
) -> bool {
    if policy
        .non_subject_object_terms
        .iter()
        .any(|term| candidate.title.eq_ignore_ascii_case(term))
    {
        return true;
    }
    let candidate_text = candidate_sentence_text(source, candidate);
    let disconnected_meta_sentence = policy
        .link_disclaimer_cues
        .iter()
        .any(|cue| candidate_text.contains(cue.to_lowercase().as_str()));
    let explicit_comparison_context = contexts.iter().any(|context| {
        context.evidence_start_byte <= candidate.start_byte
            && candidate.start_byte <= context.evidence_end_byte
            && context.components.iter().any(|component| {
                component.component_type == "ACTION"
                    && policy
                        .comparison_cues
                        .iter()
                        .any(|cue| component.text.eq_ignore_ascii_case(cue))
            })
    });
    if disconnected_meta_sentence && !explicit_comparison_context {
        return true;
    }
    let identity = candidate.entity_ids[0].as_str();
    let owns_any_context = contexts.iter().any(|context| {
        index.effective_context_owner(&candidate.scene_id, context) == Some(identity)
    });
    let covered_by_context = contexts.iter().any(|context| {
        index
            .effective_context_owner(&candidate.scene_id, context)
            .is_some_and(|owner| owner != identity)
            && context_covers_candidate(index, &candidate.scene_id, context, candidate)
    });
    covered_by_context && !owns_any_context
}

/// A nested location explicitly linked to the context's existing location
/// anchor ("nel quartiere di", "in the district of").
fn nested_location_component(
    index: &EnrichmentIndex<'_>,
    scene_id: &str,
    source: &SceneInput,
    context: &ContextualImageQuery,
    existing_end: usize,
    policy: &ImageQueryPolicy,
) -> Option<QueryComponent> {
    let entity = index
        .entities_by_scene
        .get(scene_id)?
        .iter()
        .filter(|entity| entity.kind == "LOCATION")
        .filter(|entity| {
            entity.start_byte > existing_end
                && context.evidence_start_byte <= entity.start_byte
                && entity.end_byte <= context.evidence_end_byte
        })
        .find(|entity| {
            source
                .text
                .get(existing_end..entity.start_byte)
                .is_some_and(|gap| {
                    let gap = gap.to_lowercase();
                    policy
                        .nested_location_anchor_cues
                        .iter()
                        .any(|cue| gap.ends_with(&cue.to_lowercase()))
                })
        })?;
    Some(QueryComponent {
        kind: "location".to_string(),
        component_type: entity.kind.clone(),
        text: entity.text.clone(),
        start_byte: entity.start_byte,
        end_byte: entity.end_byte,
    })
}

/// Builds the contextual rung for one owned context, or `None` when any
/// grounding/size invariant fails.
fn contextual_query_for(
    index: &EnrichmentIndex<'_>,
    candidate: &ProductionCandidate,
    context: &ContextualImageQuery,
    source: &SceneInput,
    policy: &ImageQueryPolicy,
) -> Option<SearchQuery> {
    if context.components.len() < 2 {
        return None;
    }
    let identity = candidate.entity_ids[0].as_str();
    let contains_anchor = context.components.iter().any(|component| {
        index
            .entities_by_span
            .get(&(
                candidate.scene_id.clone(),
                identity.to_string(),
                component.start_byte,
                component.end_byte,
            ))
            .is_some_and(|entity| {
                entity.text == component.text
                    && candidate
                        .evidence
                        .trim()
                        .eq_ignore_ascii_case(component.text.trim())
            })
    });
    if !contains_anchor {
        return None;
    }
    let mut components = context.components.clone();
    let existing_location = components
        .iter()
        .filter(|component| component.component_type == "LOCATION")
        .max_by_key(|component| component.end_byte)?;
    if let Some(nested) = nested_location_component(
        index,
        &candidate.scene_id,
        source,
        context,
        existing_location.end_byte,
        policy,
    ) {
        if !components.iter().any(|component| {
            component.start_byte == nested.start_byte && component.end_byte == nested.end_byte
        }) {
            components.push(nested);
            components.sort_by_key(|component| component.start_byte);
        }
    }
    let query = components
        .iter()
        .map(|component| component.text.trim())
        .collect::<Vec<_>>()
        .join(" ");
    if query.split_whitespace().count() > policy.max_query_words
        || query.chars().count() > policy.max_query_chars
        || candidate
            .queries
            .iter()
            .any(|prior| prior.text.eq_ignore_ascii_case(&query))
    {
        return None;
    }
    let evidence_text = source
        .text
        .get(context.evidence_start_byte..context.evidence_end_byte)
        .unwrap_or_default();
    let evidence_valid = context.evidence_start_byte <= context.evidence_end_byte
        && evidence_text == context.evidence;
    let components_valid = components.iter().all(|component| {
        component.start_byte <= component.end_byte
            && context.evidence_start_byte <= component.start_byte
            && component.end_byte <= context.evidence_end_byte
            && source
                .text
                .get(component.start_byte..component.end_byte)
                .is_some_and(|text| text == component.text)
    });
    let original_query = context
        .components
        .iter()
        .map(|component| component.text.trim())
        .collect::<Vec<_>>()
        .join(" ");
    if !evidence_valid || !components_valid || original_query != context.query {
        return None;
    }
    Some(SearchQuery::contextual(
        query,
        context.evidence.clone(),
        context.evidence_start_byte,
        context.evidence_end_byte,
        components,
        false,
    ))
}

/// Enriches one entity-search candidate, or disables it when the context
/// assigns its media slot elsewhere.
fn enrich_candidate(
    index: &EnrichmentIndex<'_>,
    candidate: &mut ProductionCandidate,
    source: &SceneInput,
    contexts: &[&ContextualImageQuery],
    policy: &ImageQueryPolicy,
) {
    if should_disable(index, candidate, source, contexts, policy) {
        candidate.requires_media = false;
        candidate.queries.clear();
        return;
    }
    let identity = candidate.entity_ids[0].as_str();
    let mut contextual_queries = Vec::new();
    for context in contexts.iter().copied().filter(|context| {
        index.effective_context_owner(&candidate.scene_id, context) == Some(identity)
    }) {
        if contextual_queries.len() >= policy.max_contextual_queries_per_candidate {
            break;
        }
        if let Some(query) = contextual_query_for(index, candidate, context, source, policy) {
            contextual_queries.push(query);
        }
    }
    if contextual_queries.is_empty() {
        return;
    }
    // Search the source-supported event first. The canonical identity query
    // remains the fallback when the composed search has no useful hit.
    if candidate.queries.is_empty() {
        candidate
            .queries
            .push(SearchQuery::literal(candidate.title.clone()));
    }
    candidate.requires_media = true;
    contextual_queries.append(&mut candidate.queries);
    candidate.queries = contextual_queries;
}

/// Adds one compact contextual search rung to non-person entity searches when
/// another grounded visual entity occurs in the same complete source sentence.
/// A person can combine with a non-person visual concept, while person-person
/// and other named-entity pairs require an asserted relation.
pub(crate) fn enrich_contextual_search_queries<'a>(
    _language: &str,
    sources: &'a [SceneInput],
    scenes: &'a [SceneManifest],
    query_seeds: &'a HashMap<&'a str, &'a [semantic_core::ContextualImageQuery]>,
    candidates: &mut [ProductionCandidate],
) {
    let policy = image_query_policy();
    let index = EnrichmentIndex::build(sources, scenes, query_seeds, candidates);
    for candidate in candidates
        .iter_mut()
        .filter(|candidate| candidate.recipe == "entity_search" && candidate.entity_ids.len() == 1)
    {
        let Some(source) = index
            .sources_by_scene
            .get(candidate.scene_id.as_str())
            .copied()
        else {
            continue;
        };
        let Some(contexts) = index.contexts_by_scene.get(candidate.scene_id.as_str()) else {
            continue;
        };
        enrich_candidate(&index, candidate, source, contexts, policy);
    }
}

#[cfg(test)]
#[path = "enrich_tests.rs"]
mod tests;
