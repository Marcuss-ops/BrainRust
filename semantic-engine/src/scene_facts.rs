use super::*;

/// Everything the fact builders derive from one grounded scene. Grouping the
/// outputs keeps the stage boundary explicit instead of threading a dozen
/// `let` bindings through the orchestrator.
pub(super) struct SceneFacts {
    pub(super) entity_facts: Vec<EntityFact>,
    pub(super) person_roles: Vec<semantic_core::PersonRole>,
    pub(super) age_facts: Vec<semantic_core::PersonAgeFact>,
    pub(super) person_relations: Vec<semantic_core::PersonRelation>,
    pub(super) places: Vec<semantic_core::Place>,
    pub(super) routes: Vec<semantic_core::Route>,
    pub(super) quantities: Vec<semantic_core::quantity::Quantity>,
    pub(super) quantity_comparisons: Vec<semantic_core::quantity::QuantityComparison>,
    pub(super) relations: RelationView,
    pub(super) micro_groups: Vec<SemanticGroup>,
    pub(super) micro_summaries: Vec<semantic_core::MicroSummary>,
}

/// Inputs shared by the fact-derivation stage. Borrowing one struct keeps the
/// stage signature at one argument and makes the data flow visible.
pub(super) struct SceneDerive<'a> {
    scene_id: &'a str,
    source: &'a str,
    language: &'a str,
    sentence_spans: &'a [semantic_core::SentenceSpan],
    entities: &'a [VisualEntity],
    entity_ids: &'a [String],
    identities: &'a IdentityLookup,
    analysis: &'a visualner::Analysis,
    temporal_facts: &'a [TemporalFact],
}

/// Roles, ages, relations, places, routes, legal facts, quantities and micro
/// summaries: the grounded facts a scene contributes before any overlay is
/// chosen. This was half of `analyze_scene_with_search`'s body.
pub(super) fn derive_scene_facts(input: SceneDerive<'_>) -> SceneFacts {
    let SceneDerive {
        scene_id,
        source,
        language,
        sentence_spans,
        entities,
        entity_ids,
        identities,
        analysis,
        temporal_facts,
    } = input;
    let mut entity_facts = extract_entity_facts(
        source,
        sentence_spans,
        entities,
        &analysis.numeric_facts,
        temporal_facts,
        identities,
    );
    let person_roles = roles::extract(
        source,
        language,
        sentence_spans,
        entities,
        entity_ids,
        temporal_facts,
    );
    let age_facts = ages::extract(
        source,
        language,
        sentence_spans,
        entities,
        entity_ids,
        &analysis.numeric_facts,
        temporal_facts,
    );
    let person_relations =
        person_relations::extract(source, language, sentence_spans, entities, entity_ids);
    let places = places::extract(source, language, sentence_spans, entities, entity_ids);
    let routes = routes::extract_with_people(
        source,
        language,
        sentence_spans,
        &places,
        entities,
        entity_ids,
    );
    entity_facts.extend(legal::extract(
        source,
        language,
        sentence_spans,
        entities,
        entity_ids,
        temporal_facts,
        &analysis.numeric_facts,
    ));
    for fact in &mut entity_facts {
        fact.finalize();
    }
    let mut quantities =
        quantities::extract(source, language, &analysis.numeric_facts, temporal_facts);
    quantities::attach_grounded_facts(&mut quantities, &entity_facts);
    let quantity_comparisons = quantities::comparisons(source, language, &quantities);
    let built = micro::build(
        language,
        scene_id,
        source,
        &entity_facts,
        entities,
        entity_ids,
    );
    let relations = build_relations(
        source,
        language,
        sentence_spans,
        entities,
        &analysis.relations,
        &entity_facts,
        identities,
    );
    SceneFacts {
        entity_facts,
        person_roles,
        age_facts,
        person_relations,
        places,
        routes,
        quantities,
        quantity_comparisons,
        relations,
        micro_groups: built.groups,
        micro_summaries: built.summaries,
    }
}

