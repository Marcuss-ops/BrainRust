//! Language-agnostic subject–predicate–value resolver over Universal
//! Dependencies.
//!
//! One algorithm reads every language: a typed [`Quantity`] is linked to the
//! predicate token that consumes it through the UDPipe `head`/`dependency`
//! graph, and that predicate is then linked to its subject. The ontology
//! (`lexicon/attribute_predicates.json`) supplies only canonical predicate
//! names, the quantity dimension each accepts, the entity kinds it may bind and
//! a lemma list per language. There is no per-language branch and no second
//! quantity or identity extractor: the resolver consumes the facts the engine
//! already produced.
//!
//! Fail closed: an unresolvable or ambiguous subject produces no binding.

use std::collections::HashSet;

use semantic_core::lexicon::AttributePredicates;
use semantic_core::{Quantity, SentenceSpan};
use visualner::{AnalyzedToken, VisualEntity};

/// One grounded predicate binding: this quantity is this subject's attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedAttributeBinding {
    pub quantity_index: usize,
    pub entity_index: usize,
    /// Canonical ontology predicate (`height`, `weight`, ...).
    pub predicate: String,
}

/// Resolves every quantity whose dimension the ontology knows to the one
/// subject its dependency neighborhood supports.
pub(crate) fn resolve(
    language: &str,
    sentences: &[SentenceSpan],
    tokens: &[AnalyzedToken],
    entities: &[VisualEntity],
    quantities: &[Quantity],
    ontology: &AttributePredicates,
) -> Vec<ResolvedAttributeBinding> {
    if tokens.is_empty() || entities.is_empty() {
        return Vec::new();
    }
    let mut bindings = Vec::new();
    for (quantity_index, quantity) in quantities.iter().enumerate() {
        if quantity.value.is_none()
            || quantity.start_byte >= quantity.end_byte
            || ontology.for_dimension(&quantity.dimension).next().is_none()
        {
            continue;
        }
        let Some(sentence) = sentences.iter().find(|sentence| {
            sentence.start_byte <= quantity.start_byte && quantity.end_byte <= sentence.end_byte
        }) else {
            continue;
        };
        let sentence_tokens: Vec<&AnalyzedToken> = tokens
            .iter()
            .filter(|token| token.start >= sentence.start_byte && token.end <= sentence.end_byte)
            .collect();
        let Some(number_token) = numeric_token(&sentence_tokens, quantity) else {
            continue;
        };
        let Some(unit_head) = token_by_id(&sentence_tokens, number_token.head) else {
            continue;
        };
        let predicate = closest_predicate(
            language,
            &sentence_tokens,
            unit_head,
            &quantity.dimension,
            ontology,
        );
        let Some(predicate) = predicate else {
            continue;
        };
        let sentence_entities: Vec<usize> = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| {
                entity.start >= sentence.start_byte && entity.end <= sentence.end_byte
            })
            .map(|(index, _)| index)
            .collect();
        let entity_index = subject_for(
            &sentence_tokens,
            entities,
            &sentence_entities,
            unit_head,
            &predicate,
        );
        let Some(entity_index) = entity_index else {
            continue;
        };
        bindings.push(ResolvedAttributeBinding {
            quantity_index,
            entity_index,
            predicate: predicate.canonical.clone(),
        });
    }
    bindings
}

struct PredicateHit<'a> {
    token: &'a AnalyzedToken,
    canonical: String,
    subject_kinds: Vec<String>,
}

