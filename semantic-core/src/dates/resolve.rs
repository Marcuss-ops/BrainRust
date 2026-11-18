//! Resolution pass: offset ordering, nested-span dropping, calendar
//! validation and the wire materialization of the temporal facts.

use super::Era::Bce;
use super::{
    days_in_month, RawFact, TemporalFact, PRECISION_DAY, PRECISION_MONTH, PRECISION_UNRESOLVED,
    PRECISION_YEAR,
};

//–––––––––––––––––––––––––– Resolution pass –––––––––––––––––––––––––––––––

/// Sorts by offset, drops facts nested inside a wider fact, validates the
/// calendar position, and materializes the wire facts with verbatim surface
/// evidence.
pub(super) fn resolve(source: &str, mut facts: Vec<RawFact>) -> Vec<TemporalFact> {
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
            precision: precision.to_string(),
            start_byte: fact.start,
            end_byte: fact.end,
            evidence: surface.to_string(),
            era: (fact.era == Bce && !impossible).then(|| "BCE".to_string()),
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
