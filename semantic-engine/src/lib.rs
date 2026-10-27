//! Unified Rust semantic engine (`semantic_manifest.v2`).
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

pub mod cache;
mod chart_rules;
pub mod consolidate;
pub mod groups;
pub mod identity_lookup;
pub mod legal;
pub mod micro;
pub mod protocol;
pub mod story;

pub(crate) mod candidates;
pub(crate) mod facts;
pub(crate) mod grouping;
pub(crate) mod proposals;

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
use visualner::{analyze_with_context, ExtractOptions, TemporalFact, VisualEntity};

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
        visualner: "visualner".to_string(),
        semantic_core: env!("CARGO_PKG_VERSION").to_string(),
        phrase_impact: "phrase_impact+editorial-selection-v3+image-query-fanout-v3+chart-gate-v1"
            .to_string(),
        lexicon: semantic_core::lexicon::LEXICON_VERSION.to_string(),
    }
}

/// Analyses one scene and records its identities in the shared registry.
/// Extractor refusals become a `rejected` scene rather than failing the script.
pub fn analyze_scene(
    scene: &SceneInput,
    language: &str,
    safety_cap: usize,
    registry: &mut IdentityRegistry,
) -> SceneManifest {
    let context = visualner::ScriptContext::default();
    analyze_scene_with_search(scene, language, safety_cap, registry, &context).0
}

