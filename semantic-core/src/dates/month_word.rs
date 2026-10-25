//! Month-word dates (`12 marzo 2001`, `March 12, 2001`, `12 de março de
//! 2001`): month-token location, the case-folded scan and the day/year
//! parts composed around the month token.

use super::era::{era_markers, era_of, matches_marker};
use super::scan::{digits_at, month_names, month_token_bounded, next_char_width};
use super::words::{
    connector_before, skip_connectors_after, spelled_number_after, spelled_number_before,
};
use super::Era::{self, Ce};
use super::{RawFact, SpelledNumber};

//–––––––––––––––––– Month-word facts (12 marzo 2001) ––––––––––––––––––––––

pub(super) fn collect_month_word_facts(
    source: &str,
    language: &str,
    words: &[SpelledNumber],
) -> Vec<RawFact> {
    let view = FoldedSource::build(source);
    let mut out = Vec::new();
    let mut folded_cursor = 0;
    while let Some((folded_start, month, folded_width)) = find_month(&view, folded_cursor) {
        let folded_next = folded_start + next_char_width(&view.text[folded_start..]);
        if month_token_bounded(&view.text, folded_start, folded_width) {
            if let Some((start, end)) = view.to_source(folded_start, folded_start + folded_width) {
                let width = end - start;
                if let Some(fact) = month_word_fact(source, language, start, month, width, words) {
                    // "19, 20 e 26 settembre 2007" shares one month and year
                    // across a coordinated day list. The last day keeps the
                    // full month/year span; the earlier days keep their own
                    // digit span but inherit the shared month and year.
                    let days = coordinated_days_before(source, start);
                    if days.len() > 1 && fact.month.is_some() && fact.year > 0 && !fact.unresolved {
                        for (day, day_start, day_end) in &days[..days.len() - 1] {
                            out.push(RawFact {
                                start: *day_start,
                                end: *day_end,
                                year: fact.year,
                                era: fact.era,
                                month: fact.month,
                                day: Some(*day),
                                ambiguous: false,
                                unresolved: false,
                            });
                        }
                    }
                    out.push(fact);
                }
            }
        }
        folded_cursor = folded_next;
    }
    out
}

/// Case-folded view of the source used only to locate month tokens. Carries
/// an index map back into the original byte offsets, so case-folding that
/// changes lengths (e.g. Turkish `İ`) cannot corrupt facts.
struct FoldedSource {
    text: String,
    /// Char index in `text` → byte offset in the original source.
    offsets: Vec<usize>,
}

impl FoldedSource {
    /// Builds the folded view; month scanning is disabled (empty map) when
    /// case folding is not information-preserving.
    fn build(source: &str) -> Self {
        let text = source.to_lowercase();
        if text.chars().count() == source.chars().count() {
            FoldedSource {
                text,
                offsets: source
                    .char_indices()
                    .map(|(offset, _)| offset)
                    .chain(std::iter::once(source.len()))
                    .collect(),
            }
        } else {
            FoldedSource {
                text: String::new(),
                offsets: Vec::new(),
            }
        }
    }

    /// Maps a folded-text span onto the original source byte offsets, so
    /// facts keep verbatim surfaces and offsets even when case folding
    /// changes byte lengths inside a month token.
    fn to_source(&self, folded_start: usize, folded_end: usize) -> Option<(usize, usize)> {
        if folded_start > folded_end || folded_end > self.text.len() {
            return None;
        }
        // A byte index inside a multi-byte character is not a span. Returning
        // None keeps the fact contract honest instead of panicking the worker.
        if !self.text.is_char_boundary(folded_start) || !self.text.is_char_boundary(folded_end) {
            return None;
        }
        let char_start = self.text[..folded_start].chars().count();
        let char_end = self.text[..folded_end].chars().count();
        let start = self.offsets.get(char_start).copied()?;
        let end = self.offsets.get(char_end).copied()?;
        (end >= start).then_some((start, end))
    }
}

