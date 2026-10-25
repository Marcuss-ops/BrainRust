//! Numeric date shapes: ISO (`YYYY-MM-DD`, `YYYY/MM/DD`), the bare
//! four-digit year and order-ambiguous `DD/MM/YYYY` forms.

use super::era::era_after;
use super::scan::{digits_at, language_is_day_first, next_after_separator_valid, separated_digits};
use super::Era::Ce;
use super::RawFact;

pub(super) fn collect_numeric_facts(source: &str, language: &str) -> Vec<RawFact> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let index = match bytes[cursor..]
            .iter()
            .position(u8::is_ascii_digit)
            .map(|position| cursor + position)
        {
            Some(index) => index,
            None => break,
        };
        if let Some(end) = try_iso_or_yyyy(source, index, &mut out) {
            cursor = end;
            continue;
        }
        if let Some(end) = try_order_numeric(source, index, language, &mut out) {
            cursor = end;
            continue;
        }
        cursor = index + 1;
    }
    out
}

/// `YYYY-MM[-DD]`, `YYYY/MM[/DD]`, and (without a delimiter) the bare
/// four-digit year. Delimited forms with impossible months or days are still
/// reported, unresolved, so the source surface survives.
fn try_iso_or_yyyy(source: &str, index: usize, out: &mut Vec<RawFact>) -> Option<usize> {
    let bytes = source.as_bytes();
    let year_len = digits_at(bytes, index).filter(|&count| count == 4)?;
    let year: i32 = source[index..index + year_len].parse().ok()?;
    let after_year = index + year_len;
    let delimiter = bytes.get(after_year).copied().filter(|&byte| {
        matches!(byte, b'-' | b'/') && next_after_separator_valid(bytes, after_year)
    });
    let Some(delimiter) = delimiter else {
        if (1000..=2099).contains(&year) {
            let era = era_after(source, after_year).unwrap_or(Ce);
            out.push(RawFact::bare(index, after_year, year, era));
            return Some(after_year);
        }
        return None;
    };
    let mut cursor = after_year + 1;
    let month_len = digits_at(bytes, cursor).filter(|&count| (1..=2).contains(&count))?;
    let month: u8 = source[cursor..cursor + month_len].parse().ok()?;
    cursor += month_len;
    let day = match bytes
        .get(cursor)
        .copied()
        .filter(|&byte| byte == delimiter && next_after_separator_valid(bytes, cursor))
    {
        Some(_) => {
            cursor += 1;
            let day_len = digits_at(bytes, cursor).filter(|&count| (1..=2).contains(&count))?;
            let day: u8 = source[cursor..cursor + day_len].parse().ok()?;
            cursor += day_len;
            Some(day)
        }
        None => None,
    };
    let era = era_after(source, cursor).unwrap_or(Ce);
    out.push(RawFact {
        start: index,
        end: cursor,
        year,
        era,
        month: Some(month),
        day,
        ambiguous: false,
        unresolved: false,
    });
    Some(cursor)
}

/// `DD/MM/YYYY`, `MM/DD/YYYY`, `DD-MM-YYYY`. The order is resolved by
/// impossible months first ("13/04/2020" is unambiguously day-first), by
/// language convention otherwise ("03/04/2020" is flagged ambiguous).
/// Two-digit years are rejected: two number-words do not name a decade.
fn try_order_numeric(
    source: &str,
    index: usize,
    language: &str,
    out: &mut Vec<RawFact>,
) -> Option<usize> {
    let bytes = source.as_bytes();
    let a_len = digits_at(bytes, index).filter(|&count| (1..=2).contains(&count))?;
    let a: u8 = source[index..index + a_len].parse().ok()?;
    let mut cursor = index + a_len;
    if !separated_digits(bytes, cursor) {
        return None;
    }
    cursor += 1;
    let b_len = digits_at(bytes, cursor).filter(|&count| (1..=2).contains(&count))?;
    let b: u8 = source[cursor..cursor + b_len].parse().ok()?;
    cursor += b_len;
    if !separated_digits(bytes, cursor) {
        return None;
    }
    cursor += 1;
    let year_len = digits_at(bytes, cursor).filter(|&count| count == 4)?;
    let year: i32 = source[cursor..cursor + year_len].parse().ok()?;
    let end = cursor + year_len;
    let era = era_after(source, end).unwrap_or(Ce);
    let (month, day, ambiguous) = match (a > 12, b > 12) {
        (true, true) => {
            // Both parts would be impossible months; report the textual
            // order, unresolved.
            out.push(RawFact {
                start: index,
                end,
                year,
                era,
                month: Some(a),
                day: Some(b),
                ambiguous: false,
                unresolved: true,
            });
            return Some(end);
        }
        (true, false) => (b, a, false),
        (false, true) => (a, b, false),
        (false, false) => {
            // Both calendar-legal: the language convention decides, and the
            // fact is flagged ambiguous.
            if language_is_day_first(language) {
                (b, a, true)
            } else {
                (a, b, true)
            }
        }
    };
    out.push(RawFact::dated(
        index,
        end,
        year,
        Some(era),
        month,
        Some(day),
        ambiguous,
    ));
    Some(end)
}
