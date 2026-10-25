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
    temporal_facts_with_words(source_text, language, &[])
}

/// One number written in words, located by the locale's ICU spellout rules.
/// The date parser receives these spans instead of a per-language word table,
/// so every locale with CLDR RBNF rules can spell days and years.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpelledNumber {
    pub start: usize,
    pub end: usize,
    pub value: i64,
}

/// Same as [`temporal_facts`] with the caller's spelled-number spans.
pub fn temporal_facts_with_words(
    source_text: &str,
    language: &str,
    words: &[SpelledNumber],
) -> Vec<TemporalFact> {
    let language = language.trim().to_lowercase();
    let mut raw: Vec<RawFact> = numeric::collect_numeric_facts(source_text, &language);
    raw.extend(month_word::collect_month_word_facts(
        source_text,
        &language,
        words,
    ));
    raw.extend(words::collect_spelled_year_facts(source_text, words));
    raw.extend(era::collect_era_years(source_text));
    resolve::resolve(source_text, raw)
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

use Era::Ce;

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

mod era;
mod month_word;
mod numeric;
mod resolve;
mod scan;
mod words;

#[cfg(test)]
mod tests;
