//! Grounded numeric and date surface validation for raw projection.

use super::*;

pub(super) fn clean_numeric_value(value: &str, lexicon: &NumericLexiconV2) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if is_numeric_value(value, lexicon) {
        return value.to_string();
    }
    let mut trimmed = value.to_string();
    while !is_numeric_value(&trimmed, lexicon) {
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() < 2 {
            break;
        }
        trimmed = fields[..fields.len() - 1].join(" ");
    }
    if is_numeric_value(&trimmed, lexicon) {
        return trimmed;
    }
    let mut parts: Vec<&str> = value
        .split(|ch: char| ch.is_whitespace() || ch == '\'' || ch == '\u{2019}')
        .filter(|part| !part.is_empty())
        .collect();
    while parts.len() > 1 {
        parts.remove(0);
        let candidate = parts.join(" ");
        if is_numeric_value(&candidate, lexicon) && starts_with_number(&candidate, lexicon) {
            return candidate;
        }
    }
    if parts.len() == 1
        && is_numeric_value(parts[0], lexicon)
        && starts_with_number(parts[0], lexicon)
    {
        return parts[0].to_string();
    }
    String::new()
}

pub(super) fn is_numeric_value(value: &str, lexicon: &NumericLexiconV2) -> bool {
    let words: BTreeSet<String> = lexicon
        .words
        .iter()
        .map(|word| word.trim().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    let context: BTreeSet<String> = lexicon
        .context
        .iter()
        .map(|word| word.trim().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    let mut token = String::new();
    let mut saw_number = false;
    let mut valid = true;
    let flush = |token: &mut String, saw_number: &mut bool, valid: &mut bool| {
        if token.is_empty() {
            return;
        }
        let word = token.to_lowercase();
        token.clear();
        if words.contains(&word) {
            *saw_number = true;
        } else if !context.contains(&word) {
            *valid = false;
        }
    };
    for ch in value.chars() {
        if ch.is_numeric() {
            saw_number = true;
        } else if ch.is_alphabetic() {
            token.push(ch);
        } else {
            flush(&mut token, &mut saw_number, &mut valid);
        }
    }
    flush(&mut token, &mut saw_number, &mut valid);
    valid && saw_number
}

pub(super) fn starts_with_number(value: &str, lexicon: &NumericLexiconV2) -> bool {
    let Some(first) = value.split_whitespace().next() else {
        return false;
    };
    let first = first.trim_matches(|ch: char| {
        matches!(
            ch,
            '\'' | '\u{2019}' | '.' | '-' | ',' | '%' | '€' | '$' | '£' | '¥'
        )
    });
    if first.is_empty() {
        return false;
    }
    if first.chars().any(|ch| ch.is_numeric()) {
        return true;
    }
    if first.chars().any(|ch| !ch.is_alphabetic()) {
        return false;
    }
    lexicon
        .words
        .iter()
        .any(|word| word.eq_ignore_ascii_case(first))
}

pub(super) fn recover_numeric_suffix(text: &str, value: &str, mentions: &[RawMentionV2]) -> String {
    let Some(last) = value.chars().last() else {
        return value.to_string();
    };
    if !last.is_numeric() {
        return value.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for mention in mentions {
        let start = mention.start_rune.max(0) as usize;
        let end = mention.end_rune.max(0) as usize;
        if start >= end || end > chars.len() {
            continue;
        }
        let segment: String = chars[start..end].iter().collect();
        let Some(byte_at) = segment.find(value) else {
            continue;
        };
        let rune_at = segment[..byte_at].chars().count();
        let absolute_end = start + rune_at + value.chars().count();
        if absolute_end >= chars.len() {
            return value.to_string();
        }
        if matches!(chars[absolute_end], '%' | '€' | '$' | '£' | '¥') {
            return format!("{value}{}", chars[absolute_end]);
        }
        return value.to_string();
    }
    value.to_string()
}

pub(super) fn numeric_entity_grounded(
    language: &str,
    source_language: &str,
    value: &str,
    _canonical_name: &str,
    source_values: &[String],
) -> bool {
    if language.eq_ignore_ascii_case(source_language) {
        return true;
    }
    let allowed: BTreeSet<i64> = source_values
        .iter()
        .flat_map(|value| numeric_values(value))
        .collect();
    let values = numeric_values(value);
    !values.is_empty() && values.iter().all(|number| allowed.contains(number))
}

pub(super) fn numeric_values(value: &str) -> Vec<i64> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if ch.is_alphabetic() || ch.is_numeric() {
            current.push(ch.to_lowercase().next().unwrap_or(ch));
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    let mut values = Vec::new();
    let mut running = 0i64;
    let flush = |values: &mut Vec<i64>, running: &mut i64| {
        if *running != 0 {
            values.push(*running);
            *running = 0;
        }
    };
    for token in tokens {
        if let Ok(number) = token.parse::<i64>() {
            flush(&mut values, &mut running);
            values.push(number);
            continue;
        }
        if matches!(token.as_str(), "and" | "e" | "y" | "de" | "del") {
            continue;
        }
        let Some(number) = numeric_word(&token) else {
            flush(&mut values, &mut running);
            continue;
        };
        if number >= 20 && number % 10 == 0 {
            running += number;
        } else if number == 100 {
            if running == 0 {
                running = 100;
            } else {
                running *= 100;
            }
        } else {
            running += number;
        }
    }
    flush(&mut values, &mut running);
    values
}

pub(super) fn numeric_word(token: &str) -> Option<i64> {
    Some(match token {
        "zero" | "cero" => 0,
        "uno" | "una" | "one" | "um" | "uma" | "un" => 1,
        "two" | "dos" | "due" | "dois" | "duas" => 2,
        "three" | "tres" | "tre" | "três" => 3,
        "four" | "cuatro" | "quattro" | "quatro" => 4,
        "five" | "cinco" | "cinque" => 5,
        "six" | "seis" | "sei" => 6,
        "seven" | "siete" | "sette" => 7,
        "eight" | "ocho" | "otto" | "oito" => 8,
        "nine" | "nueve" | "nove" => 9,
        "ten" | "diez" | "dieci" | "dez" => 10,
        "eleven" | "once" | "undici" | "onze" => 11,
        "twelve" | "doce" | "dodici" | "doze" => 12,
        "thirteen" | "trece" | "tredici" | "treze" => 13,
        "fourteen" | "catorce" | "quattordici" | "quatorze" => 14,
        "fifteen" | "quince" | "quindici" | "quinze" => 15,
        "sixteen" | "dieciséis" | "sedici" | "dezesseis" => 16,
        "seventeen" | "diciassette" => 17,
        "eighteen" | "diciotto" => 18,
        "nineteen" | "diciannove" => 19,
        "twenty" | "veinte" | "venti" | "vinte" => 20,
        "thirty" | "treinta" | "trenta" | "trinta" => 30,
        "forty" | "cuarenta" | "quaranta" => 40,
        "fifty" | "cincuenta" | "cinquanta" => 50,
        "sixty" | "sesenta" | "sessanta" => 60,
        "seventy" | "setenta" | "settanta" => 70,
        "eighty" | "ochenta" | "ottanta" => 80,
        "ninety" | "noventa" | "novanta" => 90,
        "hundred" | "cien" | "ciento" | "cento" | "cem" => 100,
        _ => return None,
    })
}

pub(super) fn date_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\b(?:[0-9]{1,2}\s+(?:gennaio|febbraio|marzo|aprile|maggio|giugno|luglio|agosto|settembre|ottobre|novembre|dicembre)\s+[0-9]{4}|[0-9]{4})\b")
            .expect("date regex")
    })
}

