use std::fmt;
use std::sync::OnceLock;

use regex::Regex;

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
        Regex::new(
            r"(?i)\b(\d{1,2})\s+(gennaio|january|enero|janeiro|febbraio|february|febrero|fevereiro|marzo|march|março|abril|aprile|april|maggio|may|mayo|maio|giugno|june|junio|junho|luglio|july|julio|julho|agosto|august|settembre|september|septiembre|setiembre|setembro|ottobre|october|octubre|outubro|novembre|november|noviembre|novembro|dicembre|december|diciembre|dezembro)\s+(?:(?:del|de|do|di)\s+)?((?:19|20)\d{2})\b",
        )
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
    match name.to_lowercase().as_str() {
        "gennaio" | "january" | "enero" | "janeiro" => Some(1),
        "febbraio" | "february" | "febrero" | "fevereiro" => Some(2),
        "marzo" | "march" | "março" => Some(3),
        "aprile" | "april" | "abril" => Some(4),
        "maggio" | "may" | "mayo" | "maio" => Some(5),
        "giugno" | "june" | "junio" | "junho" => Some(6),
        "luglio" | "july" | "julio" | "julho" => Some(7),
        "agosto" | "august" => Some(8),
        "settembre" | "september" | "septiembre" | "setiembre" | "setembro" => Some(9),
        "ottobre" | "october" | "octubre" | "outubro" => Some(10),
        "novembre" | "november" | "noviembre" | "novembro" => Some(11),
        "dicembre" | "december" | "diciembre" | "dezembro" => Some(12),
        _ => None,
    }
}

fn valid_calendar_date(day: i32, month: i32, year: i32) -> bool {
    if day < 1 || !(1..=12).contains(&month) {
        return false;
    }
    day <= days_in_month(month, year)
}

fn days_in_month(month: i32, year: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
