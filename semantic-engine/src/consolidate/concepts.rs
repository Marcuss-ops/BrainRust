//! Visual-concept and fact-card production: the concept budget, the fact
//! card recipes and the deterministic priority scoring.

use std::collections::{BTreeMap, BTreeSet};

use semantic_core::{Candidate, ConsolidatedFact, ProductionCandidate, SceneManifest, SearchQuery};

/// Maximum generic visual concepts kept per scene. Named identities are never
/// capped; this bound only stops thousands of generic nouns from becoming
/// production candidates. The complete list stays in `important_words`.
const MAX_GENERIC_CONCEPTS_PER_SCENE: usize = 4;

/// Below this deterministic specificity score a generic phrase is inventory
/// only. A scene whose nouns are all abstract gets no stock-image search
/// instead of four weak ones; the planner can still use the inventory.
const MIN_CONCEPT_PRIORITY: f32 = 0.45;

pub(super) fn push_visual_concepts(
    scenes: &[SceneManifest],
    identities: &BTreeMap<String, IdentityState>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    // The complete concept inventory stays in `important_words`; only the
    // per-scene best few enter production.
    let mut selected: BTreeSet<String> = BTreeSet::new();
    for scene in scenes {
        let named_spans: Vec<(usize, usize)> = scene
            .entities
            .iter()
            .filter(|entity| is_named_kind(&entity.kind))
            .map(|entity| (entity.start_byte, entity.end_byte))
            .collect();
        let mut concepts: Vec<&Candidate> = scene
            .entities
            .iter()
            .filter(|entity| entity.kind == "VISUAL_CONCEPT")
            // Editorial/legal abstractions are useful to semantic analysis,
            // but they are not image-search subjects. Keep this vocabulary
            // in policy data so it can evolve without changing extraction.
            .filter(|entity| !is_policy_excluded_visual_concept(&entity.text))
            .filter(|entity| {
                // A fragment of a longer named entity ("d'Assise d'Appello"
                // inside "Corte d'Assise d'Appello") is inventory, not a
                // second production candidate.
                !named_spans.iter().any(|(start, end)| {
                    *start <= entity.start_byte
                        && entity.end_byte <= *end
                        && (*start < entity.start_byte || entity.end_byte < *end)
                })
            })
            .collect();
        concepts.sort_by(|left, right| {
            concept_priority(right)
                .partial_cmp(&concept_priority(left))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.text.len().cmp(&left.text.len()))
                .then_with(|| left.start_byte.cmp(&right.start_byte))
        });
        let mut scene_selected = BTreeSet::new();
        for concept in concepts
            .into_iter()
            // A visual concept becomes a production search only when the
            // script independently returns to it. This corpus-level signal
            // is language-neutral and replaces lists of abstract nouns.
            .filter(|concept| {
                identities
                    .get(&concept.id)
                    .is_some_and(|identity| identity.mentions.len() >= 2)
            })
            .filter(|concept| concept_priority(concept) >= MIN_CONCEPT_PRIORITY)
        {
            if scene_selected.insert(concept.id.clone()) {
                selected.insert(concept.id.clone());
                if scene_selected.len() >= MAX_GENERIC_CONCEPTS_PER_SCENE {
                    break;
                }
            }
        }
    }
    for id in selected {
        let Some(identity) = identities.get(&id) else {
            continue;
        };
        if identity.kind != "VISUAL_CONCEPT" {
            continue;
        }
        let priority = concept_identity_priority(identity);
        if priority < MIN_CONCEPT_PRIORITY {
            continue;
        }
        let Some(anchor) = identity_anchor(identity) else {
            continue;
        };
        let mut scene_ids = BTreeSet::new();
        for mention in &identity.mentions {
            scene_ids.insert(mention.scene_id.clone());
        }
        candidates.push(ProductionCandidate {
            id: String::new(),
            recipe: "entity_search".to_string(),
            rank: 0,
            priority,
            scene_id: anchor.scene_id.clone(),
            scene_ids: scene_ids.into_iter().collect(),
            entity_ids: vec![identity.id.clone()],
            fact_ids: Vec::new(),
            group_ids: Vec::new(),
            title: identity.canonical.clone(),
            evidence: anchor.surface.clone(),
            start_byte: anchor.start,
            end_byte: anchor.end,
            suggested_visual: "search_asset".to_string(),
            chart_items: Vec::new(),
            temporal_surfaces: Vec::new(),
            asset_key: format!("entity:{}", identity.id),
            queries: vec![SearchQuery::literal(anchor.surface.clone())],
            requires_media: true,
            dedup_key: format!("entity:{}:search", identity.id),
            covered_fact_ids: Vec::new(),
            rationale: vec![
                "the source returns to this noun phrase, giving it script-level salience"
                    .to_string(),
                "the phrase remains a typed visual concept, not a named identity".to_string(),
            ],
        });
    }
}

