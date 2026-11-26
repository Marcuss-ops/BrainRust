//! Unified Rust semantic engine (`semantic_manifest.v3`).
//!
//! One request carries every scene of a script. Each scene is analysed once by
//! `visualner` (entities, relations, bullets, numeric and temporal facts), then
//! projected into typed visual *proposals*. Rust decides what the text contains
//! and what could be represented; it never downloads assets, never renders, and
//! never admits a candidate that is not grounded in the source.
//!
//! Shared analysis primitives (numeric parsing, date normalization, identity
//! registry, grounding, the manifest contract) live in `semantic-core` so every
//! extractor agrees on them. Killer rule: NO EVIDENCE -> NO CANDIDATE.

#![deny(unsafe_code)]

mod ages;
pub mod cache;
mod chart_rules;
pub mod consolidate;
pub mod discourse;
mod events;
pub mod groups;
pub mod identity_lookup;
pub mod legal;
pub mod micro;
mod person_relations;
pub mod protocol;
pub mod story;

pub(crate) mod candidates;
pub(crate) mod facts;
pub(crate) mod grouping;
mod places;
pub(crate) mod proposals;
mod quantities;
mod roles;
mod routes;

use candidates::*;
use facts::*;
use grouping::*;
use proposals::*;

#[cfg(test)]
mod quote_candidates_tests;

use std::collections::{BTreeMap, BTreeSet, HashSet};

use semantic_core::grounding::{check_span, retain_grounded, Grounding};
use semantic_core::lexicon;
use semantic_core::{
    AssertedRelation, Candidate, CoMention, ComposedImageQuery, ContextualImageQuery, EntityFact,
    IdentityRegistry, ImportantSegment, QueryComponent, RelationStatus, RelationView,
    SceneManifest, SearchFeedbackEntry, SearchQuery, SemanticGroup, SemanticGroupMember,
    SemanticManifest, VisualProposal,
};
use visualner::{ExtractOptions, TemporalFact, VisualEntity};

use groups::{is_visual_entity as is_visual_subject, VisualCandidate};
use identity_lookup::IdentityLookup;

/// Wire version of the manifest produced by this engine.
pub use semantic_core::SCHEMA_VERSION;
/// Re-exported contract types so callers (and tests) build the same request
/// shape the worker deserializes.
pub use semantic_core::{SceneInput, ScriptRequest};

/// Entity safety cap used when a request does not set one. This bounds
/// pathological inputs only: it is NOT an editorial budget. The real top-N is
/// chosen by the Go planner, so a scene with eight people keeps all eight here.
pub const DEFAULT_ENTITY_SAFETY_CAP: usize = 10_000;

/// Analyzer versions recorded on every manifest for replay provenance. The
/// cache keys on these so a version bump never serves a stale result.
pub fn analyzer_versions() -> semantic_core::AnalyzerVersions {
    semantic_core::AnalyzerVersions {
        visualner: "visualner.v2.additive_types".to_string(),
        semantic_core: env!("CARGO_PKG_VERSION").to_string(),
        phrase_impact: "phrase_impact+editorial-selection-v3+image-query-fanout-v3+chart-gate-v1"
            .to_string(),
        lexicon: semantic_core::lexicon::LEXICON_VERSION.to_string(),
        entity_index: semantic_core::entity_linker::entity_index_version().to_string(),
        linker: format!(
            "{}+sha256:{}",
            semantic_core::entity_linker::linker_version(),
            semantic_core::entity_linker::linker_weights_sha256()
        ),
        numbers: "number_words.v1".to_string(),
        dates: "relative_time.v1".to_string(),
        quantities: "typed_quantities.v3.chart_meta_series".to_string(),
        roles: "person_roles.v4.all_11".to_string(),
        age_facts: "person_age_facts.v2.localized_dated_age".to_string(),
        person_relations: "person_relations.seed11.v2".to_string(),
        places: "place_index.v3.multilingual_labels".to_string(),
        routes: "place_routes.v3.cross_scene_hops".to_string(),
        events: "event_frames.v4.documentary_taxonomy".to_string(),
        discourse: "discourse.v7.role_nominal_basis".to_string(),
    }
}

/// Cache identity and request fingerprint live with the cache that owns them;
/// re-exported here because they are part of this crate's internal API.
pub(crate) use cache::cache_identity;
pub(crate) use cache::request_body;
pub use cache::request_fingerprint;