/// Inputs for the visual-candidate stage.
pub(super) struct SceneContentInput<'a> {
    pub(crate) scene_id: &'a str,
    pub(crate) source: &'a str,
    pub(crate) language: &'a str,
    pub(crate) sentence_spans: &'a [semantic_core::SentenceSpan],
    pub(crate) entities: &'a [VisualEntity],
    pub(crate) entity_ids: &'a [String],
    pub(crate) identities: &'a IdentityLookup,
    pub(crate) facts: &'a SceneFacts,
    pub(crate) temporal_facts: &'a [TemporalFact],
    pub(crate) analysis: &'a visualner::Analysis,
}

/// Grounded visual candidates, grouped structures and search proposals. Every
/// candidate is still retained only after its grounding check, exactly as
/// before the split; this stage owns the *selection* half of the old body.
pub(super) struct SceneContent {
    pub(crate) entity_candidates: Vec<Candidate>,
    pub(crate) important_words: Vec<Candidate>,
    pub(crate) important_segments: Vec<ImportantSegment>,
    pub(crate) composed_image_queries: Vec<ComposedImageQuery>,
    pub(crate) contextual: candidates::contextual::ContextualQueries,
    pub(crate) bullets: Vec<Candidate>,
    pub(crate) visual_proposals: Vec<VisualProposal>,
    pub(crate) search_feedback: Vec<SearchFeedbackEntry>,
    pub(crate) semantic_groups: Vec<SemanticGroup>,
    pub(crate) rejected_candidates: usize,
}

/// One scene's phase-1 outcome. A refusal is kept as a reason + id so the
/// ordered finalize phase can materialize the same rejected manifest without
/// re-running the extractor.
pub(super) enum LocalSceneResult {
    Draft(Box<crate::cache::scene::LocalSceneDraft>),
    Rejected { scene_id: String, reason: String },
}

/// Phase 1: pure scene-local analysis. No identity registry, no cross-scene
/// state, so scenes can run in parallel and a cache hit is safe. Everything
/// that depends on earlier scenes is deferred to [`finalize_scene_with_search`].
pub(super) fn analyze_local_scene(
    scene: &SceneInput,
    language: &str,
    safety_cap: usize,
    context: &visualner::ScriptContext,
    reference_date: Option<&str>,
    analyzers: &semantic_core::AnalyzerVersions,
    cache: Option<&crate::cache::scene::SceneCache>,
) -> LocalSceneResult {
    let key = cache.map(|_| crate::cache::scene::SceneCache::key(&scene.text, language, analyzers));
    if let (Some(cache), Some(key)) = (cache, key.as_deref()) {
        if let Some(mut draft) = cache.load(key, language) {
            // Scene ids are positional and not part of the cache key.
            draft.scene_id = scene.scene_id.clone();
            return LocalSceneResult::Draft(Box::new(draft));
        }
    }
    let options = ExtractOptions {
        language: language.to_string(),
        // The extractor keeps every grounded entity up to the safety cap; the
        // editorial top-N belongs to the planner.
        entity_count: safety_cap,
    };
    let mut analysis = match visualner::analyze_with_context_preserving_mentions_and_reference(
        &scene.text,
        &options,
        context,
        reference_date,
    ) {
        Ok(analysis) => analysis,
        Err(reason) => {
            return LocalSceneResult::Rejected {
                scene_id: scene.scene_id.clone(),
                reason,
            }
        }
    };

    // One canonical segmentation per scene. Every downstream builder consumes
    // these spans instead of re-splitting the source (the previous per-entity
    // split made analysis O(entities x text)).
    let sentence_spans = semantic_core::split_sentences(&scene.text, language, None);
    let temporal_facts =
        retain_grounded_temporal(&scene.text, std::mem::take(&mut analysis.temporal_facts));
    let draft = crate::cache::scene::LocalSceneDraft {
        scene_id: scene.scene_id.clone(),
        analysis,
        temporal_facts,
        sentence_spans,
    };
    if let (Some(cache), Some(key)) = (cache, key.as_deref()) {
        cache.store(key, language, &draft);
    }
    LocalSceneResult::Draft(Box::new(draft))
}