/// Walks the bounded dependency neighborhood of the unit head and keeps the
/// closest token whose lemma the ontology resolves for this dimension.
fn closest_predicate<'a>(
    language: &str,
    tokens: &[&'a AnalyzedToken],
    unit_head: &'a AnalyzedToken,
    dimension: &str,
    ontology: &AttributePredicates,
) -> Option<PredicateHit<'a>> {
    let mut best: Option<(usize, PredicateHit<'a>)> = None;
    for (distance, token) in neighborhood(tokens, unit_head) {
        let Some(lemma) = token.lemma.as_deref() else {
            continue;
        };
        // `flat` and `case` connect names and adpositions, never predicates.
        if matches!(
            token.dependency.as_deref(),
            Some("flat") | Some("flat:name") | Some("case")
        ) {
            continue;
        }
        let Some(definition) = ontology.resolve(language, lemma, dimension) else {
            continue;
        };
        let hit = PredicateHit {
            token,
            canonical: definition.canonical.clone(),
            subject_kinds: definition.subject_kinds.clone(),
        };
        match &best {
            Some((best_distance, best_hit)) if distance > *best_distance => {}
            Some((best_distance, best_hit)) if distance == *best_distance => {
                if best_hit.canonical != hit.canonical {
                    // Two canonical predicates at the same graph distance are
                    // an ambiguity, not permission to pick by registry order.
                    return None;
                }
            }
            _ => best = Some((distance, hit)),
        }
    }
    best.map(|(_, hit)| hit)
}

/// Tokens reachable from `unit_head` within four dependency edges, paired with
/// their shortest distance. The first occurrence of a token id wins.
fn neighborhood<'a>(
    tokens: &[&'a AnalyzedToken],
    unit_head: &'a AnalyzedToken,
) -> Vec<(usize, &'a AnalyzedToken)> {
    let mut seen: HashSet<i32> = HashSet::new();
    let mut out: Vec<(usize, &AnalyzedToken)> = Vec::new();
    let mut push = |token: &'a AnalyzedToken, distance: usize, out: &mut Vec<_>| {
        let Some(id) = token.id else {
            return;
        };
        if seen.insert(id) {
            out.push((distance, token));
        }
    };
    push(unit_head, 0, &mut out);
    // Ancestor chain, then each ancestor's direct children. This reaches a
    // predicate that governs the measure noun ("pesa 80 kg") as well as one
    // coordinated with it ("è alto ... e pesa ...").
    let mut cursor = unit_head;
    for distance in 1..=4 {
        let Some(parent) = token_by_id(tokens, cursor.head) else {
            break;
        };
        push(parent, distance, &mut out);
        for child in children(tokens, parent) {
            push(child, distance + 1, &mut out);
        }
        cursor = parent;
    }
    for child in children(tokens, unit_head) {
        push(child, 1, &mut out);
        for grandchild in children(tokens, child) {
            push(grandchild, 2, &mut out);
        }
    }
    out
}

fn numeric_token<'a>(
    tokens: &[&'a AnalyzedToken],
    quantity: &Quantity,
) -> Option<&'a AnalyzedToken> {
    tokens
        .iter()
        .copied()
        .filter(|token| {
            token.upos.as_deref() == Some("NUM")
                && token.start < quantity.end_byte
                && quantity.start_byte < token.end
        })
        .max_by_key(|token| {
            let overlap = token
                .end
                .min(quantity.end_byte)
                .saturating_sub(token.start.max(quantity.start_byte));
            (overlap, std::cmp::Reverse(token.end - token.start))
        })
}

