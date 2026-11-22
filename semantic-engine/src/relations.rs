use super::*;

/// Relations split into co-mentions (grouping signal) and asserted relations.
/// `visualner` only proves co-occurrence. Asserted edges are admitted only
/// from explicit ownership/location predicates already grounded as facts.
pub(super) fn build_relations(
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