/// Phase 2: ordered, deterministic finalization. Registers this scene's
/// identities in the shared registry and derives every cross-scene-aware
/// fact, then returns the same asset-search entries and production seeds as
/// before. Retrieval never happens here: every entry is `pending`.
pub(super) fn finalize_scene_with_search(
    draft: crate::cache::scene::LocalSceneDraft,
    scene: &SceneInput,
    language: &str,
    safety_cap: usize,
    registry: &mut IdentityRegistry,
    context: &visualner::ScriptContext,
) -> SceneAnalysis {
    let crate::cache::scene::LocalSceneDraft {
        mut analysis,
        temporal_facts,
        sentence_spans,
        ..
    } = draft;
    let source = scene.text.as_str();
    let (entities, rejected_entities) = prepare_entities(
        source,
        language,
        safety_cap,
        context,
        registry,
        &mut analysis,
    );
    let identities = IdentityLookup::build(registry, &scene.scene_id, &entities, context, language);
    let entity_ids: Vec<String> = (0..entities.len())
        .map(|index| identities.id_of(index))
        .collect();
    let facts = derive_scene_facts(SceneDerive {
        scene_id: &scene.scene_id,
        source,
        language,
        sentence_spans: &sentence_spans,
        entities: &entities,
        entity_ids: &entity_ids,
        identities: &identities,
        analysis: &analysis,
        temporal_facts: &temporal_facts,
    });
    let content = derive_scene_content(SceneContentInput {
        scene_id: &scene.scene_id,
        source,
        language,
        sentence_spans: &sentence_spans,
        entities: &entities,
        entity_ids: &entity_ids,
        identities: &identities,
        facts: &facts,
        temporal_facts: &temporal_facts,
        analysis: &analysis,
    });

    SceneAnalysis {
        manifest: SceneManifest {
            scene_id: scene.scene_id.clone(),
            status: "valid".to_string(),
            reason: None,
            entities: content.entity_candidates,
            identities: registry.identities().to_vec(),
            person_roles: facts.person_roles,
            age_facts: facts.age_facts,
            person_relations: facts.person_relations,
            places: facts.places,
            routes: facts.routes,
            relations: facts.relations,
            numeric_facts: analysis.numeric_facts,
            quantities: facts.quantities,
            quantity_comparisons: facts.quantity_comparisons,
            temporal_facts,
            entity_facts: facts.entity_facts,
            important_words: content.important_words,
            important_segments: content.important_segments,
            composed_image_queries: content.composed_image_queries,
            contextual_image_queries: content.contextual.manifest_queries,
            bullets: content.bullets,
            short_phrases: phrase_candidates(source, &sentence_spans, &entities, 0, 18),
            long_phrases: phrase_candidates(source, &sentence_spans, &entities, 18, usize::MAX),
            quotes: quote_candidates(source, &sentence_spans),
            visual_proposals: content.visual_proposals,
            semantic_groups: content.semantic_groups,
            micro_summaries: facts.micro_summaries,
            rejected_items: rejected_entities + content.rejected_candidates,
        },
        search_feedback: content.search_feedback,
        query_seeds: content.contextual.production_seeds,
        tokens: analysis.tokens,
    }
}

/// Single-scene compatibility path used by `analyze_scene`. Production batches
/// use the two phases directly so phase 1 can run in parallel and be cached.
pub(super) fn analyze_scene_with_search(
    scene: &SceneInput,
    language: &str,
    safety_cap: usize,
    registry: &mut IdentityRegistry,
    context: &visualner::ScriptContext,
    reference_date: Option<&str>,
) -> SceneAnalysis {
    let analyzers = crate::analyzer_versions();
    match analyze_local_scene(
        scene,
        language,
        safety_cap,
        context,
        reference_date,
        &analyzers,
        None,
    ) {
        LocalSceneResult::Rejected { scene_id, reason } => rejected_scene(&scene_id, reason),
        LocalSceneResult::Draft(draft) => {
            finalize_scene_with_search(*draft, scene, language, safety_cap, registry, context)
        }
    }
}