/// The subject the dependency graph attributes the predicate to.
///
/// Order matters: a measure attached directly to a person ("Luca 175 cm")
/// wins over the predicate's subject, otherwise the predicate's own subject is
/// used, then a relative-clause head, then the unique person in the predicate
/// subtree, and finally the unique person in the sentence.
fn subject_for(
    tokens: &[&AnalyzedToken],
    entities: &[VisualEntity],
    sentence_entities: &[usize],
    unit_head: &AnalyzedToken,
    predicate: &PredicateHit<'_>,
) -> Option<usize> {
    if predicate
        .token
        .features
        .as_deref()
        .is_some_and(|features| features.contains("Number=Plur"))
    {
        return None;
    }
    if let Some(index) = local_owner(
        tokens,
        entities,
        sentence_entities,
        unit_head,
        &predicate.subject_kinds,
    ) {
        return Some(index);
    }
    let mut subjects: Vec<usize> = Vec::new();
    for child in children(tokens, predicate.token) {
        if !child
            .dependency
            .as_deref()
            .is_some_and(|dependency| dependency.starts_with("nsubj"))
        {
            continue;
        }
        // A unit noun can be the parser's syntactic subject ("188 cm" in
        // "Marco è alto 188 cm"); it is not a person and must not trip the
        // plural/coordination guard below. Only real subjects are checked.
        let Some(index) =
            entity_for_token(entities, sentence_entities, child, &predicate.subject_kinds)
        else {
            continue;
        };
        // A coordinated subject ("Marco e Luca") has more than one person;
        // binding the value to one of them would be a guess.
        if child
            .features
            .as_deref()
            .is_some_and(|features| features.contains("Number=Plur"))
            || children(tokens, child).iter().any(|nephew| {
                nephew.dependency.as_deref() == Some("conj")
                    && entity_for_token(
                        entities,
                        sentence_entities,
                        nephew,
                        &predicate.subject_kinds,
                    )
                    .is_some()
            })
        {
            return None;
        }
        if !subjects.contains(&index) {
            subjects.push(index);
        }
    }
    if subjects.len() == 1 {
        return subjects.first().copied();
    }
    if subjects.len() > 1 {
        return None;
    }
    if predicate
        .token
        .dependency
        .as_deref()
        .is_some_and(|dependency| dependency.starts_with("acl"))
    {
        if let Some(head) = token_by_id(tokens, predicate.token.head) {
            if let Some(index) =
                entity_for_token(entities, sentence_entities, head, &predicate.subject_kinds)
            {
                return Some(index);
            }
        }
    }
    if let Some(index) = unique_subtree_entity(
        tokens,
        entities,
        sentence_entities,
        predicate.token,
        &predicate.subject_kinds,
    ) {
        return Some(index);
    }
    let candidates: Vec<usize> = sentence_entities
        .iter()
        .copied()
        .filter(|index| entities[*index].kind_in(&predicate.subject_kinds))
        .collect();
    (candidates.len() == 1).then(|| candidates[0])
}

/// A person the measure noun is directly attached to: `Luca 175 cm` (nmod) or
/// `Marco Rossi 188 cm` (nmod:poss). This is the local, strongest evidence.
fn local_owner(
    tokens: &[&AnalyzedToken],
    entities: &[VisualEntity],
    sentence_entities: &[usize],
    unit_head: &AnalyzedToken,
    subject_kinds: &[String],
) -> Option<usize> {
    for token in [Some(unit_head), token_by_id(tokens, unit_head.head)]
        .into_iter()
        .flatten()
    {
        if let Some(index) = entity_for_token(entities, sentence_entities, token, subject_kinds) {
            return Some(index);
        }
    }
    None
}

fn unique_subtree_entity(
    tokens: &[&AnalyzedToken],
    entities: &[VisualEntity],
    sentence_entities: &[usize],
    root: &AnalyzedToken,
    subject_kinds: &[String],
) -> Option<usize> {
    let mut found: Vec<usize> = Vec::new();
    let mut frontier: Vec<(&AnalyzedToken, usize)> = vec![(root, 0)];
    let mut seen: HashSet<i32> = HashSet::new();
    while let Some((token, depth)) = frontier.pop() {
        if token.id.is_some_and(|id| !seen.insert(id)) || depth > 3 {
            continue;
        }
        if let Some(index) = entity_for_token(entities, sentence_entities, token, subject_kinds) {
            if !found.contains(&index) {
                found.push(index);
            }
        }
        for child in children(tokens, token) {
            frontier.push((child, depth + 1));
        }
    }
    (found.len() == 1).then(|| found[0])
}

fn entity_for_token(
    entities: &[VisualEntity],
    sentence_entities: &[usize],
    token: &AnalyzedToken,
    subject_kinds: &[String],
) -> Option<usize> {
    sentence_entities.iter().copied().find(|index| {
        let entity = &entities[*index];
        entity.start < token.end && token.start < entity.end && entity.kind_in(subject_kinds)
    })
}

fn token_by_id<'a>(tokens: &[&'a AnalyzedToken], id: Option<i32>) -> Option<&'a AnalyzedToken> {
    let id = id?;
    tokens.iter().copied().find(|token| token.id == Some(id))
}

