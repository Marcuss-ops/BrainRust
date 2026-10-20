//! Internal split of phrase_impact (pure move, no behavior change).

use super::*;

pub fn split_sentences(text: &str, language: &str) -> Vec<Segment> {
    split_sentence_spans(text, language, Some(MAX_SEGMENT_WORDS))
}

/// Canonical sentence boundary detector shared with Scene Intelligence. The
/// optional word limit preserves phrase-impact's bounded embedding chunks
/// while letting SI retain its original whole-sentence granularity.
pub(crate) fn split_sentence_spans(
    text: &str,
    language: &str,
    max_words: Option<usize>,
) -> Vec<Segment> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut cuts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i].1;
        if !matches!(ch, '.' | '!' | '?' | '。' | '！' | '？') {
            i += 1;
            continue;
        }
        if ch == '.' && is_decimal_point(&chars, i) {
            i += 1;
            continue;
        }
        let punctuation_end = punctuation_run_end(&chars, i);
        let next_index = punctuation_end + 1;
        let next = chars.get(next_index).map(|(_, c)| *c);
        let following_index = (next_index..chars.len())
            .find(|&index| !chars[index].1.is_whitespace())
            .unwrap_or(chars.len());

        // Split before a plausible capitalized sentence even when punctuation
        // is missing its following space, but not within initialisms.
        if ch == '.'
            && next.is_some_and(|value| !value.is_whitespace() && value.is_ascii_uppercase())
            && following_index == next_index
            && !(i > 0 && chars[i - 1].1.is_ascii_uppercase())
        {
            cuts.push(chars[punctuation_end].0 + chars[punctuation_end].1.len_utf8());
            i = punctuation_end + 1;
            continue;
        }
        if matches!(ch, '。' | '！' | '？')
            && next.is_some_and(|value| !value.is_whitespace())
            && chars
                .get(following_index)
                .is_some_and(|(_, value)| value.is_uppercase())
        {
            cuts.push(chars[punctuation_end].0 + chars[punctuation_end].1.len_utf8());
            i = punctuation_end + 1;
            continue;
        }
        if next.is_some_and(|value| {
            !value.is_whitespace()
                && !matches!(value, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»')
        }) && !matches!(ch, '。' | '！' | '？')
        {
            i = punctuation_end + 1;
            continue;
        }
        if matches!(ch, '.' | '。') && is_dotted_initialism(&chars, i, following_index) {
            i += 1;
            continue;
        }
        if ch == '.' && is_inner_dotted_abbreviation(&chars, i) {
            i += 1;
            continue;
        }
        if ch == '.' && suppress_abbreviation(&chars, i, following_index, language) {
            i += 1;
            continue;
        }
        cuts.push(chars[punctuation_end].0 + chars[punctuation_end].1.len_utf8());
        i = punctuation_end + 1;
        while i < chars.len()
            && matches!(chars[i].1, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»')
        {
            if let Some(last) = cuts.last_mut() {
                *last = chars[i].0 + chars[i].1.len_utf8();
            }
            i += 1;
        }
    }

    let mut segments = Vec::new();
    let mut start = 0;
    for end in cuts {
        push_segment(text, start, end, &mut segments);
        start = end;
    }
    push_segment(text, start, text.len(), &mut segments);

    let Some(max_words) = max_words else {
        return segments;
    };
    let mut bounded = Vec::new();
    for segment in segments {
        if word_count(&segment.text) <= max_words {
            bounded.push(segment);
        } else {
            split_long_segment(
                text,
                segment.start_byte,
                segment.end_byte,
                max_words,
                &mut bounded,
            );
        }
    }
    bounded
}

pub(super) fn push_segment(text: &str, start: usize, end: usize, output: &mut Vec<Segment>) {
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return;
    }
    let slice = &text[start..end];
    let leading = slice.len() - slice.trim_start().len();
    let value = slice.trim();
    if value.is_empty() {
        return;
    }
    let start_byte = start + leading;
    output.push(Segment {
        start_byte,
        end_byte: start_byte + value.len(),
        text: value.to_string(),
    });
}

pub(super) fn split_long_segment(
    text: &str,
    start: usize,
    end: usize,
    max_words: usize,
    output: &mut Vec<Segment>,
) {
    let source = &text[start..end];
    let mut ranges = Vec::new();
    let mut word_start = None;
    for (offset, ch) in source.char_indices() {
        if ch.is_whitespace() {
            if let Some(begin) = word_start.take() {
                ranges.push((begin, offset));
            }
        } else if word_start.is_none() {
            word_start = Some(offset);
        }
    }
    if let Some(begin) = word_start {
        ranges.push((begin, source.len()));
    }
    let mut first = 0;
    while first < ranges.len() {
        let end_word = (first + max_words).min(ranges.len());
        push_segment(
            text,
            start + ranges[first].0,
            start + ranges[end_word - 1].1,
            output,
        );
        first = end_word;
    }
}

pub(super) fn is_decimal_point(chars: &[(usize, char)], i: usize) -> bool {
    i > 0
        && i + 1 < chars.len()
        && chars[i - 1].1.is_ascii_digit()
        && chars[i + 1].1.is_ascii_digit()
}

