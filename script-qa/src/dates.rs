use std::fmt;
use std::sync::OnceLock;

use regex::Regex;
use semantic_core::dates::days_in_month;

/// A complete numeric calendar date explicitly present in source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CalendarDate {
    pub day: i32,
    pub month: i32,
    pub year: i32,
}

impl fmt::Display for CalendarDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:02}-{:02}-{:04}",
            self.day, self.month, self.year
        )
    }
}

/// The month spellings the quality gate accepts, with their calendar month.
///
/// This is one table, not two: the regex alternation and the name lookup are
/// both derived from it, so a spelling can never be recognized by the pattern
/// and then fail to map to a month (or vice versa). It stays gate-scoped
/// (the five languages the Go gate accepts, including the historical
/// `setiembre` variant) rather than using the engine's wider CLDR table: the
/// gate must keep byte-for-byte parity with the Go implementation it ports.
const GATE_MONTHS: &[(&str, i32)] = &[
    ("gennaio", 1),
    ("january", 1),
    ("enero", 1),
    ("janeiro", 1),
    ("febbraio", 2),
    ("february", 2),
    ("febrero", 2),
    ("fevereiro", 2),
    ("marzo", 3),
    ("march", 3),
    ("março", 3),
    ("aprile", 4),
    ("abril", 4),
    ("april", 4),
    ("maggio", 5),
    ("mayo", 5),
    ("maio", 5),
    ("may", 5),
    ("giugno", 6),
    ("june", 6),
    ("junio", 6),
    ("junho", 6),
    ("luglio", 7),
    ("july", 7),
    ("julio", 7),
    ("julho", 7),
    ("agosto", 8),
    ("august", 8),
    ("settembre", 9),
    ("september", 9),
    ("septiembre", 9),
    ("setiembre", 9),
    ("setembro", 9),
    ("ottobre", 10),
    ("october", 10),
    ("octubre", 10),
    ("outubro", 10),
    ("novembre", 11),
    ("november", 11),
    ("noviembre", 11),
    ("novembro", 11),
    ("dicembre", 12),
    ("december", 12),
    ("diciembre", 12),
    ("dezembro", 12),
];

/// Extract complete dates written as `13 agosto 2007` (five supported month
/// languages) or `13/08/2007`, `13-08-2007`, `13.08.2007`. Years are limited
/// to 19xx/20xx, matching the Go gate.
pub fn explicit_calendar_dates(text: &str) -> Vec<CalendarDate> {
    let mut dates = Vec::new();

    for captures in month_pattern().captures_iter(text) {
        let Some(day) = captures.get(1).and_then(|m| m.as_str().parse::<i32>().ok()) else {
            continue;
        };
        let Some(month) = captures.get(2).and_then(|m| month_number(m.as_str())) else {
            continue;
        };
        let Some(year) = captures.get(3).and_then(|m| m.as_str().parse::<i32>().ok()) else {
            continue;
        };
        if valid_calendar_date(day, month, year) {
            dates.push(CalendarDate { day, month, year });
        }
    }

    for captures in numeric_pattern().captures_iter(text) {
        let Some(day) = captures.get(1).and_then(|m| m.as_str().parse::<i32>().ok()) else {
            continue;
        };
        let Some(month) = captures.get(2).and_then(|m| m.as_str().parse::<i32>().ok()) else {
            continue;
        };
        let Some(year) = captures.get(3).and_then(|m| m.as_str().parse::<i32>().ok()) else {
            continue;
        };
        if valid_calendar_date(day, month, year) {
            dates.push(CalendarDate { day, month, year });
        }
    }

    dates
}

fn month_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        let alternation = GATE_MONTHS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join("|");
        Regex::new(&format!(
            r"(?i)\b(\d{{1,2}})\s+({alternation})\s+(?:(?:del|de|do|di)\s+)?((?:19|20)\d{{2}})\b"
        ))
        .expect("calendar month regex")
    })
}

fn numeric_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"\b(\d{1,2})[./-](\d{1,2})[./-]((?:19|20)\d{2})\b")
            .expect("numeric calendar date regex")
    })
}

fn month_number(name: &str) -> Option<i32> {
    let lower = name.to_lowercase();
    GATE_MONTHS
        .iter()
        .find(|(candidate, _)| *candidate == lower)
        .map(|(_, month)| *month)
}

fn valid_calendar_date(day: i32, month: i32, year: i32) -> bool {
    if day < 1 || !(1..=12).contains(&month) {
        return false;
    }
    day <= i32::from(days_in_month(month as u8, year))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gate_month_name_maps_to_its_calendar_month() {
        for (name, month) in GATE_MONTHS {
            assert_eq!(
                explicit_calendar_dates(&format!("13 {name} 2007")),
                vec![CalendarDate {
                    day: 13,
                    month: *month,
                    year: 2007
                }],
                "month spelling {name}"
            );
        }
    }

    #[test]
    fn calendar_validation_uses_the_shared_gregorian_rule() {
        assert!(semantic_core::dates::is_leap_year(2024));
        assert!(!semantic_core::dates::is_leap_year(1900));
        assert_eq!(explicit_calendar_dates("29 febbraio 2024").len(), 1);
        assert!(explicit_calendar_dates("29 febbraio 2023").is_empty());
        assert!(explicit_calendar_dates("31 aprile 2024").is_empty());
    }
}
