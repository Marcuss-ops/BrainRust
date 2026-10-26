//! Numeric-value span projection independent of the visual top-N budget.

use super::values::value_candidates;

/// Byte span of one numeric-value candidate (NUMBER/PERCENT/MONEY),
/// independent of the visual top-N budget ( NumericFacts).
#[derive(Debug, Clone, PartialEq)]
pub struct NumericSpan {
    pub start: usize,
    pub end: usize,
    pub entity_type: String,
}

/// Collects every numeric-value span from `source_text` in textual order.
/// This projection exists so a scene with more numbers than the visual
/// top-N budget keeps its numeric facts: charts and composed statistics read
/// from this list, not from the ranked entities.
pub fn numeric_value_spans(source_text: &str) -> Vec<NumericSpan> {
    value_candidates(source_text)
        .into_iter()
        .filter(|candidate| {
            matches!(
                candidate.entity_type.as_deref(),
                Some("NUMBER") | Some("PERCENT") | Some("MONEY") | Some("MEASUREMENT")
            )
        })
        .map(|candidate| NumericSpan {
            start: candidate.start,
            end: candidate.end,
            // A measurement is still a number for the chart code; its type
            // stays NUMBER here so chart decisions are unchanged.
            entity_type: match candidate.entity_type.as_deref() {
                Some("MEASUREMENT") | None => "NUMBER".to_string(),
                Some(other) => other.to_string(),
            },
        })
        .collect()
}
