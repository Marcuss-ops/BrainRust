//! Context-grounded search candidates and their source-backed ranking.

use super::*;

#[path = "contextual_queries/enrich.rs"]
mod enrich;
pub(super) use enrich::enrich_contextual_search_queries;

/// Rank a source-grounded scene query above a bare identity lookup. The query
/// ladder is already attached by `enrich_contextual_search_queries`; this pass
/// changes only production ordering and leaves the inventory and fallback rung
/// intact.
pub(super) fn apply_visual_query_priority(
    language: &str,
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut [ProductionCandidate],
) {
    let lexicon = semantic_core::lexicon::lexicon();
    let object_terms = lexicon.object_terms(language);

    // A grounded context query already searches every named component it
    // contains. Do not spend another image slot on one of those same names
    // in the same scene (for example, "plant Collins Park East Toledo" plus
    // a second search for "East Toledo"). Keep the identities in the
    // semantic inventory; only suppress the duplicate production search.
    let contextual_components: HashSet<(String, String)> = candidates
        .iter()
        .filter(|candidate| candidate.recipe == "contextual_search" && candidate.requires_media)
        .flat_map(|candidate| {
            candidate
                .queries
                .iter()
                .filter(|query| query.origin == "derived_from_source_context")
                .flat_map(move |query| {
                    query.components.iter().map(move |component| {
                        (candidate.scene_id.clone(), component.text.to_lowercase())
                    })
                })
        })
        .collect();
    for candidate in candidates.iter_mut().filter(|candidate| {
        candidate.recipe == "entity_search"
            && candidate.requires_media
            && !candidate
                .rationale
                .iter()
                .any(|reason| reason == "global_document_anchor")
    }) {
        if contextual_components
            .contains(&(candidate.scene_id.clone(), candidate.title.to_lowercase()))
        {
            candidate.requires_media = false;
            candidate.queries.clear();
            candidate
                .rationale
                .push("covered by a grounded contextual image query in this scene".to_string());
        }
    }

    for candidate in candidates
        .iter_mut()
        .filter(|candidate| candidate.recipe == "entity_search" && candidate.requires_media)
    {
        let contextual_query = candidate
            .queries
            .iter()
            .find(|query| query.origin == "derived_from_source_context");
        let owns_context = contextual_query.is_some_and(|query| {
            query.components.iter().any(|component| {
                component
                    .text
                    .trim()
                    .eq_ignore_ascii_case(candidate.title.trim())
            })
        });
        let component_count = if owns_context {
            contextual_query.map_or(0, |query| query.components.len())
        } else {
            0
        };
        let kind = candidate
            .entity_ids
            .first()
            .and_then(|id| identities.get(id))
            .map(|identity| identity.kind.as_str())
            .unwrap_or("");
        let is_document_anchor = candidate
            .rationale
            .iter()
            .any(|reason| reason == "global_document_anchor");
        let bare_object = matches!(kind, "OBJECT" | "PRODUCT" | "FACILITY" | "VISUAL_CONCEPT")
            && is_generic_object_surface(&candidate.title, object_terms, lexicon);
        candidate.priority = visual_search_priority(
            candidate.priority,
            candidate.title.split_whitespace().count(),
            bare_object,
            component_count,
            is_document_anchor,
        );
    }
}

