//! TemporalFacts: every source-grounded DATE occurrence, independent of the
//! visual top-N budget.
//!
//! The visual extractor keeps its certified top-N behaviour (`entity_count`
//! is an editorial budget for visual entities). A scene whose voiceover
//! mentions five dates must not lose facts before PipelineGen can group them
//! into a timeline, so this projection reuses the same source-evidence rule
//! and emits **all** date occurrences regardless of that budget:
//!
//! ```text
//! VisualNER
//!  ├── Entità visive classificate (budget attuale)      ← extract/analyze
//!  ├── TemporalFacts (tutte le date ammesse)            ← temporal_facts
//!  └── ...
//! ```
//!
//! Normalization is intentionally separate from the `DATE` classification of
//! the top-N extractor. Not every surface the extractor calls a date is a
//! validated calendar date; here each fact carries:
//!
//!  - `precision` — what the narration states: `day` (year+month+day),
//!    `month` (year+month), `year` (year only), or `unresolved` (presentable
//!    source form that could not be validated as a calendar date);
//!  - `normalized_date` — the ISO form **only** when the calendar position is
//!    fully determined; for a spoken year it is `YYYY`, never a fabricated
//!    day or month. A spoken year stays a single label;
//!  - `ambiguous` — true for order-ambiguous numeric forms such as
//!    `03/04/2020`, where the language convention picks the layout but the
//!    source itself does not.
//!
//! The killer rule is inherited from the extractor: NO EVIDENCE → NO FACT.
//! Every fact's `surface` equals the verbatim substring of the source text
//! at exactly `start_byte..end_byte`. Byte offsets are UTF-8.
//!
//! The recognizers are deliberately zero-dependency and deterministic: no
//! third-party date parser, no locale database, so two runs of the same
//! input produce byte-identical facts.

use serde::Serialize;

/// Stated year+month+day.
pub const PRECISION_DAY: &str = "day";
/// Stated year+month.
pub const PRECISION_MONTH: &str = "month";
/// Stated year only.
pub const PRECISION_YEAR: &str = "year";
/// Presentable source form that could not be validated as a calendar date.
pub const PRECISION_UNRESOLVED: &str = "unresolved";

/// One grounded date fact. The wire shape follows the integration contract:
/// `surface`/`type`/`normalized_date`/`precision`/`start_byte`/`end_byte`/
/// `evidence`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TemporalFact {
    /// The exact source form (e.g. `"15 marzo 2008"`), preserving case.
    pub surface: String,
    /// Always `"DATE"`: presentation kinds (timeline, multi-image, chart)
    /// are decided by PipelineGen, not by this linguistic recognizer.
    #[serde(rename = "type")]
    pub kind: String,
    /// Normalized ISO form (`YYYY-MM-DD`, `YYYY-MM` or `YYYY`) when the
    /// stated calendar position is fully resolved. `None` for unresolved
    /// facts and for BCE dates (whose proleptic ISO sign convention is out
    /// of this contract's scope); the era is carried in [`TemporalFact::era`].
    #[serde(rename = "normalized_date")]
    pub normalized: Option<String>,
    /// One of `day`, `month`, `year`, `unresolved`.
    pub precision: &'static str,
    /// Inclusive UTF-8 byte offset where the fact's evidence begins.
    pub start_byte: usize,
    /// Exclusive UTF-8 byte offset where the fact's evidence ends.
    pub end_byte: usize,
    /// Verbatim source excerpt at `[start_byte, end_byte)`.
    pub evidence: String,
    /// `"BCE"` when the narration states a before-Christ era (e.g.
    /// `476 a.C.`); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub era: Option<&'static str>,
    /// True when a numeric form is order-ambiguous in the source (e.g.
    /// `03/04/2020`); the normalized value follows the language convention.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ambiguous: bool,
}

/// Collects every date fact for `source_text` in textual order, sorted by
/// offset, without duplicate or nested spans. Never fails: an unparseable
/// fragment simply yields no fact, while parseable but calendar-impossible
/// forms are still reported as `unresolved` (the surface survives; no ISO
/// value is invented).
pub fn temporal_facts(source_text: &str, language: &str) -> Vec<TemporalFact> {
    let language = language.trim().to_lowercase();
    let mut raw: Vec<RawFact> = collect_numeric_facts(source_text, &language);
    raw.extend(collect_month_word_facts(source_text));
    raw.extend(collect_era_years(source_text));
    resolve(source_text, raw)
}