/// Same analysis, also returning the asset-search entries the script-level
/// manifest aggregates. Retrieval never happens here: every entry is `pending`.
fn analyze_scene_with_search(
    scene: &SceneInput,
    language: &str,
    safety_cap: usize,
    registry: &mut IdentityRegistry,
    context: &visualner::ScriptContext,
) -> (SceneManifest, Vec<SearchFeedbackEntry>) {
    let options = ExtractOptions {
        language: language.to_string(),
        // The extractor keeps every grounded entity up to the safety cap; the
        // editorial top-N belongs to the planner.
        entity_count: safety_cap,
    };
    let mut analysis = match analyze_with_context(&scene.text, &options, context) {
        Ok(analysis) => analysis,
        Err(reason) => {
            return (
                SceneManifest {
                    scene_id: scene.scene_id.clone(),
                    status: "rejected",
                    reason: Some(reason),
                    entities: Vec::new(),
                    identities: Vec::new(),
                    relations: RelationView::default(),
                    numeric_facts: Vec::new(),
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
                Vec::new(),
            );
        }
    };

    let source = scene.text.as_str();
    // One canonical segmentation per scene. Every downstream builder consumes
    // these spans instead of re-splitting the source (the previous per-entity
    // split made analysis O(entities x text)).
    let sentence_spans = semantic_core::split_sentences(source, language, None);
    let facility_entities =
        collect_facilities(source, safety_cap.saturating_sub(analysis.entities.len()));
    for facility in facility_entities {
        if let Some(existing) = analysis
            .entities
            .iter_mut()
            .find(|entity| entity.start < facility.end && facility.start < entity.end)
        {
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
    let (entities, rejected_entities) = retain_grounded_entities(source, analysis.entities);
    let temporal_facts = retain_grounded_temporal(source, analysis.temporal_facts);

    let identities = IdentityLookup::build(registry, &scene.scene_id, &entities, context);
    let mut entity_facts = extract_entity_facts(
        source,
        &sentence_spans,
        &entities,
        &analysis.numeric_facts,
        &temporal_facts,
        &identities,
    );
    let entity_ids: Vec<String> = (0..entities.len())
        .map(|index| identities.id_of(index))
        .collect();
    entity_facts.extend(legal::extract(
        source,
        language,
        &sentence_spans,
        &entities,
        &entity_ids,
        &temporal_facts,
        &analysis.numeric_facts,
    ));
    for fact in &mut entity_facts {
        fact.finalize();
    }
    let built = micro::build(
        language,
        &scene.scene_id,
        source,
        &entity_facts,
        &entities,
        &entity_ids,
    );
    let relations = build_relations(
        source,
        language,
        &sentence_spans,
        &entities,
        &analysis.relations,
        &entity_facts,
        &identities,
    );
    let composed_image_queries = composed_query_candidates(
        source,
        &sentence_spans,
        &entities,
        &analysis.numeric_facts,
        &temporal_facts,
    );
    let contextual_image_queries = contextual_image_query_candidates(
        language,
        source,
        &sentence_spans,
        &entities,
        &entity_ids,
        &entity_facts,
    );
    let bullets = bullet_candidates(source, &sentence_spans, &entities);
    let candidates = groups::build_candidates(
        source,
        &entities,
        &analysis.bullets,
        &analysis.chart_points,
        &analysis.lower_thirds,
        &temporal_facts,
        &identities,
    );
    let (mut visual_candidates, rejected_candidates) =
        retain_grounded_candidates(source, candidates);
    let semantic_groups = {
        let mut groups =
            chronological_groups(&scene.scene_id, source, &sentence_spans, &temporal_facts);
        groups.extend(process_groups(
            &scene.scene_id,
            source,
            &sentence_spans,
            &entities,
            &identities,
        ));
        groups.extend(comparison_groups(
            &scene.scene_id,
            source,
            &sentence_spans,
            &entities,
            &identities,
        ));
        groups.extend(causal_groups(
            &scene.scene_id,
            source,
            &sentence_spans,
            &entities,
            &identities,
            &relations,
        ));
        groups.extend(numbered_groups(&scene.scene_id, source, &sentence_spans));
        groups.extend(built.groups.clone());
        groups
    };
    // A complete dated sequence is ONE timeline: the planner can animate the
    // whole progression, and the fragment timelines of its own sentences stop
    // competing with it for the same facts.
    sequence_timeline_candidates(
        source,
        &sentence_spans,
        &semantic_groups,
        &temporal_facts,
        &entity_facts,
        &mut visual_candidates,
    );

    let mut visual_proposals = Vec::with_capacity(visual_candidates.len());
    let mut search_feedback = Vec::new();
    for (index, candidate) in visual_candidates.into_iter().enumerate() {
        let (proposal, feedback) =
            to_visual_proposal(candidate, &scene.scene_id, index, source, language);
        visual_proposals.push(proposal);
        if let Some(entry) = feedback {
            search_feedback.push(entry);
        }
    }

    let entity_candidate_list = entity_candidates(source, &sentence_spans, &entities, &identities);
    let important_words = important_word_candidates(&entity_candidate_list);
    let important_segments = segment_candidates(source, &sentence_spans, &entities, &entity_facts);

    (
        SceneManifest {
            scene_id: scene.scene_id.clone(),
            status: "valid",
            reason: None,
            entities: entity_candidate_list,
            identities: registry.identities().to_vec(),
            relations,
            numeric_facts: analysis.numeric_facts,
            temporal_facts,
            entity_facts,
            important_words,
            important_segments,
            composed_image_queries,
            contextual_image_queries,
            bullets,
            short_phrases: phrase_candidates(source, &sentence_spans, &entities, 0, 18),
            long_phrases: phrase_candidates(source, &sentence_spans, &entities, 18, usize::MAX),
            quotes: quote_candidates(source, &sentence_spans),
            visual_proposals,
            semantic_groups,
            micro_summaries: built.summaries,
            rejected_items: rejected_entities + rejected_candidates,
        },
        search_feedback,
    )
}

/// Analyses every scene of a script in one call.
pub fn analyze_script(request: &ScriptRequest) -> Result<SemanticManifest, String> {
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

    let safety_cap = if request.entity_safety_cap == 0 {
        DEFAULT_ENTITY_SAFETY_CAP
    } else {
        request.entity_safety_cap
    };

    // One registry for the whole script so identity is shared across scenes.
    let mut registry = IdentityRegistry::new();
    let mut search_entries: Vec<SearchFeedbackEntry> = Vec::new();
    let mut scenes: Vec<SceneManifest> = Vec::with_capacity(request.scenes.len());
    // Script-wide name evidence: one tokenization pass over every scene, so a
    // scene that says only a first name cannot turn it into a place.
    let texts: Vec<&str> = request
        .scenes
        .iter()
        .map(|scene| scene.text.as_str())
        .collect();
    let context = visualner::script_person_tokens(&texts, language);
    for scene in &request.scenes {
        let (manifest, entries) =
            analyze_scene_with_search(scene, language, safety_cap, &mut registry, &context);
        scenes.push(manifest);
        search_entries.extend(entries);
    }
    add_cross_scene_comparison_groups(&request.scenes, &mut scenes);
    add_cross_scene_causal_groups(&request.scenes, &mut scenes);
    add_cross_scene_process_groups(&request.scenes, &mut scenes, language);

    let story_graph = story::build_story_graph(&scenes);
    let visual_coverage = story::build_visual_coverage(&scenes);
    let visual_consolidation =
        consolidate::build(language, &request.scenes, &scenes, &search_entries);

    let analyzers = analyzer_versions();
    let fingerprint = request_fingerprint(request, &analyzers);
    let editorial = request
        .editorial
        .as_ref()
        .map(|input| {
            let mut editorial_request = input.request.clone();
            if editorial_request.semantic_bullet_spans.is_none() {
                if let Some(spans) =
                    semantic_bullet_spans(&editorial_request.transcript, &input.scenes, &scenes)
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
        .transpose()?
        .map(Into::into);

    Ok(SemanticManifest {
        schema_version: SCHEMA_VERSION,
        language: language.to_string(),
        fingerprint,
        analyzers,
        scenes,
        story_graph,
        visual_coverage,
        visual_consolidation,
        search_feedback: semantic_core::SearchFeedback {
            entries: search_entries,
        },
        editorial,
    })
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

/// FNV-1a fingerprint over every input that can change a manifest, including
/// optional editorial options and caller-supplied embeddings.
pub fn request_fingerprint(
    request: &ScriptRequest,
    analyzers: &semantic_core::AnalyzerVersions,
) -> String {
    let body = serde_json::to_vec(&(
        request.language.trim(),
        &request.scenes,
        &request.editorial,
        request.entity_safety_cap,
        analyzers,
    ))
    .unwrap_or_default();
    fnv1a_hex(&body)
}

/// FNV-1a 64 over the canonical request and analyzer versions. Equal inputs
/// give equal fingerprints (idempotent replay).
pub fn fingerprint(
    language: &str,
    scenes: &[SceneInput],
    analyzers: &semantic_core::AnalyzerVersions,
) -> String {
    let body = serde_json::to_vec(&(language, scenes, analyzers)).unwrap_or_default();
    fnv1a_hex(&body)
}

/// One FNV-1a 64 implementation for every manifest fingerprint.
fn fnv1a_hex(body: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Relations split into co-mentions (grouping signal) and asserted relations.
/// `visualner` only proves co-occurrence. Asserted edges are admitted only
/// from explicit ownership/location predicates already grounded as facts.
fn build_relations(
    source: &str,
    language: &str,
    sentences: &[semantic_core::SentenceSpan],
    entities: &[VisualEntity],
    relations: &[visualner::analysis::EntityRelation],
    facts: &[EntityFact],
    identities: &IdentityLookup,
) -> RelationView {
    let mut co_mentions = Vec::new();
    for relation in relations {
        // The pair must reference two grounded, distinct entities. Entity
        // indices are validated against the grounded list; the relation is
        // admitted only when its evidence actually occurs in the source.
        if relation.source >= entities.len() || relation.target >= entities.len() {
            continue;
        }
        // Measurements (numbers, percentages, money, dates) are not things
        // that co-occur as subjects: a spelled-out number such as "due" must
        // never pair with a person. Only visual subjects form a co-mention.
        if !is_visual_subject(&entities[relation.source])
            || !is_visual_subject(&entities[relation.target])
        {
            continue;
        }
        if relation.evidence.is_empty() || !source.contains(&relation.evidence) {
            continue;
        }
        co_mentions.push(CoMention {
            left_id: identities.id_of(relation.source),
            right_id: identities.id_of(relation.target),
            sentence_index: relation.sentence_index,
            evidence: relation.evidence.clone(),
        });
    }
    let mut asserted_relations: Vec<AssertedRelation> = facts
        .iter()
        .filter_map(|fact| {
            let target_id = fact.target_entity_id.as_ref()?;
            let predicate = match fact.predicate.as_str() {
                "owned_by" => "ownership",
                "located_in" => "location",
                _ => return None,
            };
            if fact.evidence.is_empty()
                || !source
                    .get(fact.evidence_start_byte..fact.evidence_end_byte)
                    .is_some_and(|evidence| evidence == fact.evidence)
            {
                return None;
            }
            let status = match fact.status.as_str() {
                "negated" => RelationStatus::Negated,
                "uncertain" => RelationStatus::Uncertain,
                _ => RelationStatus::Asserted,
            };
            Some(AssertedRelation {
                source_id: fact.subject_entity_id.clone(),
                target_id: target_id.clone(),
                predicate: Some(predicate.to_string()),
                status,
                sentence_index: fact.sentence_index,
                evidence: fact.evidence.clone(),
            })
        })
        .collect();
    let mut seen: HashSet<(String, String, String)> = asserted_relations
        .iter()
        .filter_map(|relation| {
            Some((
                relation.source_id.clone(),
                relation.target_id.clone(),
                relation.predicate.clone()?,
            ))
        })
        .collect();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let present: Vec<usize> = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| {
                is_visual_subject(entity)
                    && entity.start >= sentence.start_byte
                    && entity.end <= sentence.end_byte
            })
            .map(|(index, _)| index)
            .collect();
        if present.len() < 2 {
            continue;
        }
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let Some(marker) =
            semantic_core::relations::explicit_relation_marker(sentence_text, language)
        else {
            continue;
        };
        let (source_index, target_index) = if matches!(
            marker.predicate.as_str(),
            "ownership" | "funding" | "causal" | "temporal"
        ) {
            let before = present
                .iter()
                .copied()
                .rfind(|index| entities[*index].end <= sentence.start_byte + marker.start_byte);
            let after = present
                .iter()
                .copied()
                .find(|index| entities[*index].start >= sentence.start_byte + marker.start_byte);
            (
                before.unwrap_or(present[0]),
                after.unwrap_or(*present.last().unwrap()),
            )
        } else {
            (present[0], *present.last().unwrap())
        };
        if source_index == target_index {
            continue;
        }
        let source_id = identities.id_of(source_index);
        let target_id = identities.id_of(target_index);
        if !seen.insert((
            source_id.clone(),
            target_id.clone(),
            marker.predicate.clone(),
        )) {
            continue;
        }
        asserted_relations.push(AssertedRelation {
            source_id,
            target_id,
            predicate: Some(marker.predicate),
            status: marker.status,
            sentence_index,
            evidence: sentence_text.to_string(),
        });
    }
    RelationView {
        co_mentions,
        asserted_relations,
    }
}

/// Words a comparison may put between a subject and its cue. "il reddito di
/// Milano rispetto a quello di Roma" keeps a pronoun between cue and subject;
/// a whole clause does not.
const COMPARISON_CUE_GAP: usize = 4;