/// Analyses one scene and records its identities in the shared registry.
/// Extractor refusals become a `rejected` scene rather than failing the script.
pub fn analyze_scene(
    scene: &SceneInput,
    language: &str,
    safety_cap: usize,
    registry: &mut IdentityRegistry,
) -> SceneManifest {
    let context = visualner::ScriptContext::default();
    analyze_scene_with_search(scene, language, safety_cap, registry, &context, None).manifest
}

/// A scene the extractor refused, materialized as the same empty manifest a
/// successful path would produce. Keeping the literal out of the orchestration
/// makes the rejection contract explicit and testable on its own.
fn rejected_scene(scene: &SceneInput, reason: String) -> SceneAnalysis {
    SceneAnalysis {
        manifest: SceneManifest {
            scene_id: scene.scene_id.clone(),
            status: "rejected".to_string(),
            reason: Some(reason),
            entities: Vec::new(),
            identities: Vec::new(),
            person_roles: Vec::new(),
            age_facts: Vec::new(),
            person_relations: Vec::new(),
            places: Vec::new(),
            routes: Vec::new(),
            relations: RelationView::default(),
            numeric_facts: Vec::new(),
            quantities: Vec::new(),
            quantity_comparisons: Vec::new(),
            temporal_facts: Vec::new(),
            entity_facts: Vec::new(),
            bullets: Vec::new(),
            short_phrases: Vec::new(),
            long_phrases: Vec::new(),
            quotes: Vec::new(),
            important_words: Vec::new(),
            important_segments: Vec::new(),
            composed_image_queries: Vec::new(),
            contextual_image_queries: Vec::new(),
            visual_proposals: Vec::new(),
            semantic_groups: Vec::new(),
            micro_summaries: Vec::new(),
            rejected_items: 0,
        },
        search_feedback: Vec::new(),
        query_seeds: Vec::new(),
        tokens: Vec::new(),
    }
}

/// Collapses a facility fragment into the maximal facility span that contains
/// it, then removes exact duplicate facility spans. Other entity types may
/// legitimately nest (an acronym inside an organization, a place inside an
/// organization), so only facilities are collapsed.
fn collapse_facility_fragments(entities: &mut Vec<VisualEntity>) {
    // Sort facilities by start ascending and end descending: the first span at
    // a given start is the maximal one, and every later facility whose end is
    // inside the running maximum is nested. The former nested scan was O(n²)
    // on the entity list (100M comparisons at the 10k safety cap).
    let mut facilities: Vec<usize> = entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity.r#type == "FACILITY")
        .map(|(index, _)| index)
        .collect();
    facilities.sort_by(|left, right| {
        entities[*left]
            .start
            .cmp(&entities[*right].start)
            .then(entities[*right].end.cmp(&entities[*left].end))
            .then(left.cmp(right))
    });
    let mut keep = vec![true; entities.len()];
    let mut running_end: Option<usize> = None;
    for index in facilities {
        let entity = &entities[index];
        if running_end.is_some_and(|end| entity.end <= end) {
            keep[index] = false;
        } else {
            running_end = Some(entity.end);
        }
    }
    let mut keep = keep.into_iter();
    entities.retain(|_| keep.next().unwrap_or(true));
    let mut seen_facility_spans = HashSet::new();
    entities.retain(|entity| {
        entity.r#type != "FACILITY" || seen_facility_spans.insert((entity.start, entity.end))
    });
}

