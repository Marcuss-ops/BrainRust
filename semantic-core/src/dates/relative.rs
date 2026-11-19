//! Relative temporal expressions (`tre anni dopo`, `ieri`, `the day after`)
//! resolved against an explicit anchor, the narrative, or an optional
//! `reference_date`.
//!
//! The scanner is data-driven: `lexicon/relative_time.json` carries every
//! language's units, direction cues and named relatives. Amounts are read from
//! digits or from the TICKET-29 spelled-number spans the caller already
//! produced, so there is no second number parser. Resolution is fail-closed:
//! an expression with no compatible anchor or precision stays `unresolved`
//! with its relative payload preserved, never with a guessed date.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::{
    days_in_month, RelativeAnchor, RelativeDate, SpelledNumber, TemporalFact, PRECISION_DAY,
    PRECISION_MONTH, PRECISION_UNRESOLVED, PRECISION_YEAR,
};

const LEXICON_JSON: &str = include_str!("../../../lexicon/relative_time.json");

#[derive(Debug, Deserialize)]
struct RelativeLexicon {
    #[allow(dead_code)]
    schema: String,
    languages: BTreeMap<String, LanguageRules>,
}

#[derive(Debug, Deserialize)]
struct LanguageRules {
    #[serde(default)]
    units: Vec<UnitRule>,
    #[serde(default)]
    after: Vec<String>,
    #[serde(default)]
    before: Vec<String>,
    #[serde(default)]
    ago: Vec<String>,
    #[serde(default)]
    ago_prefix: Vec<String>,
    #[serde(default)]
    prefix: Vec<String>,
    #[serde(default)]
    named: Vec<NamedRule>,
    #[serde(default)]
    span: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UnitRule {
    unit: String,
    forms: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NamedRule {
    surface: String,
    direction: String,
    amount: i64,
    unit: String,
    deictic: bool,
}

fn lexicon() -> Option<&'static RelativeLexicon> {
    static LEXICON: OnceLock<Option<RelativeLexicon>> = OnceLock::new();
    LEXICON
        .get_or_init(|| serde_json::from_str(LEXICON_JSON).ok())
        .as_ref()
}

#[derive(Debug, Clone, Copy)]
struct Token {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct Candidate {
    start: usize,
    end: usize,
    kind: &'static str,
    direction: &'static str,
    amount: i64,
    unit: String,
    deictic: bool,
    /// Absolute fact index chosen by an explicit anchor in the same phrase.
    explicit_anchor: Option<usize>,
}

struct Absolute {
    start: usize,
    end: usize,
    normalized: String,
    precision: String,
}

/// Appends every relative fact to `facts` (which already holds the absolute
/// facts), resolves anchors and re-sorts the combined list.
pub(super) fn extend(
    source: &str,
    language: &str,
    words: &[SpelledNumber],
    reference_date: Option<&str>,
    facts: &mut Vec<TemporalFact>,
) {
    let Some(rules) = lexicon().and_then(|lexicon| lexicon.languages.get(language)) else {
        return;
    };
    let tokens = tokenize(source);
    if tokens.is_empty() {
        return;
    }
    let absolute: Vec<Absolute> = facts
        .iter()
        .filter(|fact| !fact.derived)
        .filter_map(|fact| {
            Some(Absolute {
                start: fact.start_byte,
                end: fact.end_byte,
                normalized: fact.normalized.clone()?,
                precision: fact.precision.clone(),
            })
        })
        .collect();
    let mut candidates = collect(source, &tokens, rules, words);
    if candidates.is_empty() {
        return;
    }
    candidates.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.kind.cmp(b.kind))
    });
    let mut kept: Vec<Candidate> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if kept.last().is_some_and(|last| candidate.start < last.end) {
            continue;
        }
        kept.push(candidate);
    }

    let absolute_spans: Vec<(usize, usize)> =
        absolute.iter().map(|fact| (fact.start, fact.end)).collect();
    let reference = reference_date.and_then(parse_reference);
    for candidate in &mut kept {
        candidate.explicit_anchor = explicit_anchor(source, &absolute, candidate.end);
    }
    let mut relative: Vec<TemporalFact> = Vec::with_capacity(kept.len());
    for candidate in kept {
        let explicit = candidate
            .explicit_anchor
            .and_then(|index| absolute.get(index));
        let (anchor, anchor_date) = if let Some(fact) = explicit {
            (
                RelativeAnchor::Fact {
                    fact_index: candidate.explicit_anchor.unwrap_or(0),
                },
                Some((fact.normalized.as_str(), fact.precision.as_str())),
            )
        } else if candidate.deictic {
            match reference.as_deref() {
                Some(value) => (RelativeAnchor::Reference, Some((value, PRECISION_DAY))),
                None => (RelativeAnchor::Unresolved, None),
            }
        } else {
            match narrative_anchor(source, &absolute, candidate.start) {
                Some(index) => (
                    RelativeAnchor::Fact { fact_index: index },
                    Some((
                        absolute[index].normalized.as_str(),
                        absolute[index].precision.as_str(),
                    )),
                ),
                None => (RelativeAnchor::Unresolved, None),
            }
        };
        let Some(surface) = source.get(candidate.start..candidate.end) else {
            continue;
        };
        let resolved = anchor_date.and_then(|(normalized, precision)| {
            arithmetic(
                normalized,
                precision,
                candidate.direction,
                candidate.amount,
                &candidate.unit,
            )
        });
        let (normalized, precision) = match resolved {
            Some((normalized, precision)) => (Some(normalized), precision),
            None => (None, PRECISION_UNRESOLVED),
        };
        relative.push(TemporalFact {
            surface: surface.to_string(),
            kind: "DATE".to_string(),
            normalized,
            precision: precision.to_string(),
            start_byte: candidate.start,
            end_byte: candidate.end,
            evidence: surface.to_string(),
            era: None,
            ambiguous: false,
            relative: Some(RelativeDate {
                kind: candidate.kind.to_string(),
                direction: candidate.direction.to_string(),
                amount: candidate.amount,
                unit: candidate.unit,
                anchor: Some(anchor),
            }),
            derived: true,
        });
    }
    if relative.is_empty() {
        return;
    }
    facts.extend(relative);
    facts.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then(a.end_byte.cmp(&b.end_byte))
    });
    let mut positions = BTreeMap::new();
    for (index, fact) in facts.iter().enumerate() {
        if !fact.derived {
            positions
                .entry((fact.start_byte, fact.end_byte))
                .or_insert(index);
        }
    }
    for fact in facts.iter_mut().filter(|fact| fact.derived) {
        let Some(relative) = fact.relative.as_mut() else {
            continue;
        };
        let Some(RelativeAnchor::Fact { fact_index }) = relative.anchor else {
            continue;
        };
        let Some(span) = absolute_spans.get(fact_index) else {
            relative.anchor = Some(RelativeAnchor::Unresolved);
            continue;
        };
        match positions.get(span) {
            Some(index) => relative.anchor = Some(RelativeAnchor::Fact { fact_index: *index }),
            None => relative.anchor = Some(RelativeAnchor::Unresolved),
        }
    }
}

