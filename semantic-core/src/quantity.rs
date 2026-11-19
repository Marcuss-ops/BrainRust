//! Typed, source-grounded quantitative facts.
//!
//! The semantic engine attaches these records to numeric spans only when a
//! known unit is present in the local lexical window. Unknown units are left
//! untyped instead of being inferred from surrounding prose.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Quantity {
    pub id: String,
    pub value: Option<f64>,
    pub raw_value: Option<f64>,
    pub scale: Option<f64>,
    pub dimension: String,
    pub unit: String,
    pub unit_surface: String,
    pub qualifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<QuantityPeriod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub evidence: String,
    pub sentence_index: usize,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct QuantityPeriod {
    pub year: i32,
    pub fact_index: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct QuantityComparison {
    pub kind: String,
    pub left: String,
    pub right: String,
    pub dimension: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difference: Option<f64>,
    pub evidence_start_byte: usize,
    pub evidence_end_byte: usize,
}
