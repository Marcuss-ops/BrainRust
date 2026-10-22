use crate::types::VisualEntity;

use super::stopwords::is_generic_phrase;

pub(super) fn named_entity_rank(entity: &VisualEntity) -> u8 {
    match entity.r#type.as_str() {
        "PERSON" | "BRAND" => 0,
        "ORGANIZATION" | "LOCATION" | "EVENT" | "WORK" | "PRODUCT" | "OBJECT" => 1,
        "DATE" | "MONEY" | "NUMBER" | "PERCENT" => 2,
        _ => 3,
    }
}

pub(super) fn entity_identity_key(entity: &VisualEntity) -> String {
    let mut value = entity.text.to_lowercase();
    for suffix in ["'s", "’s"] {
        if value.ends_with(suffix) {
            value.truncate(value.len() - suffix.len());
            break;
        }
    }
    format!("{}:{}", entity.r#type, value.trim())
}

/// Apply deterministic visualness scoring using structural signals only:
/// semantic type and phrase length, with the boilerplate penalty. Domain
/// vocabularies were deliberately removed from the scorer — a curated list
/// of dish names and ingredients made the engine "know" one demo domain and
/// silently rank every other domain blind. Visual hints belong to the
/// retrieval layer (gazetteer/stock metadata), never to this scorer.
pub(super) fn score_entity(mut entity: VisualEntity) -> VisualEntity {
    let lower = entity.text.to_lowercase();
    let word_count = lower.split_whitespace().count().max(1);
    let mut score: f32 = 0.30 + 0.20 * (word_count as f32 - 1.0);
    if entity.r#type == "OBJECT" {
        score += 0.15;
    }
    if is_generic_phrase(&lower) {
        score -= 0.25;
    }
    if entity.r#type == "PERSON" || entity.r#type == "BRAND" {
        score += 0.50;
    }
    entity.score = score.clamp(0.0, 1.0);
    entity
}