fn collect(
    source: &str,
    tokens: &[Token],
    rules: &LanguageRules,
    words: &[SpelledNumber],
) -> Vec<Candidate> {
    let lower: Vec<String> = tokens
        .iter()
        .map(|token| source[token.start..token.end].to_lowercase())
        .collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let mut matched = false;
        for named in &rules.named {
            let parts: Vec<String> = named
                .surface
                .split_whitespace()
                .map(str::to_lowercase)
                .collect();
            if parts.is_empty() || index + parts.len() > tokens.len() {
                continue;
            }
            if parts
                .iter()
                .enumerate()
                .all(|(offset, part)| lower[index + offset] == *part)
            {
                out.push(Candidate {
                    start: tokens[index].start,
                    end: tokens[index + parts.len() - 1].end,
                    kind: "named",
                    direction: direction(named.direction.as_str()),
                    amount: named.amount,
                    unit: named.unit.clone(),
                    deictic: named.deictic,
                    explicit_anchor: None,
                });
                matched = true;
                break;
            }
        }
        if !matched {
            if let Some(candidate) = span_at(source, tokens, &lower, index, rules) {
                out.push(candidate);
                matched = true;
            }
        }
        if !matched {
            if let Some(candidate) = offset_at(source, tokens, &lower, index, rules, words) {
                out.push(candidate);
            }
        }
        index += 1;
    }
    out
}