//–––––––––––––––––––––––––––– RawFact model –––––––––––––––––––––––––––––––

/// Pre-validation data for one date-span candidate. `year` is the stated
/// positive year; the era is carried separately.
#[derive(Debug, Clone, PartialEq)]
struct RawFact {
    start: usize,
    end: usize,
    year: i32,
    era: Era,
    month: Option<u8>,
    day: Option<u8>,
    ambiguous: bool,
    /// Set by a recognizer whose form the calendar rejects; the fact is
    /// still emitted (with the source surface) but as `unresolved`.
    unresolved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Era {
    Ce,
    Bce,
}

use Era::{Bce, Ce};

impl RawFact {
    fn bare(start: usize, end: usize, year: i32, era: Era) -> Self {
        RawFact {
            start,
            end,
            year,
            era,
            month: None,
            day: None,
            ambiguous: false,
            unresolved: false,
        }
    }

    fn dated(
        start: usize,
        end: usize,
        year: i32,
        era: Option<Era>,
        month: u8,
        day: Option<u8>,
        ambiguous: bool,
    ) -> Self {
        RawFact {
            start,
            end,
            year,
            era: era.unwrap_or(Ce),
            month: Some(month),
            day,
            ambiguous,
            unresolved: false,
        }
    }
}

//–––––––––––––––––––––– Numeric forms scan –––––––––––––––––––––––––––––––

fn collect_numeric_facts(source: &str, language: &str) -> Vec<RawFact> {
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

//–––––––––––––––––– Month-word facts (12 marzo 2001) ––––––––––––––––––––––

fn collect_month_word_facts(source: &str) -> Vec<RawFact> {
    let view = FoldedSource::build(source);
    let mut out = Vec::new();
    let mut folded_cursor = 0;
    while let Some((folded_start, month, folded_width)) = find_month(&view, folded_cursor) {
        let folded_next = folded_start + 1;
        if month_token_bounded(&view.text, folded_start, folded_width) {
            if let Some((start, end)) = view.to_source(folded_start, folded_start + folded_width) {
                let width = end - start;
                if let Some(fact) = month_word_fact(source, start, month, width) {
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
        while let Some(found) = hay[offset..].find(word) {
            let position = cursor + offset + found;
            if month_token_bounded(&view.text, position, word.len()) {
                let candidate = (position, *month, word.len());
                best = match best {
                    None => Some(candidate),
                    Some(existing) if candidate.0 < existing.0 => Some(candidate),
                    Some(existing) => Some(existing),
                };
                break;
            }
            offset = found + 1;
        }
    }
    best
}

/// True when `separator` holds a date separator (`/`, `-`) followed by a
/// digit.
fn separated_digits(bytes: &[u8], separator: usize) -> bool {
    bytes
        .get(separator)
        .is_some_and(|&byte| matches!(byte, b'/' | b'-'))
        && bytes
            .get(separator + 1)
            .is_some_and(|&byte| byte.is_ascii_digit())
}

/// A month token must not sit inside a longer alphabetic word.
fn month_token_bounded(text: &str, position: usize, width: usize) -> bool {
    let bytes = text.as_bytes();
    let before_ok = bytes
        .get(position.wrapping_sub(1))
        .is_none_or(|&byte| !byte.is_ascii_alphabetic());
    let after_ok = bytes
        .get((position + width).min(bytes.len()))
        .is_none_or(|&byte| !byte.is_ascii_alphabetic());
    before_ok && after_ok
}

/// Composes `day? month [year|day year]?` around a month token, covering
/// the day-first languages (`12 marzo 2001`), the English connective form
/// (`March 12, 2001`) and the prepositional day (`12 de março de 2001`).
/// Without the year the fact is unresolved: a stated day+month names no
/// year, so no ISO value is invented.
fn month_word_fact(text: &str, month_start: usize, month: u8, width: usize) -> Option<RawFact> {
    let month_end = month_start + width;
    let day_before = day_before(text, month_start);
    let day_after = day_after(text, month_end);
    let year_from = day_after.map_or(month_end, |(_, day_end)| day_end);
    let year = year_after(text, year_from);
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

//–––––––––––––––– Era-marked years (476 a.C., 44 BC) –––––––––––––––––––––

fn collect_era_years(source: &str) -> Vec<RawFact> {
    let bytes = source.as_bytes();
    let mut out: Vec<(usize, usize, i32, Era)> = Vec::new();
    for marker in era_markers() {
        let marker_len = marker.len();
        let mut cursor = 0;
        while cursor + marker_len <= bytes.len() {
            if bytes[cursor..cursor + marker_len].eq_ignore_ascii_case(marker.as_bytes()) {
                let end = cursor + marker_len;
                let before_ok = bytes
                    .get(cursor.wrapping_sub(1))
                    .is_none_or(|&byte| !byte.is_ascii_alphabetic());
                let after_ok = bytes
                    .get(end)
                    .is_none_or(|&byte| !byte.is_ascii_alphabetic());
                if before_ok && after_ok {
                    if let Some((start, year)) = year_before_marker(source, cursor) {
                        out.push((start, end, year, marker_era(marker)));
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

//–––––––––––––––––––––––––– Resolution pass –––––––––––––––––––––––––––––––

/// Sorts by offset, drops facts nested inside a wider fact, validates the
/// calendar position, and materializes the wire facts with verbatim surface
/// evidence.
fn resolve(source: &str, mut facts: Vec<RawFact>) -> Vec<TemporalFact> {
    facts.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut out_raw: Vec<RawFact> = Vec::with_capacity(facts.len());
    for fact in facts {
        if let Some(kept) = out_raw.last() {
            if fact.start >= kept.start && fact.end <= kept.end {
                continue;
            }
        }
        out_raw.push(fact);
    }
    let mut out = Vec::with_capacity(out_raw.len());
    for fact in out_raw {
        let Some(surface) = source.get(fact.start..fact.end) else {
            continue;
        };
        let valid_month = fact.month.is_none_or(|month| (1..=12).contains(&month));
        let valid_day = fact.day.is_none_or(|day| {
            let year = fact.year.max(1);
            valid_month && day >= 1 && day <= days_in_month(fact.month.unwrap_or(1), year)
        });
        let impossible = fact.unresolved || !valid_month || !valid_day || fact.year <= 0;
        let has_month = fact.month.is_some() && !impossible;
        let has_day = fact.day.is_some() && !impossible;
        let (precision, normalized) = if impossible {
            (PRECISION_UNRESOLVED, None)
        } else if has_day {
            (
                PRECISION_DAY,
                Some(iso_fact(fact.year, fact.month, fact.day)),
            )
        } else if has_month {
            (PRECISION_MONTH, Some(iso_fact(fact.year, fact.month, None)))
        } else {
            (PRECISION_YEAR, Some(iso_fact(fact.year, None, None)))
        };
        out.push(TemporalFact {
            surface: surface.to_string(),
            kind: "DATE".to_string(),
            normalized,
            precision,
            start_byte: fact.start,
            end_byte: fact.end,
            evidence: surface.to_string(),
            era: (fact.era == Bce && !impossible).then_some("BCE"),
            ambiguous: fact.ambiguous,
        });
    }
    out
}

fn iso_fact(year: i32, month: Option<u8>, day: Option<u8>) -> String {
    let year_text = format!("{year:04}");
    match (month, day) {
        (Some(month), Some(day)) => format!("{year_text}-{month:02}-{day:02}"),
        (Some(month), None) => format!("{year_text}-{month:02}"),
        _ => year_text,
    }
}

//–––––––––––––––––––––––––––– Date helpers –––––––––––––––––––––––––––––––

fn days_in_month(month: u8, year: i32) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Byte length of the ASCII digit run starting at `start`, capped at 5.
fn digits_at(bytes: &[u8], start: usize) -> Option<usize> {
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
fn next_after_separator_valid(bytes: &[u8], separator: usize) -> bool {
    bytes
        .get(separator + 1)
        .is_some_and(|&byte| byte.is_ascii_digit())
}

/// Day-first writing convention for the pipeline's supported languages
/// (`it`, `es`, `fr`, `de`, `ru`, `pl`, `tr`, `id`, …); English keeps its
/// month-first editorial convention.
fn language_is_day_first(language: &str) -> bool {
    let base = language.split(['-', '_']).next().unwrap_or("");
    !matches!(base, "en")
}

/// Era marker check at `from`: whitespace-separated and token-bounded.
fn era_after(source: &str, from: usize) -> Option<Era> {
    let bytes = source.as_bytes();
    if from == 0 {
        return None;
    }
    if !bytes[from - 1].is_ascii_whitespace() {
        return None;
    }
    let tail = source.get(from..)?;
    for marker in era_markers() {
        if matches_marker(tail, marker) {
            return Some(marker_era(marker));
        }
    }
    None
}

/// ASCII case-insensitive prefix match with an alphabetic boundary after.
fn matches_marker(tail: &str, marker: &str) -> bool {
    let bytes = tail.as_bytes();
    bytes.len() >= marker.len()
        && bytes[..marker.len()].eq_ignore_ascii_case(marker.as_bytes())
        && bytes
            .get(marker.len())
            .is_none_or(|&byte| !byte.is_ascii_alphabetic())
}

/// Era markers, lowercase ASCII, longer forms first so `b.c.e.` wins before
/// `b.c.` and `bc`.
#[rustfmt::skip]
fn era_markers() -> &'static [&'static str] {
    &[
        "b.c.e.", "b.c.", "bce", "bc",
        "a.d.", "a.d", "c.e.", "ce", "a.c.", "a.d.", "d.c.", "d.c",
        "v.chr.", "n.chr.", "p.n.e.", "n.e.", "m.ö.", "m.s.", "ad",
    ]
}

fn marker_era(marker: &str) -> Era {
    match marker {
        "b.c.e." | "b.c." | "bce" | "bc" | "a.c." | "v.chr." | "p.n.e." | "m.ö." => Bce,
        _ => Ce,
    }
}

/// Prepositions that may sit between a month and its year ("marzo del 2008").
const MONTH_YEAR_CONNECTORS: &[&str] = &[
    "de", "del", "do", "da", "di", "des", "von", "anno", "año", "anio", "yıl",
];

/// `(day_value, day_start)` for a 1–2 digit day immediately before the
/// month token, separated by whitespace or one preposition connector
/// (`12 de março`).
fn day_before(source: &str, month_start: usize) -> Option<(u32, usize)> {
    if let Some(fact) = day_digits_before(source, month_start) {
        return Some(fact);
    }
    // Connector form: whitespace + day + whitespace + connective + month.
    let bytes = source.as_bytes();
    let mut cursor = month_start;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    for connector in MONTH_YEAR_CONNECTORS {
        let Some(connector_start) = cursor.checked_sub(connector.len()) else {
            continue;
        };
        let Some(tail) = source.get(connector_start..cursor) else {
            continue;
        };
        if !tail.eq_ignore_ascii_case(connector) {
            continue;
        }
        if !bytes
            .get(connector_start.wrapping_sub(1))
            .is_some_and(|&byte| byte.is_ascii_whitespace())
        {
            continue;
        }
        if let Some(fact) = day_digits_before(source, connector_start) {
            return Some(fact);
        }
    }
    None
}

fn day_digits_before(source: &str, month_start: usize) -> Option<(u32, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = month_start;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
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
fn day_after(source: &str, month_end: usize) -> Option<(u32, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = month_end;
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    let digit_start = cursor;
    let digit_len = digits_at(bytes, cursor).filter(|&len| (1..=2).contains(&len))?;
    cursor += digit_len;
    if bytes.get(cursor).is_some_and(|&byte| byte.is_ascii_digit()) {
        return None; // The run was longer: not a plain day.
    }
    let value: u32 = source[digit_start..cursor].parse().ok()?;
    if !(1..=31).contains(&value) {
        return None;
    }
    Some((value, cursor))
}

/// Locates the stated year after a month token across at most one connector
/// word; an attached era marker stays inside the span. Returns
/// `(year, era, end_offset)`.
fn year_after(source: &str, month_end: usize) -> Option<(i32, Era, usize)> {
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
    for connector in MONTH_YEAR_CONNECTORS {
        let Some(tail) = source.get(cursor..cursor + connector.len()) else {
            continue;
        };
        let next_is_space = bytes
            .get(cursor + connector.len())
            .is_some_and(|&byte| byte.is_ascii_whitespace());
        if next_is_space && tail.eq_ignore_ascii_case(connector) {
            cursor += connector.len();
            while bytes
                .get(cursor)
                .is_some_and(|&byte| byte.is_ascii_whitespace())
            {
                cursor += 1;
            }
            break;
        }
    }
    let digit_len = digits_at(bytes, cursor).filter(|&len| (3..=4).contains(&len))?;
    let year: i32 = source[cursor..cursor + digit_len].parse().ok()?;
    if year <= 0 {
        return None;
    }
    let digits_end = cursor + digit_len;
    let mut end = digits_end;
    let era = if digits_end < bytes.len() && bytes[digits_end].is_ascii_whitespace() {
        let tail = source[digits_end..].trim_start();
        match era_markers()
            .iter()
            .find(|&&marker| matches_marker(tail, marker))
        {
            Some(&marker) => {
                end = digits_end + (source[digits_end..].len() - tail.len()) + marker.len();
                marker_era(marker)
            }
            None => Ce,
        }
    } else {
        Ce
    };
    Some((year, era, end))
}

//–––––––––––––––––––––––––––– Month table –––––––––––––––––––––––––––––––––

/// Month names recognized case-insensitively across the pipeline's supported
/// languages, mapped to calendar numbers. Names are recursive-free literals;
/// duplicated across-language homographs are intentionally harmless because
/// they agree on the month number.
#[rustfmt::skip]
fn month_names() -> &'static [(&'static str, u8)] {
    &[
        // English
        ("january", 1), ("february", 2), ("march", 3), ("april", 4), ("june", 6),
        ("july", 7), ("august", 8), ("september", 9), ("october", 10),
        ("november", 11), ("december", 12),
        // Italian (+ Spanish marzo/aprile/mayo homographs agree numerically)
        ("gennaio", 1), ("febbraio", 2), ("marzo", 3), ("aprile", 4), ("maggio", 5),
        ("giugno", 6), ("luglio", 7), ("agosto", 8), ("settembre", 9),
        ("ottobre", 10), ("novembre", 11), ("dicembre", 12),
        // Spanish
        ("enero", 1), ("febrero", 2), ("abril", 4), ("junio", 6), ("julio", 7),
        ("agosto", 8), ("septiembre", 9), ("octubre", 10), ("noviembre", 11),
        ("diciembre", 12),
        // French
        ("janvier", 1), ("février", 2), ("mars", 3), ("avril", 4),
        ("mai", 5), ("juin", 6), ("juillet", 7), ("aout", 8),
        ("septembre", 9), ("octobre", 10), ("novembre", 11), ("decembre", 12),
        // German
        ("januar", 1), ("februar", 2), ("märz", 3), ("mai", 5), ("juni", 6),
        ("juli", 7), ("august", 8), ("oktober", 10), ("november", 11),
        ("dezember", 12),
        // Portuguese (março/maio share the It./Es. homographs' numbers)
        ("janeiro", 1), ("fevereiro", 2), ("março", 3), ("abril", 4),
        ("maio", 5), ("junho", 6), ("julho", 7), ("agosto", 8),
        ("setembro", 9), ("outubro", 10), ("novembro", 11), ("dezembro", 12),
        // Russian: nominative + genitive forms
        ("январь", 1), ("января", 1), ("февраль", 2), ("февраля", 2),
        ("март", 3), ("марта", 3), ("апрель", 4), ("апреля", 4),
        ("май", 5), ("мая", 5), ("июнь", 6), ("июня", 6), ("июль", 7), ("июля", 7),
        ("август", 8), ("августа", 8), ("сентябрь", 9), ("сентября", 9),
        ("октябрь", 10), ("октября", 10), ("ноябрь", 11), ("ноября", 11),
        ("декабрь", 12), ("декабря", 12),
        // Polish: nominative + genitive
        ("styczeń", 1), ("stycznia", 1), ("luty", 2), ("lutego", 2),
        ("marzec", 3), ("marca", 3), ("kwiecień", 4), ("kwietnia", 4),
        ("maj", 5), ("maja", 5), ("czerwiec", 6), ("czerwca", 6),
        ("lipiec", 7), ("lipca", 7), ("sierpień", 8), ("sierpnia", 8),
        ("wrzesień", 9), ("września", 9), ("październik", 10), ("października", 10),
        ("listopad", 11), ("listopada", 11), ("grudzień", 12), ("grudnia", 12),
        // Turkish
        ("ocak", 1), ("şubat", 2), ("mart", 3), ("nisan", 4), ("mayıs", 5),
        ("haziran", 6), ("temmuz", 7), ("ağustos", 8), ("eylül", 9),
        ("ekim", 10), ("kasım", 11), ("aralık", 12),
        // Indonesian
        ("januari", 1), ("februari", 2), ("desember", 12),
        // English may, deliberately indexed last so unambiguous homographs
        // win at equal offsets.
        ("may", 5),
    ]
}

//–––––––––––––––––––––––––––––– Tests –––––––––––––––––––––––––––––––––––––

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(text: &str, language: &str) -> Vec<TemporalFact> {
        temporal_facts(text, language)
    }

    fn assert_grounded(text: &str, all: &[TemporalFact]) {
        for fact in all {
            assert_eq!(
                text.get(fact.start_byte..fact.end_byte)
                    .unwrap_or("OUT-OF-RANGE"),
                fact.surface,
                "surface must be verbatim at its offsets"
            );
            assert_eq!(fact.evidence, fact.surface);
            assert_eq!(fact.kind, "DATE");
        }
        let mut previous: Option<&TemporalFact> = None;
        for fact in all {
            if let Some(last) = previous {
                assert!(
                    fact.start_byte > last.start_byte
                        || (fact.start_byte == last.start_byte && fact.end_byte >= last.end_byte),
                    "facts must be ordered by offset"
                );
            }
            previous = Some(fact);
        }
    }

    #[test]
    fn day_month_year_italian_normalizes_to_iso() {
        let text = "La svolta arrivò il 15 marzo 2008.";
        let all = facts(text, "it");
        assert_grounded(text, &all);
        let fact = all
            .iter()
            .find(|fact| fact.normalized.as_deref() == Some("2008-03-15"))
            .expect("15 marzo 2008 must normalize to 2008-03-15");
        assert_eq!(fact.precision, "day");
        assert!(fact.surface.contains("15 marzo 2008"));
        assert!(!fact.ambiguous);
    }

    #[test]
    fn six_dates_survive_with_top_n_three() {
        // The plan's killer scenario: 6 dates in one text with a top-N=3
        // visual budget. TemporalFacts must preserve them all.
        let text = "Nel 1998 avvenne il primo evento, il 12 marzo 2001 arrivò la \
svolta, il 25 luglio 2005 tutto cambiò, il 2 giugno 2010 si consolidò, \
e il 7 settembre 2019 completò la storia.";
        let all = facts(text, "it");
        assert_grounded(text, &all);
        let resolved: Vec<&str> = all
            .iter()
            .filter(|fact| fact.precision != "unresolved")
            .map(|fact| fact.normalized.as_deref().unwrap_or(""))
            .collect();
        assert!(resolved.contains(&"1998"), "bare year must survive");
        assert!(resolved.contains(&"2001-03-12"));
        assert!(resolved.contains(&"2005-07-25"));
        assert!(resolved.contains(&"2010-06-02"));
        assert!(resolved.contains(&"2019-09-07"), "resolved: {resolved:?}");
        assert!(all.len() >= 5, "expected at least five facts, got: {all:?}");
    }

    #[test]
    fn english_month_day_comma_year() {
        let text = "On March 12, 2001 the deal closed.";
        let all = facts(text, "en");
        assert_grounded(text, &all);
        assert!(all
            .iter()
            .any(|fact| fact.normalized.as_deref() == Some("2001-03-12")));
    }

    #[test]
    fn ambiguous_numeric_uses_language_convention_and_flags() {
        // Italian day-first: 3 April 2020.
        let it = facts("Il 03/04/2020 cambiò tutto.", "it");
        let it_fact = it
            .iter()
            .find(|fact| fact.ambiguous)
            .expect("03/04/2020 must be flagged ambiguous");
        assert_eq!(it_fact.normalized.as_deref(), Some("2020-04-03"));
        assert_eq!(it_fact.precision, "day");

        // English month-first: March 4, 2020.
        let en = facts("On 03/04/2020 everything changed.", "en");
        let en_fact = en
            .iter()
            .find(|fact| fact.ambiguous)
            .expect("03/04/2020 must be flagged ambiguous");
        assert_eq!(en_fact.normalized.as_deref(), Some("2020-03-04"));
    }

    #[test]
    fn impossible_month_forces_day_first_unambiguously() {
        let en = facts("On 13/04/2020 the app launched.", "en");
        let fact = en
            .iter()
            .find(|fact| fact.normalized.as_deref() == Some("2020-04-13"))
            .expect("13 cannot be a month; day-first must win");
        assert_eq!(fact.precision, "day");
        assert!(!fact.ambiguous, "13/04/2020 is not order-ambiguous");
    }

    #[test]
    fn impossible_calendar_dates_stay_presentable_but_unresolved() {
        let all = facts("Il 31/02/2020 era una data impossibile.", "it");
        let fact = all
            .iter()
            .find(|fact| fact.surface.contains("31/02/2020"))
            .expect("the impossible surface must still be reported");
        assert_eq!(fact.precision, "unresolved");
        assert_eq!(fact.normalized, None, "no invented ISO value");
    }

    #[test]
    fn year_only_never_invents_day_or_month() {
        let all = facts("Nel 2001 tutto cambiò.", "it");
        let fact = all
            .iter()
            .find(|fact| fact.precision == "year")
            .expect("spoken year must be a year fact");
        assert_eq!(fact.normalized.as_deref(), Some("2001"));
        assert!(!fact.surface.contains("gennaio"), "no invented month");
    }

    #[test]
    fn iso_forms_with_dash_and_slash_delimiters() {
        let all = facts("Verified on 2020-02-29 by the archive.", "it");
        assert!(all
            .iter()
            .any(|fact| fact.normalized.as_deref() == Some("2020-02-29")));
        let slash = facts("Signed 2020/11/30 in the contract.", "it");
        assert!(slash
            .iter()
            .any(|fact| fact.normalized.as_deref() == Some("2020-11-30")));
    }

    #[test]
    fn month_year_without_day() {
        let text = "L'indagine proseguì fino a giugno 1988.";
        let all = facts(text, "it");
        let fact = all
            .iter()
            .find(|fact| fact.precision == "month")
            .expect("month+year must be a month fact");
        assert_eq!(fact.normalized.as_deref(), Some("1988-06"));
        assert!(fact.surface.contains("giugno 1988"));
        assert_grounded(text, &all);
    }

    #[test]
    fn era_years_are_classified() {
        let all = facts("Cadde nel 476 d.C. e nel 1453 d.C.", "it");
        // "d.C." is CE ("dopo Cristo"): no BCE era flag.
        assert!(
            all.iter()
                .filter(|fact| fact.surface.contains("1453"))
                .all(|fact| fact.era.is_none()),
            "d.C. years are CE: {:?}",
            all
        );
        let bc = facts("The archive dates to 44 BC.", "en");
        let fact = bc
            .iter()
            .find(|fact| fact.era == Some("BCE"))
            .expect("44 BC must be a BCE year fact");
        assert_eq!(fact.precision, "year");
    }

    #[test]
    fn day_month_without_year_is_unresolved_no_iso_invented() {
        let all = facts("La consegna fu il 12 marzo.", "it");
        let fact = all
            .iter()
            .find(|fact| fact.surface.contains("12 marzo"))
            .expect("day+month form must still be reported");
        assert_eq!(fact.normalized, None, "no invented year");
        assert_eq!(fact.precision, "unresolved");
    }

    #[test]
    fn portuguese_connector_form() {
        let text = "Nasceu em 12 de março de 2001.";
        let all = facts(text, "pt");
        assert_grounded(text, &all);
        assert!(all
            .iter()
            .any(|fact| fact.normalized.as_deref() == Some("2001-03-12")));
    }

    #[test]
    fn surfaces_come_from_the_original_not_the_folded_view() {
        let text = "Il 12 Marzo 2008 è la data ufficiale.";
        let all = facts(text, "it");
        assert_grounded(text, &all);
        assert!(all
            .iter()
            .any(|fact| fact.normalized.as_deref() == Some("2008-03-12")));
    }

    #[test]
    fn date_clusters_from_two_to_seven_all_survive() {
        // The Fase 2 acceptance ladder: 2..=7 nearby dates in one text, all
        // independent of the visual top-N budget.
        let years = ["1998", "2001", "2005", "2010", "2019", "2050", "2088"];
        for count in 2..=7 {
            let mut text = String::from("Inizio: ");
            for (i, year) in years.iter().take(count).enumerate() {
                if i > 0 {
                    text.push_str(", poi il ");
                }
                text.push_str(&format!("{} marzo", year));
                if count % 2 == 0 {
                    text.push_str(" 2008");
                }
            }
            text.push('.');
            let all = facts(&text, "it");
            assert_grounded(&text, &all);
            let resolved: Vec<Vec<String>> = all
                .iter()
                .map(|fact| {
                    fact.normalized
                        .as_deref()
                        .unwrap_or_default()
                        .split('-')
                        .map(|part| part.to_string())
                        .collect()
                })
                .collect();
            // The first `count` bare years must each survive as own facts.
            let year_values: Vec<&str> = all
                .iter()
                .filter_map(|fact| {
                    let normalized = fact.normalized.as_deref().unwrap_or("");
                    let head = normalized.split('-').next().unwrap_or("");
                    (years.contains(&head)).then_some(head)
                })
                .collect();
            for year in &years[..count] {
                assert!(
                    year_values.contains(year),
                    "count={count}: {year} lost, facts: {all:?}"
                );
            }
            assert!(
                all.len() >= count,
                "count={count}: expected at least {count} facts, got {all:?} resolved: {resolved:?}"
            );
        }
    }

    #[test]
    fn historic_years_normalize_without_era_confusion() {
        let text =
            "L'unificazione progredì nel 1848, la scoperta fu del 1492, la guerra finì nel 1943.";
        let all = facts(text, "it");
        assert_grounded(text, &all);
        let years: Vec<&str> = all
            .iter()
            .filter(|fact| fact.precision == "year")
            .filter_map(|fact| fact.normalized.as_deref())
            .collect();
        assert!(years.contains(&"1848"), "years: {years:?}");
        assert!(years.contains(&"1492"), "years: {years:?}");
        assert!(years.contains(&"1943"), "years: {years:?}");
        for fact in &all {
            assert!(fact.era.is_none(), "plain CE years carry no era: {fact:?}");
        }
    }

    #[test]
    fn narration_order_follows_offsets_not_chronology() {
        // «Nel 2005, ripensando al 1998...»: 2005 is spoken first even though
        // it is chronologically after 1998. Facts must follow offset order.
        let text = "Nel 2005, ripensando al 1998, e infine al 2010.";
        let all = facts(text, "it");
        assert_grounded(text, &all);
        let surfaces: Vec<&str> = all.iter().map(|f| f.surface.as_str()).collect();
        assert!(
            surfaces.len() >= 3
                && text.find(surfaces[0]).unwrap_or(usize::MAX)
                    < text.find(surfaces[1]).unwrap_or(usize::MAX)
                && text.find(surfaces[1]).unwrap_or(usize::MAX)
                    < text.find(surfaces[2]).unwrap_or(usize::MAX),
            "narration order must equal source order; surfaces: {surfaces:?}"
        );
        let first = all
            .iter()
            .find(|f| f.normalized.as_deref() == Some("2005"))
            .expect("2005 must be a fact");
        let second = all
            .iter()
            .find(|f| f.normalized.as_deref() == Some("1998"))
            .expect("1998 must be a fact");
        assert!(
            first.start_byte < second.start_byte,
            "spoken 2005 must come before remembered 1998"
        );
        for fact in &all {
            assert_eq!(
                text.get(fact.start_byte..fact.end_byte),
                Some(fact.surface.as_str()),
                "byte offsets must point at the verbatim surface"
            );
        }
    }

    #[test]
    fn facts_are_deterministic() {
        let text = "Primo il 12 marzo 2001, poi il 25 luglio 2005, infine l'8 febbraio 2019.";
        assert_eq!(facts(text, "it"), facts(text, "it"));
    }
}