pub(super) fn punctuation_run_end(chars: &[(usize, char)], i: usize) -> usize {
    let mut end = i;
    while end + 1 < chars.len() && matches!(chars[end + 1].1, '.' | '!' | '?' | '。' | '！' | '？')
    {
        end += 1;
    }
    end
}
pub(super) fn is_dotted_initialism(
    chars: &[(usize, char)],
    period: usize,
    following: usize,
) -> bool {
    if chars[period].1 != '.' {
        return false;
    }
    if is_clock_suffix(chars, period) {
        return true;
    }
    if period + 1 < chars.len()
        && chars[period + 1].1.is_ascii_uppercase()
        && (period == 0 || !chars[period - 1].1.is_ascii_uppercase())
    {
        return true;
    }
    if period >= 2
        && chars[period - 1].1.is_ascii_uppercase()
        && chars[period - 2].1.is_ascii_uppercase()
        && chars
            .get(following)
            .is_some_and(|(_, value)| value.is_ascii_lowercase())
    {
        let mut cursor = period - 1;
        let mut capitals = 1;
        while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase()
        {
            capitals += 1;
            cursor -= 2;
        }
        if capitals >= 2 {
            return true;
        }
    }
    if period == 0 || !chars[period - 1].1.is_ascii_uppercase() {
        return false;
    }
    if chars
        .get(following)
        .is_some_and(|(_, value)| value.is_ascii_lowercase())
    {
        let mut cursor = period - 1;
        let mut capitals = 1;
        while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase()
        {
            capitals += 1;
            cursor -= 2;
        }
        return capitals >= 2;
    }

    // A spaced single-letter initial followed by another initial or a
    // capitalized token is part of a name, not a sentence boundary.
    match chars.get(following) {
        Some((_, value)) if value.is_ascii_uppercase() => {}
        _ => return false,
    }
    let next_token_end = chars[following..]
        .iter()
        .position(|(_, value)| !value.is_ascii_alphabetic())
        .map_or(chars.len(), |relative| following + relative);
    let next_is_initial = next_token_end == following + 1
        && chars
            .get(next_token_end)
            .is_some_and(|(_, value)| *value == '.');
    if !next_is_initial && next_token_end <= following + 1 {
        return false;
    }
    let starts_initial = period == 1 || !chars[period - 2].1.is_ascii_uppercase();
    if starts_initial {
        return true;
    }

    let mut cursor = period - 1;
    let mut capitals = 1;
    while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase() {
        capitals += 1;
        cursor -= 2;
    }
    capitals >= 2
}

pub(super) fn is_clock_suffix(chars: &[(usize, char)], period: usize) -> bool {
    if period < 4
        || chars[period - 2].1 != '.'
        || !chars[period - 3].1.is_ascii_alphabetic()
        || !chars[period - 1].1.is_ascii_alphabetic()
    {
        return false;
    }
    let mut cursor = period - 4;
    while cursor > 0 && chars[cursor].1.is_whitespace() {
        cursor -= 1;
    }
    let mut saw_colon = false;
    let mut saw_digit = false;
    loop {
        match chars[cursor].1 {
            ':' => saw_colon = true,
            value if value.is_ascii_digit() => saw_digit = true,
            value if value.is_whitespace() => break,
            _ => return false,
        }
        if cursor == 0 {
            break;
        }
        cursor -= 1;
    }
    saw_colon && saw_digit
}

pub(super) fn is_inner_dotted_abbreviation(chars: &[(usize, char)], period: usize) -> bool {
    period >= 1
        && period + 2 < chars.len()
        && chars[period - 1].1.is_ascii_alphabetic()
        && chars[period + 1].1.is_ascii_alphabetic()
        && chars[period + 2].1 == '.'
}

pub(super) fn suppress_abbreviation(
    chars: &[(usize, char)],
    period: usize,
    following: usize,
    language: &str,
) -> bool {
    let mut start = period;
    while start > 0 && (chars[start - 1].1.is_ascii_alphabetic() || chars[start - 1].1 == '.') {
        start -= 1;
    }
    let token: String = chars[start..=period].iter().map(|(_, c)| *c).collect();
    let token = token.to_lowercase();
    let lang = language
        .split(['-', '_'])
        .next()
        .unwrap_or("en")
        .to_lowercase();
    let common = [
        "dr.", "mr.", "mrs.", "ms.", "prof.", "sr.", "sra.", "srta.", "st.", "vs.", "e.g.", "i.e.",
        "no.", "ph.d.",
    ];
    let localized: &[&str] = match lang.as_str() {
        "it" => &["dott.", "sig.", "ecc."],
        "es" => &["dra.", "dpto.", "núm."],
        "fr" => &["m.", "mme.", "mlle.", "etc."],
        "de" => &["z.b.", "bzw.", "usw.", "dr."],
        "pt" => &["dr.", "dra.", "sr.", "sra.", "etc."],
        _ => &[],
    };
    let corporate = ["inc.", "corp.", "ltd.", "co.", "llc.", "gmbh."];
    let next = chars.get(following).map(|(_, c)| *c);
    if corporate.contains(&token.as_str()) {
        return next.is_some_and(|c| c.is_lowercase());
    }
    let abbreviation = common.contains(&token.as_str())
        || localized.contains(&token.as_str())
        || ["etc.", "ecc."].contains(&token.as_str());
    abbreviation && next.is_some_and(|c| c.is_alphanumeric())
}