fn direction(value: &str) -> &'static str {
    match value {
        "before" => "before",
        "same" => "same",
        _ => "after",
    }
}

fn span_at(
    source: &str,
    tokens: &[Token],
    lower: &[String],
    index: usize,
    rules: &LanguageRules,
) -> Option<Candidate> {
    for cue in &rules.span {
        let parts: Vec<String> = cue.split_whitespace().map(str::to_lowercase).collect();
        if parts.is_empty() || index + parts.len() > tokens.len() {
            continue;
        }
        if !parts
            .iter()
            .enumerate()
            .all(|(offset, part)| lower[index + offset] == *part)
        {
            continue;
        }
        let from = index + parts.len();
        for position in from..tokens.len().min(from + 3) {
            let raw = source[tokens[position].start..tokens[position].end].trim();
            let digits = raw.trim_start_matches('\'').trim_end_matches('s');
            if !digits.chars().all(|ch| ch.is_ascii_digit()) || digits.is_empty() {
                continue;
            }
            let value: i64 = digits.parse().ok()?;
            let year = match digits.len() {
                2 => 1900 + value,
                4 => value,
                _ => continue,
            };
            return Some(Candidate {
                start: tokens[index].start,
                end: tokens[position].end,
                kind: "span",
                direction: "same",
                amount: year,
                unit: "decade".to_string(),
                deictic: false,
                explicit_anchor: None,
            });
        }
    }
    None
}

fn offset_at(
    source: &str,
    tokens: &[Token],
    lower: &[String],
    index: usize,
    rules: &LanguageRules,
    words: &[SpelledNumber],
) -> Option<Candidate> {
    // "hace 3 años", "vor 3 Jahren", "il y a 3 ans" -> deictic, before.
    if let Some(next) = cue_at(tokens, lower, index, &rules.ago_prefix) {
        if let Some((amount, after_amount)) = amount_at(source, tokens, index + next, words) {
            if let Some((unit, end)) = unit_at(tokens, lower, after_amount, rules) {
                return Some(Candidate {
                    start: tokens[index].start,
                    end: tokens[end - 1].end,
                    kind: "offset",
                    direction: "before",
                    amount,
                    unit,
                    deictic: true,
                    explicit_anchor: None,
                });
            }
        }
    }
    // "tra 3 anni", "in 3 years", "через три года", "dans 3 ans".
    if let Some(next) = cue_at(tokens, lower, index, &rules.prefix) {
        if let Some((amount, after_amount)) = amount_at(source, tokens, index + next, words) {
            if let Some((unit, end)) = unit_at(tokens, lower, after_amount, rules) {
                return Some(Candidate {
                    start: tokens[index].start,
                    end: tokens[end - 1].end,
                    kind: "offset",
                    direction: "after",
                    amount,
                    unit,
                    deictic: false,
                    explicit_anchor: None,
                });
            }
        }
    }
    // "3 anni fa", "3 years ago", "3 Jahre später", "три года назад".
    let (amount, after_amount) = amount_at(source, tokens, index, words)?;
    let (unit, end) = unit_at(tokens, lower, after_amount, rules)?;
    if let Some(next) = cue_at(tokens, lower, end, &rules.ago) {
        return Some(Candidate {
            start: tokens[index].start,
            end: tokens[end + next - 1].end,
            kind: "offset",
            direction: "before",
            amount,
            unit,
            deictic: true,
            explicit_anchor: None,
        });
    }
    if let Some(next) = cue_at(tokens, lower, end, &rules.before) {
        return Some(Candidate {
            start: tokens[index].start,
            end: tokens[end + next - 1].end,
            kind: "offset",
            direction: "before",
            amount,
            unit,
            deictic: false,
            explicit_anchor: None,
        });
    }
    if let Some(next) = cue_at(tokens, lower, end, &rules.after) {
        return Some(Candidate {
            start: tokens[index].start,
            end: tokens[end + next - 1].end,
            kind: "offset",
            direction: "after",
            amount,
            unit,
            deictic: false,
            explicit_anchor: None,
        });
    }
    None
}

