//! Shared low-level scanners every date recognizer reads: digit runs,
//! separator checks, month-token boundaries, the day-first convention and
//! the generated month-name table.

/// Byte width of the first character of `text`, or 1 for an empty slice.
/// Used to advance a scan cursor without splitting a UTF-8 code point.
pub(super) fn next_char_width(text: &str) -> usize {
    text.chars().next().map_or(1, char::len_utf8)
}

/// True when `separator` holds a date separator (`/`, `-`) followed by a
/// digit.
pub(super) fn separated_digits(bytes: &[u8], separator: usize) -> bool {
    bytes
        .get(separator)
        .is_some_and(|&byte| matches!(byte, b'/' | b'-'))
        && bytes
            .get(separator + 1)
            .is_some_and(|&byte| byte.is_ascii_digit())
}

/// A month token must not sit inside a longer alphabetic word.
pub(super) fn month_token_bounded(text: &str, position: usize, width: usize) -> bool {
    let bytes = text.as_bytes();
    let before_ok = bytes
        .get(position.wrapping_sub(1))
        .is_none_or(|&byte| !byte.is_ascii_alphabetic());
    let after_ok = bytes
        .get((position + width).min(bytes.len()))
        .is_none_or(|&byte| !byte.is_ascii_alphabetic());
    before_ok && after_ok
}
/// Byte length of the ASCII digit run starting at `start`, capped at 5.
pub(super) fn digits_at(bytes: &[u8], start: usize) -> Option<usize> {
    if start >= bytes.len() || !bytes[start].is_ascii_digit() {
        return None;
    }
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() && end - start < 5 {
        end += 1;
    }
    Some(end - start)
}

/// A separator is a date delimiter only when a digit follows it.
pub(super) fn next_after_separator_valid(bytes: &[u8], separator: usize) -> bool {
    bytes
        .get(separator + 1)
        .is_some_and(|&byte| byte.is_ascii_digit())
}

/// Day-first writing convention for the language. The order is a fact in
/// `language_profiles.json` (`date_order`), not a `!= "en"` rule: languages
/// that write year-month-day or month-first need their own entry, and the
/// `default` entry keeps today's day-first behavior for the rest.
pub(super) fn language_is_day_first(language: &str) -> bool {
    crate::lexicon::lexicon()
        .language_profiles
        .date_order(language)
        == crate::lexicon::DateOrder::DayFirst
}
//–––––––––––––––––––––––––––– Month table –––––––––––––––––––––––––––––––––

/// Month names recognized case-insensitively across the pipeline's supported
/// languages, mapped to calendar numbers: the CLDR-generated table in
/// `lexicon/months.json` (see `lexicon/tools/sync_cldr_months.py`). Its order
/// is the tie-break order: a name that is a strict prefix of another name
/// sorts after it, so the longer name wins at the same offset (the boundary
/// check only guards ASCII letters, so "may" must not win inside "mayıs").
/// Duplicated across-language homographs are intentionally harmless because
/// they agree on the month number.
pub(super) fn month_names() -> &'static [(String, u8)] {
    &crate::lexicon::Lexicon::global().month_date_scan_table
}