/// Facility/legal augmentation, surname promotion, grounding retention and the
/// facility collapse, in the order the manifest contract requires. The main
/// analyzer treats this as one stage instead of carrying a hundred lines of
/// mutable entity bookkeeping inline.
fn prepare_entities(
    source: &str,
    language: &str,
    safety_cap: usize,
    context: &visualner::ScriptContext,
    registry: &IdentityRegistry,
    analysis: &mut visualner::Analysis,
) -> (Vec<VisualEntity>, usize) {
    let facility_entities =
        collect_facilities(source, safety_cap.saturating_sub(analysis.entities.len()));
    for facility in facility_entities {
        if let Some(existing) = analysis
            .entities
            .iter_mut()
            .find(|entity| entity.start < facility.end && facility.start < entity.end)
        {
            // The explicit facility cue names the venue, while the overlapping
            // extractor entity may be only its proper-name tail ("Стадион
            // Северный" vs "Северный"). Keep the maximal grounded span so the
            // query does not lose the facility head.
            if facility.start <= existing.start
                && facility.end >= existing.end
                && (facility.start < existing.start || existing.end < facility.end)
            {
                existing.text = facility.text.clone();
                existing.start = facility.start;
                existing.end = facility.end;
                existing.evidence = existing.text.clone();
            }
            existing.r#type = "FACILITY".to_string();
            existing.score = existing.score.max(facility.score);
        } else {
            analysis.entities.push(facility);
        }
    }
    collect_explicit_owner_entities(source, &mut analysis.entities, safety_cap);
    // Courts, prosecutors and procedural roles are not typed reliably by the
    // general tokenizer; a curated grounded span makes them usable as fact
    // subjects without inventing any fact.
    let remaining = safety_cap.saturating_sub(analysis.entities.len());
    for legal_entity in legal::collect_entities(source, language)
        .into_iter()
        .take(remaining)
    {
        // Upgrade only the entity that starts with the legal head
        // ("Procura" -> "Procura di Pavia"). Any-overlap matching used to
        // replace a nested location ("Pavia") when the gazetteer typed it,
        // erasing a valid LOCATION from the manifest.
        if let Some(existing) = analysis
            .entities
            .iter_mut()
            .find(|entity| entity.start == legal_entity.start && entity.end <= legal_entity.end)
        {
            if legal_entity.end - legal_entity.start > existing.end - existing.start {
                existing.text = legal_entity.text;
                existing.r#type = legal_entity.r#type;
                existing.start = legal_entity.start;
                existing.end = legal_entity.end;
                existing.evidence = existing.text.clone();
                existing.score = existing.score.max(legal_entity.score);
            } else if existing.r#type == "VISUAL_CONCEPT" {
                existing.r#type = legal_entity.r#type;
            }
        } else {
            analysis.entities.push(legal_entity);
        }
    }
    let known_person_surnames: HashSet<String> = analysis
        .entities
        .iter()
        .filter(|entity| entity.r#type == "PERSON")
        .filter_map(|entity| {
            let words: Vec<&str> = entity.text.split_whitespace().collect();
            (words.len() >= 2).then(|| words.last().unwrap().to_lowercase())
        })
        .chain(
            registry
                .identities()
                .iter()
                .filter(|identity| identity.kind == "PERSON")
                .filter_map(|identity| {
                    let words: Vec<&str> = identity.canonical_surface.split_whitespace().collect();
                    (words.len() >= 2).then(|| words.last().unwrap().to_lowercase())
                }),
        )
        .collect();
    promote_known_surnames(
        source,
        &mut analysis.entities,
        context,
        &known_person_surnames,
        language,
    );
    let (mut entities, rejected_entities) =
        retain_grounded_entities(source, std::mem::take(&mut analysis.entities));
    collapse_facility_fragments(&mut entities);
    (entities, rejected_entities)
}

/// One scene's analysis output. The former positional 4-tuple made every
/// caller depend on field order and hid what the scene actually produced; the
/// named fields keep the orchestration readable and let a new output be added
/// without touching the call sites.
struct SceneAnalysis {
    manifest: SceneManifest,
    search_feedback: Vec<SearchFeedbackEntry>,
    query_seeds: Vec<ContextualImageQuery>,
    tokens: Vec<visualner::AnalyzedToken>,
}