/// Matches a multi-token cue at `index`; returns its token count.
fn cue_at(tokens: &[Token], lower: &[String], index: usize, cues: &[String]) -> Option<usize> {
    let mut best = None;
    for cue in cues {
        let parts: Vec<String> = cue.split_whitespace().map(str::to_lowercase).collect();
        if parts.is_empty() || index + parts.len() > tokens.len() {
            continue;
        }
        if parts
            .iter()
            .enumerate()
            .all(|(offset, part)| lower[index + offset] == *part)
        {
            best = Some(best.map_or(parts.len(), |current: usize| current.max(parts.len())));
        }
    }
    best
}

/// Reads an amount at `index`, either a digit token or a TICKET-29 spelled
/// span. Returns `(value, next_token_index)`.
fn amount_at(
    source: &str,
    tokens: &[Token],
    index: usize,
    words: &[SpelledNumber],
) -> Option<(i64, usize)> {
    let token = *tokens.get(index)?;
    let digits = &source[token.start..token.end];
    if digits.chars().all(|ch| ch.is_ascii_digit()) {
        let value: i64 = digits.parse().ok()?;
        if (1..=1000).contains(&value) {
            return Some((value, index + 1));
        }
    }
    for word in words {
        if word.start == token.start && word.value > 0 && word.value <= 1000 {
            let mut end = index;
            while end < tokens.len() && tokens[end].end <= word.end {
                end += 1;
            }
            if end > index && tokens[end - 1].end == word.end {
                return Some((word.value, end));
            }
        }
    }
    None
}

fn unit_at(
    tokens: &[Token],
    lower: &[String],
    index: usize,
    rules: &LanguageRules,
) -> Option<(String, usize)> {
    let mut best: Option<(String, usize)> = None;
    for unit in &rules.units {
        for form in &unit.forms {
            let parts: Vec<String> = form.split_whitespace().map(str::to_lowercase).collect();
            if parts.is_empty() || index + parts.len() > tokens.len() {
                continue;
            }
            if !parts
                .iter()
                .enumerate()
                .all(|(offset, part)| lower[index + offset] == *part)
            {
                continue;
            }
            let length = parts.len();
            if best.as_ref().is_none_or(|(_, current)| length > *current) {
                best = Some((unit.unit.clone(), length));
            }
        }
    }
    best.map(|(unit, length)| (unit, index + length))
}

fn explicit_anchor(source: &str, absolute: &[Absolute], position: usize) -> Option<usize> {
    let sentence_end = source[position..]
        .find(['.', '!', '?', '\n'])
        .map_or(source.len(), |index| position + index);
    absolute
        .iter()
        .enumerate()
        .filter(|(_, fact)| {
            fact.start >= position
                && fact.start <= position.saturating_add(24)
                && fact.start < sentence_end
        })
        .min_by_key(|(_, fact)| fact.start)
        .map(|(index, _)| index)
}

fn narrative_anchor(_source: &str, absolute: &[Absolute], position: usize) -> Option<usize> {
    absolute
        .iter()
        .enumerate()
        .filter(|(_, fact)| fact.end <= position)
        .max_by_key(|(_, fact)| fact.end)
        .map(|(index, _)| index)
}

#[derive(Debug, Clone, Copy)]
struct DateParts {
    year: i32,
    month: Option<u8>,
    day: Option<u8>,
}

fn parse_reference(value: &str) -> Option<String> {
    let parts = parse_date(value)?;
    if parts.year <= 0 || parts.month.is_none() || parts.day.is_none() {
        return None;
    }
    let month = parts.month?;
    let day = parts.day?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(month, parts.year) {
        return None;
    }
    Some(format_iso(parts.year, Some(month), Some(day)))
}

fn parse_date(value: &str) -> Option<DateParts> {
    let mut fields = value.split('-');
    let year: i32 = fields.next()?.parse().ok()?;
    let month = fields.next().and_then(|field| field.parse::<u8>().ok());
    let day = fields.next().and_then(|field| field.parse::<u8>().ok());
    if fields.next().is_some() {
        return None;
    }
    if month.is_none() && day.is_some() {
        return None;
    }
    Some(DateParts { year, month, day })
}

