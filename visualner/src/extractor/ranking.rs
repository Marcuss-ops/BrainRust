use crate::types::VisualEntity;

pub(super) fn named_entity_rank(entity: &VisualEntity) -> u8 {
    match entity.r#type.as_str() {
        "PERSON" | "BRAND" => 0,
        "ORGANIZATION" | "LOCATION" | "EVENT" | "WORK" | "PRODUCT" => 1,
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

/// Apply deterministic visualness scoring using the curated V1 rules.
pub(super) fn score_entity(mut entity: VisualEntity) -> VisualEntity {
    let lower = entity.text.to_lowercase();
    let word_count = lower.split_whitespace().count().max(1);
    let mut score: f32 = 0.30 + 0.20 * (word_count as f32 - 1.0);
    if is_visual_object_hint(&lower) {
        score += 0.15;
    }
    if VISUAL_OBJECT_KEYWORDS.iter().any(|keyword| lower.contains(keyword)) {
        score += 0.05;
    }
    if is_generic_phrase(&lower) {
        score -= 0.25;
    }
    if is_subject_phrase(&lower) {
        score -= 0.10;
    }
    if entity.r#type == "PERSON" || entity.r#type == "BRAND" {
        score += 0.50;
    }
    entity.score = score.clamp(0.0, 1.0);
    entity
}

const VISUAL_OBJECT_HINTS: &[&str] = &[
    "greek salad",
    "feta cheese",
    "olive oil",
    "lemon juice",
    "grilled sardines",
    "seafood paella",
];

const VISUAL_OBJECT_KEYWORDS: &[&str] = &[
    "cheese", "tomato", "tomatoes", "olive", "olives", "feta", "salad", "hummus",
    "chickpea", "chickpeas", "tahini", "lemon", "herbs", "sardine", "sardines",
    "shakshuka", "egg", "eggs", "pepper", "peppers", "paella", "shrimp", "mussel",
    "mussels", "rice", "oil",
];

const GENERIC_PHRASES: &[&str] = &[
    "imagine",
    "ready",
    "world",
    "discover",
    "vibrant",
    "cuisine",
    "mediterranean",
    "get ready",
    "let us",
    "imagine the",
    "ready to",
];

const SUBJECT_PHRASES: &[&str] = &["greek salad", "grilled sardines", "seafood paella"];

pub(super) fn is_visual_object_hint(lower: &str) -> bool {
    VISUAL_OBJECT_HINTS.contains(&lower)
}

pub(super) fn is_generic_phrase(lower: &str) -> bool {
    GENERIC_PHRASES.contains(&lower)
}

pub(super) fn is_subject_phrase(lower: &str) -> bool {
    SUBJECT_PHRASES.contains(&lower)
}