mod scene_facts;
use scene_facts::*;
/// Analyses every scene of a script in one call.
pub fn analyze_script(request: &ScriptRequest) -> Result<SemanticManifest, String> {
    let language = request.language.trim();
    let safety_cap = validate_script_request(request)?;

    // One registry for the whole script so identity is shared across scenes.
    let mut registry = IdentityRegistry::new();
    let ScriptSceneBatch {
        mut scenes,
        mut event_frames,
        search_entries,
        query_seeds: scene_query_seeds,
    } = analyze_scenes(request, language, safety_cap, &mut registry);

    link_event_source_facts(&mut event_frames, &scenes);
    let discourse = discourse::build(language, &request.scenes, &scenes);
    event_frames = events::merge_cross_scene(event_frames, &discourse.threads);
    let cross_scene_routes = routes::merge_cross_scene_routes(&scenes);
    add_cross_scene_comparison_groups(&request.scenes, &mut scenes);
    add_cross_scene_causal_groups(&request.scenes, &mut scenes);
    add_cross_scene_process_groups(&request.scenes, &mut scenes, language);

    // The v3 entity projection runs after every scene has registered, so the
    // linker sees the complete script context and every cross-scene mention.
    let scene_texts: Vec<(&str, &str)> = request
        .scenes
        .iter()
        .map(|scene| (scene.scene_id.as_str(), scene.text.as_str()))
        .collect();
    let entities = semantic_core::entity_linker::link_all_with_threads(
        language,
        &scene_texts,
        registry.identities(),
        &discourse.threads,
    );

    let story_graph = story::build_story_graph(&scenes);
    let visual_coverage = story::build_visual_coverage(&scenes);
    let visual_consolidation = consolidate::build(
        language,
        &request.scenes,
        &scenes,
        &event_frames,
        &search_entries,
        &scene_query_seeds,
    );

    let analyzers = analyzer_versions();
    let fingerprint = request_fingerprint(request, &analyzers);
    let editorial = build_editorial(request, &scenes)?;

    Ok(SemanticManifest {
        schema_version: SCHEMA_VERSION.to_string(),
        language: language.to_string(),
        fingerprint,
        analyzers,
        scenes,
        cross_scene_routes,
        events: event_frames,
        entities,
        story_graph,
        visual_coverage,
        discourse,
        visual_consolidation,
        search_feedback: semantic_core::SearchFeedback {
            entries: search_entries,
        },
        editorial,
    })
}

/// Validate the script envelope and return the effective entity safety cap.
fn validate_script_request(request: &ScriptRequest) -> Result<usize, String> {
    let language = request.language.trim();
    if language.is_empty() {
        return Err("language is required".to_string());
    }
    if request.scenes.is_empty() {
        return Err("scenes must not be empty".to_string());
    }
    let mut ids = BTreeSet::new();
    for scene in &request.scenes {
        if !ids.insert(scene.scene_id.as_str()) {
            return Err(format!("duplicate scene_id: {}", scene.scene_id));
        }
    }
    Ok(if request.entity_safety_cap == 0 {
        DEFAULT_ENTITY_SAFETY_CAP
    } else {
        request.entity_safety_cap
    })
}

struct ScriptSceneBatch {
    scenes: Vec<SceneManifest>,
    event_frames: Vec<semantic_core::EventFrame>,
    search_entries: Vec<SearchFeedbackEntry>,
    query_seeds: Vec<(String, Vec<ContextualImageQuery>)>,
}

/// Analyse every scene once, feeding the shared identity registry and keeping
/// the per-scene event and query outputs in source order.
fn analyze_scenes(
    request: &ScriptRequest,
    language: &str,
    safety_cap: usize,
    registry: &mut IdentityRegistry,
) -> ScriptSceneBatch {
    let texts: Vec<&str> = request
        .scenes
        .iter()
        .map(|scene| scene.text.as_str())
        .collect();
    let context = visualner::script_person_tokens(&texts, language);
    let mut scenes: Vec<SceneManifest> = Vec::with_capacity(request.scenes.len());
    let mut event_frames = Vec::new();
    let mut search_entries: Vec<SearchFeedbackEntry> = Vec::new();
    // Visual-context query seeds for the production planner. They are not
    // part of the manifest contract and never leave this call.
    let mut scene_query_seeds: Vec<(String, Vec<semantic_core::ContextualImageQuery>)> =
        Vec::with_capacity(request.scenes.len());
    for (scene_index, scene) in request.scenes.iter().enumerate() {
        let analyzed = analyze_scene_with_search(
            scene,
            language,
            safety_cap,
            registry,
            &context,
            request.reference_date.as_deref(),
        );
        let remaining_event_capacity = events::MAX_EVENT_FRAMES.saturating_sub(event_frames.len());
        event_frames.extend(events::extract_bounded_with_tokens(
            events::SceneEventInput {
                scene_index,
                scene_id: &scene.scene_id,
                source: &scene.text,
                language,
                candidates: &analyzed.manifest.entities,
                temporal_facts: &analyzed.manifest.temporal_facts,
                places: &analyzed.manifest.places,
            },
            &analyzed.tokens,
            remaining_event_capacity,
        ));
        scene_query_seeds.push((analyzed.manifest.scene_id.clone(), analyzed.query_seeds));
        scenes.push(analyzed.manifest);
        search_entries.extend(analyzed.search_feedback);
    }
    ScriptSceneBatch {
        scenes,
        event_frames,
        search_entries,
        query_seeds: scene_query_seeds,
    }
}