pub(super) fn date_identity_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)^([0-9]{1,2})\s+(gennaio|febbraio|marzo|aprile|maggio|giugno|luglio|agosto|settembre|ottobre|novembre|dicembre)(?:\s+([0-9]{4}))?$")
            .expect("date identity regex")
    })
}

pub(super) fn explicit_date_surfaces(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for matched in date_pattern().find_iter(text) {
        let value = matched.as_str().trim();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.to_lowercase()) {
            out.push(value.to_string());
        }
    }
    out
}

pub(super) fn same_grounded_date(left: &str, right: &str) -> bool {
    let (Some(left), Some(right)) = (
        date_identity_pattern().captures(left.trim()),
        date_identity_pattern().captures(right.trim()),
    ) else {
        return false;
    };
    if left.get(1).map(|value| value.as_str()) != right.get(1).map(|value| value.as_str())
        || !left
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or("")
            .eq_ignore_ascii_case(right.get(2).map(|value| value.as_str()).unwrap_or(""))
    {
        return false;
    }
    let left_year = left.get(3).map(|value| value.as_str()).unwrap_or("");
    let right_year = right.get(3).map(|value| value.as_str()).unwrap_or("");
    left_year.is_empty() || right_year.is_empty() || left_year == right_year
}

pub(super) fn date_has_year(text: &str) -> bool {
    let fields: Vec<&str> = text.split_whitespace().collect();
    fields.len() == 3 && fields[2].len() == 4
}