/// Locates the next month-token occurrence in the folded text.
fn find_month(view: &FoldedSource, cursor: usize) -> Option<(usize, u8, usize)> {
    if view.offsets.is_empty() || cursor >= view.text.len() {
        return None;
    }
    let hay = &view.text[cursor..];
    let mut best: Option<(usize, u8, usize)> = None;
    for (word, month) in month_names() {
        let mut offset = 0;
        while let Some(found) = hay[offset..].find(word.as_str()) {
            let position = cursor + offset + found;
            if month_token_bounded(&view.text, position, word.len()) {
                let candidate = (position, *month, word.len());
                best = match best {
                    None => Some(candidate),
                    // Earliest position wins. At the same position the LONGER
                    // form wins, because a month name can be a prefix of its
                    // own inflected form (Russian "март" / "марта"): the
                    // shorter one would leave the suffix unconsumed and the
                    // date unresolved.
                    Some(existing)
                        if candidate.0 < existing.0
                            || (candidate.0 == existing.0 && candidate.2 > existing.2) =>
                    {
                        Some(candidate)
                    }
                    Some(existing) => Some(existing),
                };
                break;
            }
            // `found` is relative to the current suffix (`hay[offset..]`).
            // Keep the cursor monotonic when a month name occurs more than
            // once: advance by one CHARACTER, never one byte, because month
            // names are not ASCII ("марта", "březen", "şubat") and a byte
            // step would land inside a code point.
            offset += found + next_char_width(&hay[offset + found..]);
        }
    }
    best
}
/// Composes `day? month [year|day year]?` around a month token, covering
/// the day-first languages (`12 marzo 2001`), the English connective form
/// (`March 12, 2001`) and the prepositional day (`12 de março de 2001`).
/// Without the year the fact is unresolved: a stated day+month names no
/// year, so no ISO value is invented.
fn month_word_fact(
    text: &str,
    language: &str,
    month_start: usize,
    month: u8,
    width: usize,
    words: &[SpelledNumber],
) -> Option<RawFact> {
    let month_end = month_start + width;
    let day_before = day_before(text, language, month_start, words);
    let day_after = day_after(text, language, month_end, words);
    let year_from = day_after.map_or(month_end, |(_, day_end)| day_end);
    let year = year_after(text, language, year_from, words);
    let day_value = day_after
        .map(|(value, _)| value)
        .or(day_before.map(|(value, _)| value));
    let start = day_before
        .as_ref()
        .map_or(month_start, |(_, day_start)| *day_start);
    let Some((year_value, era, year_end)) = year else {
        let day_stated = day_value?;
        let day_u8 = u8::try_from(day_stated).ok()?;
        return Some(RawFact {
            start,
            end: month_end,
            year: 0,
            era: Ce,
            month: Some(month),
            day: Some(day_u8),
            ambiguous: false,
            unresolved: true,
        });
    };
    let day = day_value
        .and_then(|value| u8::try_from(value).ok())
        .filter(|value| (1..=31).contains(value));
    Some(RawFact::dated(
        start,
        year_end,
        year_value,
        Some(era),
        month,
        day,
        false,
    ))
}
/// `(day_value, day_start)` for a 1–2 digit day immediately before the
/// month token, separated by whitespace or one preposition connector
/// (`12 de março`).
fn day_before(
    source: &str,
    language: &str,
    month_start: usize,
    words: &[SpelledNumber],
) -> Option<(u32, usize)> {
    if let Some(fact) = day_digits_before(source, month_start) {
        return Some(fact);
    }
    if let Some(fact) = spelled_number_before(source, words, month_start, 1, 31) {
        return Some(fact);
    }
    // Connector form: whitespace + day + whitespace + connective + month.
    let connector_start = connector_before(source, month_start, language)?;
    if let Some(fact) = day_digits_before(source, connector_start) {
        return Some(fact);
    }
    spelled_number_before(source, words, connector_start, 1, 31)
}
/// Reads a 1–2 digit day immediately before `position`, without crossing a
/// comma or a connector. Used by the coordinated-list scanner.
fn day_token_before(source: &str, position: usize) -> Option<(u8, usize, usize)> {
    let bytes = source.as_bytes();
    let mut end = position;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 || !bytes[end - 1].is_ascii_digit() {
        return None;
    }
    let digit_end = end;
    let mut start = digit_end;
    while start > 0 && bytes[start - 1].is_ascii_digit() && digit_end - start < 2 {
        start -= 1;
    }
    if start == digit_end || (start > 0 && bytes[start - 1].is_ascii_digit()) {
        return None;
    }
    if start > 0 && bytes[start - 1].is_ascii_alphabetic() {
        return None;
    }
    let value: u8 = source[start..digit_end].parse().ok()?;
    (1..=31)
        .contains(&value)
        .then_some((value, start, digit_end))
}

