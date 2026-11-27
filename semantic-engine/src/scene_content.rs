use super::*;

pub(super) fn derive_scene_content(input: SceneContentInput<'_>) -> SceneContent {
    let SceneContentInput {
        scene_id,
        source,
        language,
        sentence_spans,
        entities,
        entity_ids,
        identities,
        facts,
        temporal_facts,
        analysis,
    } = input;
    let composed_image_queries = composed_query_candidates(
        source,
        sentence_spans,
        entities,
        &analysis.numeric_facts,
        temporal_facts,
    );
    let contextual = contextual_image_query_candidates(
        language,
        source,
        sentence_spans,
        entities,
        entity_ids,
        &facts.entity_facts,
    );
    let bullets = bullet_candidates(source, sentence_spans, entities);
    let candidates = groups::build_candidates(
        groups::SceneSource {
            text: source,
            language,
        },
        entities,
        &analysis.bullets,
        groups::ChartInput {
            points: &analysis.chart_points,
            quantities: &facts.quantities,
            comparisons: &facts.quantity_comparisons,
            tokens: &analysis.tokens,
        },
        &analysis.lower_thirds,
        temporal_facts,
        identities,
    );
    let (mut visual_candidates, rejected_candidates) =
        retain_grounded_candidates(source, candidates);
    let semantic_groups = {
        let mut groups = chronological_groups(scene_id, source, sentence_spans, temporal_facts);
        groups.extend(process_groups(
            scene_id,
            source,
            sentence_spans,
            entities,
            identities,
        ));
        groups.extend(comparison_groups(
            scene_id,
            source,
            sentence_spans,
            entities,
            identities,
        ));
        groups.extend(causal_groups(
            scene_id,
            source,
            sentence_spans,
            entities,
            identities,
            &facts.relations,
        ));
        groups.extend(numbered_groups(scene_id, source, sentence_spans));
        groups.extend(ordered_sequence_groups(
            scene_id,
            source,
            sentence_spans,
            language,
        ));
        groups.extend(facts.micro_groups.clone());
        groups
    };
    // A complete dated sequence is ONE timeline: the planner can animate the
    // whole progression, and the fragment timelines of its own sentences stop
    // competing with it for the same facts.
    sequence_timeline_candidates(
        source,
        sentence_spans,
        &semantic_groups,
        temporal_facts,
        &facts.entity_facts,
        &mut visual_candidates,
    );

    let mut visual_proposals = Vec::with_capacity(visual_candidates.len());
    let mut search_feedback = Vec::new();
    for (index, candidate) in visual_candidates.into_iter().enumerate() {
        let (proposal, feedback) = to_visual_proposal(candidate, scene_id, index, source, language);
        visual_proposals.push(proposal);
        if let Some(entry) = feedback {
            search_feedback.push(entry);
        }
    }

    // The scene entity list keeps only typed entities, but the identity graph
    // must publish visual concepts too: entity-search proposals reference
    // their ids and the Go planner resolves every proposal entity id against
    // this registry. Dropping them here left validated B-roll proposals with
    // unborn identities. The entity list handed to the lookup stays unfiltered
    // on purpose: the lookup is indexed by it, so filtering first would shift
    // every id.
    let entity_candidates = typed_entity_candidates(source, sentence_spans, entities, identities);
    let important_words = important_word_candidates(&entity_candidates);
    let important_segments =
        segment_candidates(source, sentence_spans, entities, &facts.entity_facts);

    SceneContent {
        entity_candidates,
        important_words,
        important_segments,
        composed_image_queries,
        contextual,
        bullets,
        visual_proposals,
        search_feedback,
        semantic_groups,
        rejected_candidates,
    }
}