/// Link each event frame to the entity facts that share a participant and
/// overlap its evidence in the same scene.
fn link_event_source_facts(
    event_frames: &mut [semantic_core::EventFrame],
    scenes: &[SceneManifest],
) {
    for event in event_frames {
        let participant_ids: HashSet<&str> = event
            .participants
            .iter()
            .map(|participant| participant.entity_id.as_str())
            .collect();
        for scene in scenes {
            if !event.scene_ids.iter().any(|id| id == &scene.scene_id) {
                continue;
            }
            for fact in &scene.entity_facts {
                let has_event_participant = participant_ids
                    .contains(fact.subject_entity_id.as_str())
                    || fact
                        .target_entity_id
                        .as_deref()
                        .is_some_and(|id| participant_ids.contains(id));
                let evidence_overlaps = event.evidence.iter().any(|evidence| {
                    evidence.scene_id == scene.scene_id
                        && fact.evidence_start_byte >= evidence.start_byte
                        && fact.evidence_end_byte <= evidence.end_byte
                });
                if has_event_participant && evidence_overlaps {
                    event.source_fact_ids.push(fact.id.clone());
                }
            }
        }
        event.source_fact_ids.sort();
        event.source_fact_ids.dedup();
    }
}

/// Run the optional editorial planner after the scenes are complete, enriching
/// the request with grounded bullet spans when the caller did not supply them.
fn build_editorial(
    request: &ScriptRequest,
    scenes: &[SceneManifest],
) -> Result<Option<semantic_core::manifest::EditorialManifest>, String> {
    request
        .editorial
        .as_ref()
        .map(|input| {
            let mut editorial_request = input.request.clone();
            if editorial_request.semantic_bullet_spans.is_none() {
                if let Some(spans) =
                    semantic_bullet_spans(&editorial_request.transcript, &input.scenes, scenes)
                {
                    editorial_request.semantic_bullet_spans = Some(spans);
                }
            }
            semantic_core::phrase_impact::run_with_scene_inputs(
                editorial_request,
                &input.scenes,
                &input.chapter_options,
            )
        })
        .transpose()
        .map(|editorial| editorial.map(Into::into))
}

/// Maps scene-local semantic bullet candidates to exact transcript spans.
/// Like the Go implementation before this cutover, the semantic candidate pool
/// is activated only when every scene has a verified global slice; otherwise
/// Phrase Impact retains its historical candidate behavior.
fn semantic_bullet_spans(
    transcript: &str,
    editorial_scenes: &[semantic_core::phrase_impact::SceneInput],
    manifests: &[SceneManifest],
) -> Option<Vec<semantic_core::phrase_impact::SemanticBulletSpan>> {
    use semantic_core::phrase_impact::SemanticBulletSpan;

    let by_id: BTreeMap<_, _> = manifests
        .iter()
        .map(|manifest| (manifest.scene_id.as_str(), manifest))
        .collect();
    for scene in editorial_scenes {
        let start = scene.start_byte?;
        let end = scene.end_byte?;
        if end <= start || transcript.get(start..end)? != scene.text {
            return None;
        }
    }
    let mut spans = Vec::new();
    for scene in editorial_scenes {
        let start = scene.start_byte?;
        let manifest = by_id.get(scene.scene_id.as_str())?;
        for candidate in &manifest.bullets {
            let Some(local) = scene.text.get(candidate.start_byte..candidate.end_byte) else {
                continue;
            };
            if local != candidate.text {
                continue;
            }
            let global_start = start + candidate.start_byte;
            let global_end = start + candidate.end_byte;
            if transcript.get(global_start..global_end) != Some(candidate.text.as_str()) {
                continue;
            }
            spans.push(SemanticBulletSpan {
                start_byte: global_start,
                end_byte: global_end,
                text: candidate.text.clone(),
                score: Some(candidate.score),
            });
        }
    }
    Some(spans)
}

#[path = "relations.rs"]
mod relations;
use relations::*;
#[path = "scene_content.rs"]
mod scene_content;
use scene_content::*;

/// Words a comparison may put between a subject and its cue. "il reddito di
/// Milano rispetto a quello di Roma" keeps a pronoun between cue and subject;
/// a whole clause does not.
const COMPARISON_CUE_GAP: usize = 4;