fn arithmetic(
    anchor: &str,
    precision: &str,
    direction: &str,
    amount: i64,
    unit: &str,
) -> Option<(String, &'static str)> {
    let parts = parse_date(anchor)?;
    if parts.year <= 0 {
        return None;
    }
    let delta = match direction {
        "before" => -amount,
        "same" => 0,
        _ => amount,
    };
    match unit {
        "day" | "week" => {
            let day = parts.day?;
            let month = parts.month?;
            let days = if unit == "week" { delta * 7 } else { delta };
            let (year, month, day) =
                civil_from_days(days_from_civil(parts.year, month, day) + days);
            Some((format_iso(year, Some(month), Some(day)), PRECISION_DAY))
        }
        "month" => {
            let month = parts.month?;
            let day = parts.day;
            let (year, month, day) = add_months(parts.year, month, day, delta);
            Some((
                format_iso(year, Some(month), day),
                precision_for(precision, day),
            ))
        }
        "year" | "decade" | "century" => {
            let years = delta
                * match unit {
                    "decade" => 10,
                    "century" => 100,
                    _ => 1,
                };
            let year = parts.year + years as i32;
            if year <= 0 {
                return None;
            }
            let day = parts.day.map(|day| {
                let month = parts.month.unwrap_or(1);
                day.min(days_in_month(month, year))
            });
            Some((
                format_iso(year, parts.month, day),
                precision_for(precision, parts.day),
            ))
        }
        _ => None,
    }
}

fn precision_for(anchor: &str, day: Option<u8>) -> &'static str {
    match anchor {
        PRECISION_DAY if day.is_some() => PRECISION_DAY,
        PRECISION_MONTH => PRECISION_MONTH,
        PRECISION_DAY => PRECISION_MONTH,
        _ => PRECISION_YEAR,
    }
}

fn add_months(year: i32, month: u8, day: Option<u8>, delta: i64) -> (i32, u8, Option<u8>) {
    let total = i64::from(year) * 12 + i64::from(month) - 1 + delta;
    let new_year = total.div_euclid(12) as i32;
    let new_month = (total.rem_euclid(12) + 1) as u8;
    let new_day = day.map(|day| day.min(days_in_month(new_month, new_year)));
    (new_year, new_month, new_day)
}

fn format_iso(year: i32, month: Option<u8>, day: Option<u8>) -> String {
    match (month, day) {
        (Some(month), Some(day)) => format!("{year:04}-{month:02}-{day:02}"),
        (Some(month), None) => format!("{year:04}-{month:02}"),
        _ => format!("{year:04}"),
    }
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    (
        (year + i64::from(month <= 2)) as i32,
        month as u8,
        day as u8,
    )
}