pub(super) fn push_fact_cards(
    facts: &[ConsolidatedFact],
    identities: &BTreeMap<String, IdentityState>,
    representations: &mut BTreeMap<String, (String, String)>,
    candidates: &mut Vec<ProductionCandidate>,
) {
    for fact in facts {
        if representations.contains_key(&fact.id) {
            continue;
        }
        let subject = fact.subject_entity_id.clone();
        let label = identities
            .get(&subject)
            .map(|identity| identity.canonical.clone())
            .unwrap_or_else(|| subject.clone());
        let recipe = fact_card_recipe(fact);
        let key = format!("factcard:{}", fact.id);
        representations.insert(fact.id.clone(), (key.clone(), "fact_card".to_string()));
        candidates.push(ProductionCandidate {
            id: String::new(),
            recipe: "fact_card".to_string(),
            rank: 0,
            priority: (0.7 + fact.importance * 0.15).min(0.88),
            scene_id: fact.scene_ids.first().cloned().unwrap_or_default(),
            scene_ids: fact.scene_ids.clone(),
            entity_ids: vec![subject.clone()],
            fact_ids: vec![fact.id.clone()],
            group_ids: Vec::new(),
            title: format!("{label} — {}", fact_card_title(fact)),
            evidence: fact.evidence.clone(),
            start_byte: fact.evidence_start_byte,
            end_byte: fact.evidence_end_byte,
            suggested_visual: "info_card".to_string(),
            chart_items: Vec::new(),
            temporal_surfaces: Vec::new(),
            asset_key: key.clone(),
            queries: Vec::new(),
            requires_media: false,
            dedup_key: key,
            covered_fact_ids: vec![fact.id.clone()],
            rationale: vec![
                "one contextual card preserves this claim's evidence and status".to_string(),
                format!("recipe: {recipe}"),
            ],
        });
    }
}

fn fact_card_recipe(fact: &ConsolidatedFact) -> String {
    match (fact.predicate.as_str(), fact.normalized_value.as_deref()) {
        ("investigation_status", Some("under_investigation")) => {
            "investigation_status_card".to_string()
        }
        ("investigation_status", Some("denies_accusations")) => "statement_card".to_string(),
        ("legal_outcome" | "legal_finality", _) => "legal_outcome_card".to_string(),
        _ => "fact_card".to_string(),
    }
}

fn fact_card_title(fact: &ConsolidatedFact) -> &'static str {
    match (fact.predicate.as_str(), fact.normalized_value.as_deref()) {
        ("investigation_status", Some("under_investigation")) => "stato del procedimento",
        ("investigation_status", Some("denies_accusations")) => "dichiarazioni",
        ("legal_outcome" | "legal_finality", _) => "esito giudiziario",
        _ => "scheda informativa",
    }
}

fn concept_priority(candidate: &Candidate) -> f32 {
    let lower = candidate.text.to_lowercase();
    let tokens = candidate.text.split_whitespace().count();
    let chars = candidate.text.chars().count();
    let length_bonus = ((tokens.saturating_sub(1)) as f32 * 0.06).min(0.15)
        + ((chars.min(24) as f32) / 24.0) * 0.05;
    let penalty = visual_phrase_penalty(&lower);
    (0.22 + candidate.score.clamp(0.0, 1.0) * 0.4 + length_bonus - penalty).clamp(0.05, 0.95)
}

fn concept_identity_priority(identity: &IdentityState) -> f32 {
    let lower = identity.canonical.to_lowercase();
    let tokens = identity.canonical.split_whitespace().count();
    let penalty = visual_phrase_penalty(&lower) + 0.1;
    (0.25
        + identity.max_score.clamp(0.0, 1.0) * 0.35
        + (identity.mentions.len().min(5) as f32) * 0.03
        + ((tokens.saturating_sub(1)) as f32 * 0.04).min(0.1)
        - penalty)
        .clamp(0.05, 0.85)
}

/// The extractor already enforces a complete, multiword noun phrase before a
/// generic concept reaches this production selector. This final penalty uses
/// only the shared multilingual filler data and phrase shape; it does not
/// maintain a language-specific blacklist of abstract nouns or verb endings.
fn visual_phrase_penalty(lower: &str) -> f32 {
    let words: Vec<&str> = lower.split_whitespace().collect();
    let lexicon = semantic_core::lexicon::lexicon();
    let mut penalty = if lexicon.stopwords.contains_filler_phrase(lower)
        || lexicon
            .stopwords
            .stop_phrases
            .iter()
            .any(|phrase| phrase == lower)
    {
        0.35
    } else {
        0.0
    };
    if words.len() < 2 {
        penalty += 0.25;
    }
    if words.len() > 5 {
        penalty += 0.10;
    }
    penalty
}

fn is_policy_excluded_visual_concept(text: &str) -> bool {
    let normalized = text.to_lowercase();
    crate::consolidate::image_query_policy()
        .nonvisual_concept_phrases
        .iter()
        .any(|phrase| normalized.contains(&phrase.to_lowercase()))
}

use super::identities::{identity_anchor, is_named_kind};
use super::IdentityState;