fn children<'a>(tokens: &[&'a AnalyzedToken], parent: &'a AnalyzedToken) -> Vec<&'a AnalyzedToken> {
    let Some(parent_id) = parent.id else {
        return Vec::new();
    };
    tokens
        .iter()
        .copied()
        .filter(|token| token.head == Some(parent_id))
        .collect()
}

trait EntityKindMatch {
    fn kind_in(&self, kinds: &[String]) -> bool;
}

impl EntityKindMatch for VisualEntity {
    fn kind_in(&self, kinds: &[String]) -> bool {
        kinds
            .iter()
            .any(|kind| kind.eq_ignore_ascii_case(&self.r#type))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(
        id: i32,
        text: &str,
        upos: &str,
        dependency: &str,
        head: i32,
        lemma: &str,
        features: &str,
    ) -> AnalyzedToken {
        AnalyzedToken {
            text: text.to_string(),
            lemma: Some(lemma.to_string()),
            upos: Some(upos.to_string()),
            features: Some(features.to_string()),
            dependency: Some(dependency.to_string()),
            id: Some(id),
            head: Some(head),
            sentence_id: Some(1),
            start: 0,
            end: 0,
        }
    }

    #[test]
    fn a_registry_lemma_resolves_only_for_its_own_dimension() {
        let ontology = semantic_core::lexicon::lexicon()
            .attribute_predicates
            .clone();
        assert_eq!(
            ontology
                .resolve("it", "pesare", "mass")
                .map(|predicate| predicate.canonical.as_str()),
            Some("weight")
        );
        assert!(ontology.resolve("it", "pesare", "length").is_none());
        assert!(ontology.resolve("xx", "pesare", "mass").is_none());
    }

    #[test]
    fn coordinated_subjects_never_bind_one_value_to_one_person() {
        let ontology = semantic_core::lexicon::lexicon()
            .attribute_predicates
            .clone();
        let mut tokens = vec![
            token(1, "Marco", "PROPN", "nsubj", 4, "Marco", "Number=Sing"),
            token(2, "Luca", "PROPN", "conj", 1, "Luca", "Number=Sing"),
            token(3, "sono", "AUX", "cop", 4, "essere", "Number=Plur"),
            token(4, "alti", "ADJ", "root", 0, "alto", "Number=Plur"),
            token(5, "180", "NUM", "nummod", 6, "180", "NumType=Card"),
            token(6, "cm", "NOUN", "obl", 4, "cm", ""),
        ];
        // Byte spans are required by the resolver; place them in order.
        let mut cursor = 0usize;
        for item in &mut tokens {
            item.start = cursor;
            cursor += item.text.len() + 1;
            item.end = item.start + item.text.len();
        }
        let source_len = cursor;
        let entities = vec![
            VisualEntity {
                text: "Marco".into(),
                r#type: "PERSON".into(),
                score: 1.0,
                start: tokens[0].start,
                end: tokens[0].end,
                evidence: "Marco".into(),
            },
            VisualEntity {
                text: "Luca".into(),
                r#type: "PERSON".into(),
                score: 1.0,
                start: tokens[1].start,
                end: tokens[1].end,
                evidence: "Luca".into(),
            },
        ];
        let quantities = vec![Quantity {
            id: "q_01".into(),
            value: Some(1.8),
            raw_value: Some(180.0),
            scale: None,
            dimension: "length".into(),
            unit: "cm".into(),
            unit_surface: "cm".into(),
            qualifier: "exact".into(),
            range_id: None,
            period: None,
            subject_entity_id: None,
            predicate: None,
            start_byte: tokens[4].start,
            end_byte: tokens[4].end,
            evidence: "180".into(),
            sentence_index: 0,
            status: "asserted".into(),
        }];
        let bindings = resolve(
            "it",
            &[SentenceSpan {
                start_byte: 0,
                end_byte: source_len,
            }],
            &tokens,
            &entities,
            &quantities,
            &ontology,
        );
        assert!(bindings.is_empty(), "{bindings:?}");
    }
}