/// Promotes each already-grounded contextual image query to a production
/// candidate in its own right. Previously contextual queries only existed as
/// rungs attached to an identity candidate; when that owner was absent or had
/// a lower identity score, the best visual event could never compete for a
/// media slot.
/// Promotes each already-grounded contextual image query to a production
/// candidate in its own right. Previously contextual queries only existed as
/// rungs attached to an identity candidate; when that owner was absent or had
/// a lower identity score, the best visual event could never compete for a
/// media slot.
pub(super) fn push_contextual_search_candidates(
    language: &str,
    sources: &[SceneInput],
    scenes: &[SceneManifest],
    query_seeds: &HashMap<&str, &[semantic_core::ContextualImageQuery]>,
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    let sources_by_scene: HashMap<&str, &str> = sources
        .iter()
        .map(|source| (source.scene_id.as_str(), source.text.as_str()))
        .collect();
    let mut seen = HashSet::new();
    for scene in scenes {
        let Some(source) = sources_by_scene.get(scene.scene_id.as_str()).copied() else {
            continue;
        };
        let contextual_queries = query_seeds
            .get(scene.scene_id.as_str())
            .copied()
            .unwrap_or(scene.contextual_image_queries.as_slice());
        for contextual in contextual_queries {
            let Some((components, has_visual_plan_evidence)) = resolve_contextual_components(
                language, source, scene, contextual, identities, candidates,
            ) else {
                continue;
            };
            let query = components
                .iter()
                .map(|component| component.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            let source_query = contextual
                .components
                .iter()
                .map(|component| component.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            if source_query != contextual.query || query.split_whitespace().count() > 7 {
                continue;
            }
            let dedup_key = format!("contextual_query:{}", query.to_lowercase());
            if !seen.insert(dedup_key.clone()) {
                continue;
            }
            let entity_ids = contextual_entity_ids(scene, &components, identities);
            let queries = contextual_search_queries(
                &query,
                contextual,
                &components,
                identities,
                &entity_ids,
                has_visual_plan_evidence,
            );
            candidates.push(build_contextual_candidate(
                scene,
                contextual,
                &components,
                &query,
                dedup_key,
                entity_ids,
                queries,
            ));
        }
    }
}

/// Applies the irrealis gate, finds the enriched component rung and validates
/// every span against the source before a candidate may be emitted.
fn resolve_contextual_components(
    language: &str,
    source: &str,
    scene: &SceneManifest,
    contextual: &semantic_core::ContextualImageQuery,
    identities: &BTreeMap<String, IdentityState>,
    candidates: &[ProductionCandidate],
) -> Option<(Vec<semantic_core::QueryComponent>, bool)> {
    let policy = image_query_policy();
    let lower_evidence = contextual.evidence.to_lowercase();
    let future_or_proposed = policy
        .irrealis_visual_cues
        .iter()
        .any(|cue| lower_evidence.contains(&cue.to_lowercase()));
    let has_visual_plan_evidence = policy
        .irrealis_visual_evidence_cues
        .iter()
        .any(|cue| lower_evidence.contains(&cue.to_lowercase()));
    if future_or_proposed && !has_visual_plan_evidence {
        return None;
    }
    let enriched_query = candidates
        .iter()
        .filter(|candidate| {
            candidate.scene_id == scene.scene_id && candidate.recipe == "entity_search"
        })
        .flat_map(|candidate| candidate.queries.iter())
        .find(|query| {
            query.origin == "derived_from_source_context"
                && query.evidence_start_byte == Some(contextual.evidence_start_byte)
                && query.evidence_end_byte == Some(contextual.evidence_end_byte)
        });
    let mut components = enriched_query
        .map(|query| query.components.clone())
        .unwrap_or_else(|| contextual.components.clone());
    append_nested_location(
        language,
        source,
        scene,
        contextual,
        identities,
        &mut components,
    );
    if !contextual_components_are_valid(source, contextual, &components) {
        return None;
    }
    Some((components, has_visual_plan_evidence))
}

/// Adds an explicitly linked nested place (for example "East Toledo" inside
/// "Toledo") when the source names it immediately after the location anchor.
fn append_nested_location(
    language: &str,
    source: &str,
    scene: &SceneManifest,
    contextual: &semantic_core::ContextualImageQuery,
    identities: &BTreeMap<String, IdentityState>,
    components: &mut Vec<semantic_core::QueryComponent>,
) {
    let policy = image_query_policy();
    let location_anchor_end = components
        .iter()
        .filter(|component| component.component_type == "LOCATION")
        .max_by_key(|component| component.end_byte)
        .map(|component| component.end_byte)
        .or_else(|| components.iter().map(|component| component.end_byte).max());
    let Some(anchor_end) = location_anchor_end else {
        return;
    };
    let location_is_explicitly_linked = |start: usize| {
        source.get(anchor_end..start).is_some_and(|gap| {
            let gap = gap.to_lowercase();
            policy
                .nested_location_anchor_cues
                .iter()
                .any(|cue| gap.ends_with(&cue.to_lowercase()))
        })
    };
    let nested_location = scene
        .entities
        .iter()
        .find(|entity| {
            entity.kind == "LOCATION"
                && entity.start_byte > anchor_end
                && entity.start_byte >= contextual.evidence_start_byte
                && entity.end_byte <= contextual.evidence_end_byte
                && location_is_explicitly_linked(entity.start_byte)
        })
        .map(|entity| {
            (
                entity.kind.clone(),
                entity.text.clone(),
                entity.start_byte,
                entity.end_byte,
            )
        })
        .or_else(|| {
            identities.values().find_map(|identity| {
                if identity.kind != "LOCATION"
                    && !(identity.kind == "VISUAL_CONCEPT"
                        && semantic_core::lexicon::lexicon()
                            .is_place(&identity.canonical, language))
                {
                    return None;
                }
                let evidence = source.get(anchor_end..contextual.evidence_end_byte)?;
                evidence
                    .match_indices(&identity.canonical)
                    .map(|(offset, text)| {
                        let start = anchor_end + offset;
                        (text, start, start + text.len())
                    })
                    .find(|(_, start, _)| location_is_explicitly_linked(*start))
                    .map(|(text, start, end)| (identity.kind.clone(), text.to_string(), start, end))
            })
        });
    if let Some((kind, text, start_byte, end_byte)) = nested_location {
        if !components
            .iter()
            .any(|component| component.start_byte == start_byte && component.end_byte == end_byte)
        {
            components.push(semantic_core::QueryComponent {
                kind: "location".to_string(),
                component_type: kind,
                text,
                start_byte,
                end_byte,
            });
            components.sort_by_key(|component| component.start_byte);
        }
    }
}

/// The evidence and every component must be exact source spans, and two
/// places must share an explicit relation inside one sentence.
fn contextual_components_are_valid(
    source: &str,
    contextual: &semantic_core::ContextualImageQuery,
    components: &[semantic_core::QueryComponent],
) -> bool {
    if components.len() < 2
        || contextual.evidence_start_byte >= contextual.evidence_end_byte
        || source.get(contextual.evidence_start_byte..contextual.evidence_end_byte)
            != Some(contextual.evidence.as_str())
        || contextual.query.trim().is_empty()
    {
        return false;
    }
    let components_are_grounded = components.iter().all(|component| {
        component.start_byte >= contextual.evidence_start_byte
            && component.start_byte < component.end_byte
            && component.end_byte <= contextual.evidence_end_byte
            && source
                .get(component.start_byte..component.end_byte)
                .is_some_and(|text| text == component.text)
    });
    if !components_are_grounded {
        return false;
    }
    let location_components: Vec<_> = components
        .iter()
        .filter(|component| component.component_type == "LOCATION")
        .collect();
    if location_components.len() >= 2 && location_components.len() == components.len() {
        let mut ordered = location_components;
        ordered.sort_by_key(|component| component.start_byte);
        let gap = source
            .get(ordered[0].end_byte..ordered[1].start_byte)
            .unwrap_or_default();
        let lower_gap = gap.to_lowercase();
        let explicit_place_relation = image_query_policy()
            .location_anchor_cues
            .iter()
            .any(|cue| lower_gap.ends_with(&cue.to_lowercase()));
        if !explicit_place_relation
            || gap
                .chars()
                .any(|character| matches!(character, '.' | '!' | '?' | '\n'))
        {
            // Two places in separate sentences are not a scene relation. This
            // catches list spillover such as a city named several sentences
            // before a train route.
            return false;
        }
    }
    true
}

fn contextual_entity_ids(
    scene: &SceneManifest,
    components: &[semantic_core::QueryComponent],
    identities: &BTreeMap<String, IdentityState>,
) -> Vec<String> {
    let mut entity_ids = Vec::new();
    for component in components {
        if let Some(identity) = scene.entities.iter().find(|entity| {
            entity.start_byte == component.start_byte
                && entity.end_byte == component.end_byte
                && entity.text == component.text
        }) {
            if identities.contains_key(&identity.id) && !entity_ids.contains(&identity.id) {
                entity_ids.push(identity.id.clone());
            }
        }
    }
    entity_ids
}

fn contextual_priority(components: &[semantic_core::QueryComponent]) -> f32 {
    let complexity_bonus = (components.len().saturating_sub(2).min(2) as f32) * 0.025;
    let action_bonus = if components
        .iter()
        .any(|component| component.component_type == "ACTION")
    {
        0.025
    } else {
        0.0
    };
    (0.79 + complexity_bonus + action_bonus).min(0.88)
}

fn contextual_search_queries(
    query: &str,
    contextual: &semantic_core::ContextualImageQuery,
    components: &[semantic_core::QueryComponent],
    identities: &BTreeMap<String, IdentityState>,
    entity_ids: &[String],
    has_visual_plan_evidence: bool,
) -> Vec<SearchQuery> {
    let mut queries = vec![SearchQuery::contextual(
        query.to_string(),
        contextual.evidence.clone(),
        contextual.evidence_start_byte,
        contextual.evidence_end_byte,
        components.to_vec(),
        has_visual_plan_evidence,
    )];
    for entity_id in entity_ids {
        let Some(identity) = identities.get(entity_id) else {
            continue;
        };
        if identity.canonical.eq_ignore_ascii_case(query)
            || queries
                .iter()
                .any(|candidate| candidate.text.eq_ignore_ascii_case(&identity.canonical))
        {
            continue;
        }
        queries.push(SearchQuery::literal(identity.canonical.clone()));
        if queries.len() >= 3 {
            break;
        }
    }
    queries
}

fn build_contextual_candidate(
    scene: &SceneManifest,
    contextual: &semantic_core::ContextualImageQuery,
    components: &[semantic_core::QueryComponent],
    query: &str,
    dedup_key: String,
    entity_ids: Vec<String>,
    queries: Vec<SearchQuery>,
) -> ProductionCandidate {
    ProductionCandidate {
        id: String::new(),
        recipe: "contextual_search".to_string(),
        rank: 0,
        priority: contextual_priority(components),
        scene_id: scene.scene_id.clone(),
        scene_ids: vec![scene.scene_id.clone()],
        entity_ids,
        fact_ids: Vec::new(),
        group_ids: Vec::new(),
        title: query.to_string(),
        evidence: contextual.evidence.clone(),
        start_byte: contextual.evidence_start_byte,
        end_byte: contextual.evidence_end_byte,
        suggested_visual: "search_asset".to_string(),
        chart_items: Vec::new(),
        chart_meta: None,
        visual_payload: None,
        temporal_surfaces: Vec::new(),
        asset_key: dedup_key.clone(),
        queries,
        requires_media: true,
        dedup_key,
        covered_fact_ids: Vec::new(),
        rationale: vec![
            "source-grounded contextual visual event".to_string(),
            "query components and evidence are exact source spans".to_string(),
        ],
    }
}
pub(super) fn is_generic_object_surface(
    surface: &str,
    object_terms: &[String],
    lexicon: &semantic_core::lexicon::Lexicon,
) -> bool {
    let normalized = surface.to_lowercase();
    if lexicon
        .gazetteer
        .object_words
        .iter()
        .any(|term| term == &normalized)
        || object_terms.iter().any(|term| term == &normalized)
    {
        return true;
    }
    let words: Vec<String> = normalized
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_string()
        })
        .filter(|word| !word.is_empty())
        .collect();
    let has_object_head = words.iter().any(|word| {
        lexicon
            .gazetteer
            .object_words
            .iter()
            .any(|term| term == word)
            || object_terms.iter().any(|term| term == word)
            || lexicon
                .facility
                .facility_nouns
                .iter()
                .any(|term| term == word)
    });
    let unqualified_common_phrase = surface
        .split_whitespace()
        .all(|word| word.chars().next().is_none_or(char::is_lowercase));
    has_object_head && words.len() <= 3 && unqualified_common_phrase
}