/// Days in a coordinated list ending at `month_start`, in source order:
/// `19, 20 e 26` → `[19, 20, 26]`. A comma or a conjunction is required
/// between members; a bare sequence such as `19 20 26` is not a list.
fn coordinated_days_before(source: &str, month_start: usize) -> Vec<(u8, usize, usize)> {
    let mut days = Vec::new();
    let mut cursor = month_start;
    while let Some((day, start, end)) = day_token_before(source, cursor) {
        days.push((day, start, end));
        cursor = start;
        let bytes = source.as_bytes();
        let mut separator_end = cursor;
        while separator_end > 0 && bytes[separator_end - 1].is_ascii_whitespace() {
            separator_end -= 1;
        }
        if separator_end == 0 {
            break;
        }
        if bytes[separator_end - 1] == b',' {
            cursor = separator_end - 1;
            continue;
        }
        let mut word_start = separator_end;
        while word_start > 0
            && (bytes[word_start - 1].is_ascii_alphabetic() || bytes[word_start - 1] == b'\'')
        {
            word_start -= 1;
        }
        if word_start == separator_end {
            break;
        }
        let word = source[word_start..separator_end].to_lowercase();
        if !is_list_connector(&word) {
            break;
        }
        cursor = word_start;
        if days.len() >= 8 {
            break;
        }
    }
    days.reverse();
    days
}

fn is_list_connector(word: &str) -> bool {
    matches!(
        word,
        "e" | "ed" | "and" | "y" | "et" | "en" | "und" | "ve" | "si" | "și"
    )
}

fn day_digits_before(source: &str, month_start: usize) -> Option<(u32, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = month_start;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    // German ordinal days carry a dot ("12. März"); it belongs to the date but
    // not to the day digits. Only a dot directly after the digits is consumed.
    if cursor > 1 && bytes[cursor - 1] == b'.' && bytes[cursor - 2].is_ascii_digit() {
        cursor -= 1;
    }
    if cursor == 0 || !bytes[cursor - 1].is_ascii_digit() {
        return None;
    }
    let digit_end = cursor;
    let mut start = digit_end;
    while start > 0 && bytes[start - 1].is_ascii_digit() && digit_end - start < 2 {
        start -= 1;
    }
    let width = digit_end - start;
    if width == 0 || width > 2 || (start > 0 && bytes[start - 1].is_ascii_digit()) {
        return None; // 3+ digit run: not a day.
    }
    if start > 0 && bytes[start - 1].is_ascii_alphabetic() {
        return None; // Part of a larger token.
    }
    let value: u32 = source[start..digit_end].parse().ok()?;
    if !(1..=31).contains(&value) {
        return None;
    }
    Some((value, start))
}

/// `(day_value, end_offset)` for a 1–2 digit day immediately after the
/// month token (`March 12, 2001`). Longer digit runs (`marzo 2001`) are
/// years, not days, and are left to `year_after`.
fn day_after(
    source: &str,
    _language: &str,
    month_end: usize,
    words: &[SpelledNumber],
) -> Option<(u32, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = month_end;
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if let Some(digit_len) = digits_at(bytes, cursor).filter(|&len| (1..=2).contains(&len)) {
        let digit_start = cursor;
        let digits_end = cursor + digit_len;
        if bytes
            .get(digits_end)
            .is_some_and(|&byte| byte.is_ascii_digit())
        {
            return None; // The run was longer: not a plain day.
        }
        let value: u32 = source[digit_start..digits_end].parse().ok()?;
        if !(1..=31).contains(&value) {
            return None;
        }
        return Some((value, digits_end));
    }
    let (value, end) = spelled_number_after(source, words, cursor, 1, 31)?;
    Some((value as u32, end))
}

/// Locates the stated year after a month token across at most one connector
/// word; an attached era marker stays inside the span. Returns
/// `(year, era, end_offset)`.
fn year_after(
    source: &str,
    language: &str,
    month_end: usize,
    words: &[SpelledNumber],
) -> Option<(i32, Era, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = month_end;
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    // "March 12, 2001": a single comma may separate the parts.
    if bytes.get(cursor).is_some_and(|&byte| byte == b',') {
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
    }
    cursor = skip_connectors_after(source, cursor, language);
    while bytes
        .get(cursor)
        .is_some_and(|&byte| byte.is_ascii_whitespace())
    {
        cursor += 1;
    }
    let (year, digits_end, spelled) =
        match digits_at(bytes, cursor).filter(|&len| (3..=4).contains(&len)) {
            Some(digit_len) => {
                let year: i32 = source[cursor..cursor + digit_len].parse().ok()?;
                (year, cursor + digit_len, false)
            }
            None => {
                let (value, end) = spelled_number_after(source, words, cursor, 1000, 2099)?;
                (value as i32, end, true)
            }
        };
    if year <= 0 {
        return None;
    }
    let mut end = digits_end;
    let era = if !spelled && digits_end < bytes.len() && bytes[digits_end].is_ascii_whitespace() {
        let tail = source[digits_end..].trim_start();
        match era_markers()
            .iter()
            .find(|marker| matches_marker(tail, &marker.marker))
        {
            Some(marker) => {
                end = digits_end + (source[digits_end..].len() - tail.len()) + marker.marker.len();
                era_of(marker)
            }
            None => Ce,
        }
    } else {
        Ce
    };
    Some((year, era, end))
}
