//! Era-marked years (`476 a.C.`, `44 BC`, `1054 p.n.e.`) and the era-marker
//! vocabulary shared with the year scanners.

use super::Era::{Bce, Ce};
use super::{Era, RawFact};

//–––––––––––––––– Era-marked years (476 a.C., 44 BC) –––––––––––––––––––––

pub(super) fn collect_era_years(source: &str) -> Vec<RawFact> {
    let bytes = source.as_bytes();
    let mut out: Vec<(usize, usize, i32, Era)> = Vec::new();
    for marker in era_markers() {
        let marker_len = marker.marker.len();
        let mut cursor = 0;
        while cursor + marker_len <= bytes.len() {
            if bytes[cursor..cursor + marker_len].eq_ignore_ascii_case(marker.marker.as_bytes()) {
                let end = cursor + marker_len;
                let before_ok = bytes
                    .get(cursor.wrapping_sub(1))
                    .is_none_or(|&byte| !byte.is_ascii_alphabetic());
                let after_ok = bytes
                    .get(end)
                    .is_none_or(|&byte| !byte.is_ascii_alphabetic());
                if before_ok && after_ok {
                    if let Some((start, year)) = year_before_marker(source, cursor) {
                        out.push((start, end, year, era_of(marker)));
                    }
                }
                cursor = end.max(cursor + 1);
            } else {
                cursor += 1;
            }
        }
    }
    out.sort_by_key(|entry| entry.0);
    out.dedup_by_key(|entry| entry.0);
    out.into_iter()
        .map(|(start, end, year, era)| RawFact::bare(start, end, year, era))
        .collect()
}

/// Reads the 1–4 digit year immediately before an era-marker position,
/// skipping-whitespace: `476 a.C.`, `44 BC`, `1054 p.n.e.`.
fn year_before_marker(source: &str, position: usize) -> Option<(usize, i32)> {
    let bytes = source.as_bytes();
    let mut cursor = position;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    if cursor == 0 || !bytes[cursor - 1].is_ascii_digit() {
        return None;
    }
    let digit_end = cursor;
    let mut start = digit_end;
    while start > 0 && bytes[start - 1].is_ascii_digit() && digit_end - start < 4 {
        start -= 1;
    }
    if digit_end - start == 0 || (start > 0 && bytes[start - 1].is_ascii_digit()) {
        return None; // 5+ digit run: a quantity, not a year.
    }
    if start > 0 && bytes[start - 1].is_ascii_alphabetic() {
        return None; // Part of a larger token (e.g. "r1054 p.n.e.").
    }
    let year: i32 = source[start..digit_end].parse().ok()?;
    (year > 0).then_some((start, year))
}
/// Era marker check at `from`: whitespace-separated and token-bounded.
pub(super) fn era_after(source: &str, from: usize) -> Option<Era> {
    let bytes = source.as_bytes();
    if from == 0 {
        return None;
    }
    if !bytes[from - 1].is_ascii_whitespace() {
        return None;
    }
    let tail = source.get(from..)?;
    for marker in era_markers() {
        if matches_marker(tail, &marker.marker) {
            return Some(era_of(marker));
        }
    }
    None
}

/// ASCII case-insensitive prefix match with an alphabetic boundary after.
pub(super) fn matches_marker(tail: &str, marker: &str) -> bool {
    let bytes = tail.as_bytes();
    bytes.len() >= marker.len()
        && bytes[..marker.len()].eq_ignore_ascii_case(marker.as_bytes())
        && bytes
            .get(marker.len())
            .is_none_or(|&byte| !byte.is_ascii_alphabetic())
}

/// Era markers from `lexicon/orthography.json`, in data order: longer forms
/// first so `b.c.e.` wins before `b.c.` and `bc`.
pub(super) fn era_markers() -> &'static [crate::lexicon::EraMarker] {
    &crate::lexicon::lexicon().orthography.era_markers
}

pub(super) fn era_of(marker: &crate::lexicon::EraMarker) -> Era {
    if marker.era == "bce" {
        Bce
    } else {
        Ce
    }
}