pub(super) fn score_for_date(score: f64) -> f64 {
    if score <= 0.0 {
        0.8
    } else {
        score
    }
}

/// The producer can surface one grounded number twice: once as an entity
/// (`18 unità`) and once from the numeric vocabulary (`18`). Both become the
/// same card at the same instant. Collapse only same-value, same-start
/// duplicates so a repeated value later in the scene still gets its card.
pub(super) fn dedupe_grounded_metric_numbers(
    numbers: Vec<TimedAnnotationV2>,
) -> Vec<TimedAnnotationV2> {
    let mut seen = BTreeSet::new();
    numbers
        .into_iter()
        .filter(|number| {
            let family = number.annotation_type.trim().to_ascii_uppercase();
            if family == "DATE" || family == "TIME" {
                return true;
            }
            let start_us = if number.start_us > 0 {
                number.start_us
            } else {
                number.start_ms.saturating_mul(1000)
            };
            seen.insert((family, metric_surface_key(&number.text), start_us))
        })
        .collect()
}

pub(super) fn metric_surface_key(text: &str) -> String {
    let trimmed = text.trim();
    let numeric: String = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '.' | ',' | '-' | '+'))
        .collect();
    if numeric.is_empty() {
        trimmed.to_lowercase()
    } else {
        numeric
    }
}

pub(super) fn dedupe_grounded_dates(numbers: Vec<TimedAnnotationV2>) -> Vec<TimedAnnotationV2> {
    let mut dropped = vec![false; numbers.len()];
    for left_index in 0..numbers.len() {
        if !numbers[left_index]
            .annotation_type
            .eq_ignore_ascii_case("DATE")
        {
            continue;
        }
        let Some(left) = date_tokens(&numbers[left_index].text) else {
            continue;
        };
        let left_start = if numbers[left_index].start_us > 0 {
            numbers[left_index].start_us
        } else {
            numbers[left_index].start_ms * 1000
        };
        let left_end =
            if numbers[left_index].start_us + numbers[left_index].duration_us > left_start {
                numbers[left_index].start_us + numbers[left_index].duration_us
            } else {
                numbers[left_index].end_ms * 1000
            };
        for right_index in 0..numbers.len() {
            if left_index == right_index
                || dropped[right_index]
                || !numbers[right_index]
                    .annotation_type
                    .eq_ignore_ascii_case("DATE")
            {
                continue;
            }
            let Some(right) = date_tokens(&numbers[right_index].text) else {
                continue;
            };
            if !(subset(&left, &right) || subset(&right, &left)) {
                continue;
            }
            let right_start = if numbers[right_index].start_us > 0 {
                numbers[right_index].start_us
            } else {
                numbers[right_index].start_ms * 1000
            };
            let right_end =
                if numbers[right_index].start_us + numbers[right_index].duration_us > right_start {
                    numbers[right_index].start_us + numbers[right_index].duration_us
                } else {
                    numbers[right_index].end_ms * 1000
                };
            let overlap = left_end.min(right_end) - left_start.max(right_start);
            let shorter = (left_end - left_start).min(right_end - right_start);
            if overlap <= 0 || shorter <= 0 || overlap * 2 < shorter {
                continue;
            }
            if right.len() > left.len()
                || (right.len() == left.len()
                    && numbers[right_index].text.len() > numbers[left_index].text.len())
            {
                dropped[left_index] = true;
                break;
            }
            dropped[right_index] = true;
        }
    }
    numbers
        .into_iter()
        .enumerate()
        .filter_map(|(index, number)| (!dropped[index]).then_some(number))
        .collect()
}

pub(super) fn date_tokens(text: &str) -> Option<BTreeSet<String>> {
    let months = [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ];
    let tokens: Vec<String> = text
        .split_whitespace()
        .map(|token| token.to_lowercase())
        .collect();
    if tokens.is_empty() {
        return None;
    }
    let mut out = BTreeSet::new();
    for token in tokens {
        if months.contains(&token.as_str()) {
            out.insert(token);
            continue;
        }
        if token.len() != 4 && token.len() > 2 {
            return None;
        }
        if !token.chars().all(|ch| ch.is_numeric()) {
            return None;
        }
        out.insert(token);
    }
    Some(out)
}

pub(super) fn subset(left: &BTreeSet<String>, right: &BTreeSet<String>) -> bool {
    !left.is_empty() && left.len() <= right.len() && left.iter().all(|token| right.contains(token))
}