pub(super) fn visual_search_priority(
    priority: f32,
    query_words: usize,
    bare_object: bool,
    contextual_components: usize,
    is_document_anchor: bool,
) -> f32 {
    if contextual_components >= 2 {
        let bonus = if contextual_components >= 3 {
            0.18
        } else {
            0.12
        };
        return (priority + bonus).min(0.97);
    }
    // The document anchor is a deliberate exception: it is selected by
    // cross-scene recurrence and gazetteer evidence, not by name type alone.
    if is_document_anchor {
        return priority;
    }
    if bare_object {
        priority * 0.45
    } else if query_words == 1 {
        priority * 0.82
    } else {
        priority
    }
}

fn visual_query_owner_rank(entity_type: &str) -> u8 {
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

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct ImageQueryPolicy {
    pub(crate) max_contextual_queries_per_scene: usize,
    pub(crate) max_contextual_queries_per_candidate: usize,
    pub(crate) max_query_words: usize,
    pub(crate) max_query_chars: usize,
    pub(crate) visual_action_cues: Vec<String>,
    pub(crate) scene_reference_cues: Vec<String>,
    pub(crate) anaphoric_object_terms: Vec<String>,
    pub(crate) location_inheritance_cues: Vec<String>,
    pub(crate) visual_clause_separators: Vec<String>,
    pub(crate) linked_clause_visual_cues: Vec<String>,
    pub(crate) location_preceding_subject_cues: Vec<String>,
    pub(crate) comparison_cues: Vec<String>,
    pub(crate) link_disclaimer_cues: Vec<String>,
    pub(crate) irrealis_visual_cues: Vec<String>,
    pub(crate) irrealis_visual_evidence_cues: Vec<String>,
    pub(crate) passive_owner_cues: Vec<String>,
    pub(crate) non_subject_object_terms: Vec<String>,
    pub(crate) location_anchor_cues: Vec<String>,
    pub(crate) nested_location_anchor_cues: Vec<String>,
    pub(crate) nonvisual_action_context_phrases: Vec<String>,
    pub(crate) nonvisual_concept_phrases: Vec<String>,
    pub(crate) concrete_visual_subject_cues: Vec<String>,
    pub(crate) renderable_pair_cues: Vec<String>,
}

impl Default for ImageQueryPolicy {
    fn default() -> Self {
        Self {
            max_contextual_queries_per_candidate: 3,
            max_contextual_queries_per_scene: 10,
            max_query_words: 7,
            max_query_chars: 64,
            visual_action_cues: Vec::new(),
            scene_reference_cues: Vec::new(),
            anaphoric_object_terms: Vec::new(),
            location_inheritance_cues: Vec::new(),
            visual_clause_separators: Vec::new(),
            linked_clause_visual_cues: Vec::new(),
            location_preceding_subject_cues: Vec::new(),
            comparison_cues: Vec::new(),
            link_disclaimer_cues: Vec::new(),
            irrealis_visual_cues: Vec::new(),
            irrealis_visual_evidence_cues: Vec::new(),
            passive_owner_cues: Vec::new(),
            non_subject_object_terms: Vec::new(),
            location_anchor_cues: Vec::new(),
            nested_location_anchor_cues: Vec::new(),
            nonvisual_action_context_phrases: Vec::new(),
            nonvisual_concept_phrases: Vec::new(),
            concrete_visual_subject_cues: Vec::new(),
            renderable_pair_cues: Vec::new(),
        }
    }
}

pub(crate) fn image_query_policy() -> &'static ImageQueryPolicy {
    static POLICY: OnceLock<ImageQueryPolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        serde_json::from_str(include_str!("../../config/image_query_policy.json"))
            .expect("image_query_policy.json must match the semantic query policy schema")
    })
}