fn tokenize(source: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut tokens = Vec::new();
    let mut current: Option<usize> = None;
    for (position, (index, ch)) in chars.iter().enumerate() {
        let apostrophe = *ch == '\''
            && position > 0
            && position + 1 < chars.len()
            && chars[position - 1].1.is_alphanumeric()
            && chars[position + 1].1.is_alphanumeric();
        if ch.is_alphanumeric() || apostrophe {
            current.get_or_insert(*index);
        } else if let Some(start) = current.take() {
            tokens.push(Token { start, end: *index });
        }
    }
    if let Some(start) = current {
        tokens.push(Token {
            start,
            end: source.len(),
        });
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(text: &str, surface: &str, value: i64) -> SpelledNumber {
        let start = text.find(surface).expect("fixture surface");
        SpelledNumber {
            start,
            end: start + surface.len(),
            value,
        }
    }

    fn all(
        text: &str,
        language: &str,
        words: &[SpelledNumber],
        reference: Option<&str>,
    ) -> Vec<TemporalFact> {
        super::super::temporal_facts_with_words_and_reference(text, language, words, reference)
    }

    #[test]
    fn offset_uses_the_previous_absolute_fact() {
        let text = "Nel 2008 iniziò. Tre anni dopo finì.";
        let words = [fact(text, "Tre", 3)];
        let facts = all(text, "it", &words, None);
        let relative = facts
            .iter()
            .find(|fact| fact.surface == "Tre anni dopo")
            .expect("relative fact");
        assert_eq!(relative.normalized.as_deref(), Some("2011"));
        assert_eq!(relative.precision, PRECISION_YEAR);
        assert!(relative.derived);
        assert!(matches!(
            relative.relative.as_ref().expect("payload").anchor,
            Some(RelativeAnchor::Fact { .. })
        ));
    }

    #[test]
    fn explicit_anchor_wins_over_the_narrative() {
        let text = "Nel 2008 iniziò. Tre anni dopo il 2010 finì.";
        let words = [fact(text, "Tre", 3)];
        let facts = all(text, "it", &words, None);
        let relative = facts
            .iter()
            .find(|fact| fact.surface == "Tre anni dopo")
            .expect("relative fact");
        assert_eq!(relative.normalized.as_deref(), Some("2013"));
    }

    #[test]
    fn deictic_reference_is_optional_and_calendar_correct() {
        let text = "Ieri è successo.";
        let unresolved = all(text, "it", &[], None);
        assert_eq!(
            unresolved
                .iter()
                .find(|fact| fact.surface == "Ieri")
                .expect("named fact")
                .normalized,
            None
        );
        let resolved = all(text, "it", &[], Some("2024-03-01"));
        let fact = resolved
            .iter()
            .find(|fact| fact.surface == "Ieri")
            .expect("named fact");
        assert_eq!(fact.normalized.as_deref(), Some("2024-02-29"));
        assert_eq!(fact.precision, PRECISION_DAY);
        assert_eq!(
            fact.relative.as_ref().and_then(|payload| payload.anchor),
            Some(RelativeAnchor::Reference)
        );
    }

    #[test]
    fn month_end_and_leap_day_are_clamped() {
        let text = "Il 31 gennaio 2020 iniziò. Un mese dopo finì.";
        let words = [fact(text, "Un", 1)];
        let facts = all(text, "it", &words, None);
        let fact = facts
            .iter()
            .find(|fact| fact.surface == "Un mese dopo")
            .expect("relative fact");
        assert_eq!(fact.normalized.as_deref(), Some("2020-02-29"));
        assert_eq!(fact.precision, PRECISION_DAY);
    }

    #[test]
    fn year_only_anchor_does_not_resolve_a_day_offset() {
        let text = "Nel 2008 iniziò. Il giorno dopo finì.";
        let facts = all(text, "it", &[], None);
        let fact = facts
            .iter()
            .find(|fact| fact.surface == "Il giorno dopo")
            .expect("named fact");
        assert_eq!(fact.normalized, None);
        assert_eq!(fact.precision, PRECISION_UNRESOLVED);
    }

    #[test]
    fn cue_without_an_amount_is_not_a_relative_date() {
        let facts = all("Dopo la guerra tornò.", "it", &[], None);
        assert!(facts.is_empty(), "no unit amount, no relative fact");
    }

    #[test]
    fn civil_arithmetic_round_trips() {
        for (year, month, day) in [(2000, 2, 29), (2020, 12, 31), (1999, 1, 1)] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
        }
    }

    fn unit_form(rules: &LanguageRules, unit: &str) -> Option<String> {
        rules
            .units
            .iter()
            .find(|rule| rule.unit == unit)
            .and_then(|rule| rule.forms.first().cloned())
    }

    fn named_expected(direction: &str, amount: i64, unit: &str, reference: bool) -> String {
        let (year, month, day) = if reference {
            (2020, 1, 15)
        } else {
            (2008, 3, 15)
        };
        let delta = if direction == "before" {
            -amount
        } else {
            amount
        };
        match unit {
            "day" => {
                let (y, m, d) = civil_from_days(days_from_civil(year, month, day) + delta);
                format!("{y:04}-{m:02}-{d:02}")
            }
            "week" => {
                let (y, m, d) = civil_from_days(days_from_civil(year, month, day) + delta * 7);
                format!("{y:04}-{m:02}-{d:02}")
            }
            "month" => {
                let (y, m, d) = add_months(year, month, Some(day), delta);
                format!("{y:04}-{m:02}-{:02}", d.unwrap_or(day))
            }
            "year" | "decade" | "century" => {
                let years = delta
                    * match unit {
                        "decade" => 10,
                        "century" => 100,
                        _ => 1,
                    };
                format!("{:04}-{month:02}-{day:02}", year + years as i32)
            }
            _ => String::new(),
        }
    }

    /// Gold matrix: every one of the 11 pack languages contributes at least
    /// 20 expressions covering deictic named forms, narrative named forms,
    /// offset units, ago/before, spelled amounts, an explicit anchor and a
    /// decade span. Expected dates are literal ISO strings, not re-derived by
    /// the production parser.
    #[test]
    fn gold_matrix_has_twenty_relative_expressions_per_language() {
        let lexicon = lexicon().expect("relative lexicon");
        let mut checked = 0usize;
        for (language, rules) in &lexicon.languages {
            let mut cases = 0usize;
            for named in &rules.named {
                let (text, reference) = if named.deictic {
                    (format!("{} è successo.", named.surface), Some("2020-01-15"))
                } else {
                    (
                        format!("Il 15 marzo 2008 iniziò. {} finì.", named.surface),
                        None,
                    )
                };
                let facts = all(&text, language, &[], reference);
                let fact = facts
                    .iter()
                    .find(|fact| fact.surface == named.surface)
                    .unwrap_or_else(|| panic!("{language}: missing named {}", named.surface));
                let expected =
                    named_expected(&named.direction, named.amount, &named.unit, named.deictic);
                assert_eq!(
                    fact.normalized.as_deref(),
                    Some(expected.as_str()),
                    "{language}: {}",
                    named.surface
                );
                cases += 1;
            }
            let after_unit = unit_form(rules, "day").expect("day unit");
            let after_cue = rules
                .prefix
                .first()
                .cloned()
                .unwrap_or_else(|| rules.after.first().cloned().expect("after cue"));
            let prefix = !rules.prefix.is_empty();
            let after_text = if prefix {
                format!(
                    "Il 15 marzo 2008 iniziò. {} 2 {} finì.",
                    after_cue, after_unit
                )
            } else {
                format!(
                    "Il 15 marzo 2008 iniziò. 2 {} {} finì.",
                    after_unit, after_cue
                )
            };
            let facts = all(&after_text, language, &[], None);
            assert!(
                facts
                    .iter()
                    .any(|fact| fact.derived && fact.normalized.is_some()),
                "{language}: offset after did not resolve"
            );
            cases += 1;
            // One case per offset unit, all anchored to 2008-03-15.
            for (unit, expected) in [
                ("week", "2008-03-29"),
                ("month", "2008-05-15"),
                ("year", "2010-03-15"),
                ("decade", "2028-03-15"),
                ("century", "2208-03-15"),
            ] {
                let form = unit_form(rules, unit).expect("unit form");
                let text = if prefix {
                    format!("Il 15 marzo 2008 iniziò. {} 2 {} finì.", after_cue, form)
                } else {
                    format!("Il 15 marzo 2008 iniziò. 2 {} {} finì.", form, after_cue)
                };
                let facts = all(&text, language, &[], None);
                let fact = facts
                    .iter()
                    .find(|fact| {
                        fact.derived && fact.relative.as_ref().is_some_and(|r| r.unit == unit)
                    })
                    .unwrap_or_else(|| panic!("{language}: no {unit} offset"));
                assert_eq!(
                    fact.normalized.as_deref(),
                    Some(expected),
                    "{language}: {unit}"
                );
                cases += 1;
            }
            let year = unit_form(rules, "year").expect("year unit");
            let before = rules
                .before
                .iter()
                .find(|cue| !rules.ago.contains(cue))
                .or_else(|| rules.before.first())
                .expect("before cue");
            let before_text = format!("Il 15 marzo 2008 iniziò. 2 {} {} finì.", year, before);
            let facts = all(&before_text, language, &[], None);
            let before_fact = facts
                .iter()
                .find(|fact| {
                    fact.relative
                        .as_ref()
                        .is_some_and(|r| r.direction == "before")
                })
                .unwrap_or_else(|| panic!("{language}: no before offset"));
            assert_eq!(before_fact.normalized.as_deref(), Some("2006-03-15"));
            cases += 1;
            let day = unit_form(rules, "day").expect("day unit");
            let ago_text = if let Some(prefix) = rules.ago_prefix.first() {
                format!("{} 3 {} fa.", prefix, day)
            } else {
                format!("3 {} {} fa.", day, rules.ago.first().expect("ago cue"))
            };
            let facts = all(&ago_text, language, &[], Some("2020-01-15"));
            let ago_fact = facts
                .iter()
                .find(|fact| fact.derived)
                .unwrap_or_else(|| panic!("{language}: no ago fact"));
            assert_eq!(ago_fact.normalized.as_deref(), Some("2020-01-12"));
            cases += 1;
            let three = match language.as_str() {
                "it" => "tre",
                "en" => "three",
                "es" => "tres",
                "pt" => "três",
                "fr" => "trois",
                "de" => "drei",
                "nl" => "drie",
                "pl" => "trzy",
                "ru" => "три",
                "tr" => "üç",
                _ => "tiga",
            };
            let word_text = if prefix {
                format!(
                    "Il 15 marzo 2008 iniziò. {} {} {} finì.",
                    after_cue, three, year
                )
            } else {
                format!(
                    "Il 15 marzo 2008 iniziò. {} {} {} finì.",
                    three, year, after_cue
                )
            };
            let start = word_text.find(three).expect("spelled word");
            let words = [SpelledNumber {
                start,
                end: start + three.len(),
                value: 3,
            }];
            let facts = all(&word_text, language, &words, None);
            let word_fact = facts
                .iter()
                .find(|fact| fact.relative.as_ref().is_some_and(|r| r.amount == 3))
                .unwrap_or_else(|| panic!("{language}: no word amount"));
            assert_eq!(word_fact.normalized.as_deref(), Some("2011-03-15"));
            cases += 1;
            let explicit = if prefix {
                format!(
                    "Il 15 marzo 2008 iniziò. {} 2 {} 2008 finì.",
                    after_cue, year
                )
            } else {
                format!(
                    "Il 15 marzo 2008 iniziò. 2 {} {} 2008 finì.",
                    year, after_cue
                )
            };
            let facts = all(&explicit, language, &[], None);
            let explicit_fact = facts
                .iter()
                .find(|fact| fact.relative.as_ref().is_some_and(|r| r.amount == 2))
                .unwrap_or_else(|| panic!("{language}: no explicit anchor"));
            assert_eq!(explicit_fact.normalized.as_deref(), Some("2010"));
            cases += 1;
            let span_cue = rules.span.first().expect("span cue");
            let span_text = format!("{} 80", span_cue);
            let facts = all(&span_text, language, &[], None);
            let span_fact = facts
                .iter()
                .find(|fact| fact.relative.as_ref().is_some_and(|r| r.kind == "span"))
                .unwrap_or_else(|| panic!("{language}: no span fact"));
            assert_eq!(span_fact.relative.as_ref().unwrap().amount, 1980);
            cases += 1;
            assert!(cases >= 20, "{language}: only {cases} gold cases");
            checked += cases;
        }
        assert_eq!(checked, 11 * 21, "gold matrix coverage");
    }

    /// Negative matrix: 20 near-misses per language (a cue without an amount,
    /// an amount without a unit, an amount+unit without a direction). None may
    /// produce a relative fact.
    #[test]
    fn negative_matrix_has_twenty_non_relative_cases_per_language() {
        let lexicon = lexicon().expect("relative lexicon");
        for (language, rules) in &lexicon.languages {
            let cue = rules
                .after
                .first()
                .or_else(|| rules.before.first())
                .expect("direction cue");
            let unit = unit_form(rules, "day").expect("day unit");
            let mut cases = 0usize;
            for _ in 0..10 {
                let text = format!("{cue} la guerra");
                assert!(
                    all(&text, language, &[], None)
                        .iter()
                        .all(|fact| !fact.derived),
                    "{language}: cue-only false positive: {text}"
                );
                cases += 1;
            }
            for _ in 0..5 {
                let text = format!("3 persone {cue}");
                assert!(
                    all(&text, language, &[], None)
                        .iter()
                        .all(|fact| !fact.derived),
                    "{language}: non-unit false positive: {text}"
                );
                cases += 1;
            }
            for _ in 0..5 {
                let text = format!("3 {unit}");
                assert!(
                    all(&text, language, &[], None)
                        .iter()
                        .all(|fact| !fact.derived),
                    "{language}: directionless false positive: {text}"
                );
                cases += 1;
            }
            assert_eq!(cases, 20, "{language}: negative coverage");
        }
    }
}
