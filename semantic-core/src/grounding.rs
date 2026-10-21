//! Grounding: the anti-hallucination gate.
//!
//! NO EVIDENCE -> NO CANDIDATE. Before any span leaves the engine it is
//! checked against the source: the candidate's `evidence` must equal the
//! verbatim slice at `[start_byte, end_byte)`, and the slice must exist. A
//! candidate that fails is dropped and counted in `rejected_items`, never
//! silently altered or invented.

use serde::Serialize;

/// Outcome of checking one span against its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Grounding {
    Grounded,
    /// `evidence` does not match `source[start_byte..end_byte)`.
    Mismatched,
    /// The byte range is out of bounds or not a char boundary.
    OutOfBounds,
}

/// Checks that `evidence` is exactly `source[start_byte..end_byte)`.
pub fn check_span(source: &str, start_byte: usize, end_byte: usize, evidence: &str) -> Grounding {
    if start_byte > end_byte || end_byte > source.len() {
        return Grounding::OutOfBounds;
    }
    if !source.is_char_boundary(start_byte) || !source.is_char_boundary(end_byte) {
        return Grounding::OutOfBounds;
    }
    if &source[start_byte..end_byte] == evidence {
        Grounding::Grounded
    } else {
        Grounding::Mismatched
    }
}

/// Keeps only the spans whose evidence matches the source, returning the
/// survivors and the number rejected. Generic over any span carrying offsets
/// and an evidence string via the accessors.
pub fn retain_grounded<T, FStart, FEnd, FEvidence>(
    source: &str,
    items: Vec<T>,
    start_of: FStart,
    end_of: FEnd,
    evidence_of: FEvidence,
) -> (Vec<T>, usize)
where
    FStart: Fn(&T) -> usize,
    FEnd: Fn(&T) -> usize,
    FEvidence: Fn(&T) -> &str,
{
    let mut kept = Vec::new();
    let mut rejected = 0;
    for item in items {
        if check_span(source, start_of(&item), end_of(&item), evidence_of(&item))
            == Grounding::Grounded
        {
            kept.push(item);
        } else {
            rejected += 1;
        }
    }
    (kept, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_span_is_grounded() {
        assert_eq!(
            check_span("hello world", 0, 5, "hello"),
            Grounding::Grounded
        );
    }

    #[test]
    fn mismatched_evidence_is_rejected() {
        assert_eq!(
            check_span("hello world", 0, 5, "world"),
            Grounding::Mismatched
        );
    }

    #[test]
    fn out_of_bounds_is_rejected() {
        assert_eq!(check_span("hi", 0, 9, "hi"), Grounding::OutOfBounds);
        assert_eq!(check_span("hi", 2, 1, "i"), Grounding::OutOfBounds);
    }

    #[test]
    fn non_char_boundary_is_rejected() {
        // 0..1 slices the middle of the 'é' two-byte char.
        assert_eq!(check_span("é", 0, 1, "é"), Grounding::OutOfBounds);
    }
}
