//! Canonical sentence segmentation (single source of truth).
//!
//! Every extractor that needs sentence boundaries calls this module: VisualNER
//! (bullets, relations, chart grouping), Phrase Impact (extractive summary,
//! bullets, chapters) and Scene Intelligence (temporal/causal/comparison).
//! A second segmenter is what let the same script report different sentence
//! counts on different paths, so there is exactly ONE implementation here and
//! the callers only project its spans.
//!
//! The splitter is deterministic and language-aware:
//!
//! - `.` between two digits is a decimal point (`3.5`), not a boundary;
//! - dotted initialisms (`U.S.A.`), clock suffixes (`10:30 a.m.`), inner
//!   abbreviations (`e.g.`) and localized titles (`dott.`, `Mme.`, `z.b.`)
//!   do not end a sentence;
//! - CJK terminators (`。！？`) split without a following space;
//! - trailing closing quotes/brackets stay attached to the sentence;
//! - an optional `max_words` bound splits over-long sentences at word
//!   boundaries without changing the surviving text.
//!
//! Spans are trimmed and byte-based (UTF-8): `text[start_byte..end_byte]` is
//! always the exact sentence surface.

/// One trimmed sentence span inside the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentenceSpan {
    /// Inclusive UTF-8 byte offset where the sentence begins.
    pub start_byte: usize,
    /// Exclusive UTF-8 byte offset where the sentence ends.
    pub end_byte: usize,
}

impl SentenceSpan {
    /// The verbatim sentence surface.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.start_byte..self.end_byte).unwrap_or("")
    }
}

/// Splits `text` into trimmed sentence spans in source order.
///
/// `max_words` bounds each span (Phrase Impact's embedding chunks); `None`
/// keeps whole sentences (VisualNER, Scene Intelligence).
pub fn split_sentences(text: &str, language: &str, max_words: Option<usize>) -> Vec<SentenceSpan> {
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
        // "1. La fondazione..." is a list marker, not a sentence boundary:
        // splitting the rank into its own sentence would lose the item text.
        if ch == '.' && is_list_marker(&chars, i) {
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
        if word_count(segment.text(text)) <= max_words {
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

fn push_segment(text: &str, start: usize, end: usize, output: &mut Vec<SentenceSpan>) {
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
    output.push(SentenceSpan {
        start_byte,
        end_byte: start_byte + value.len(),
    });
}

fn split_long_segment(
    text: &str,
    start: usize,
    end: usize,
    max_words: usize,
    output: &mut Vec<SentenceSpan>,
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

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// A period that closes a one- or two-digit ordinal at the start of the
/// text or after whitespace, followed by whitespace and the item text.
fn is_list_marker(chars: &[(usize, char)], i: usize) -> bool {
    if chars[i].1 != '.' || !chars.get(i + 1).is_some_and(|(_, c)| c.is_whitespace()) {
        return false;
    }
    let mut cursor = i;
    while cursor > 0 && chars[cursor - 1].1.is_ascii_digit() {
        cursor -= 1;
    }
    let digits = i - cursor;
    if digits == 0 || digits > 2 {
        return false;
    }
    // The marker must start a line or follow a sentence terminator. This
    // keeps a sentence-final cardinal ("Ha segnato 2. Poi uscì.") split, and
    // the whitespace requirement keeps a decimal tail ("3.14.") out.
    if cursor > 0 && !chars[cursor - 1].1.is_whitespace() {
        return false;
    }
    let mut whitespace_start = cursor;
    while whitespace_start > 0 && chars[whitespace_start - 1].1.is_whitespace() {
        whitespace_start -= 1;
    }
    if whitespace_start == 0 {
        return true;
    }
    if chars[whitespace_start..cursor]
        .iter()
        .any(|(_, value)| *value == '\n')
    {
        return true;
    }
    matches!(chars[whitespace_start - 1].1, '.' | '!' | '?' | ':')
}

fn is_decimal_point(chars: &[(usize, char)], i: usize) -> bool {
    i > 0
        && i + 1 < chars.len()
        && chars[i - 1].1.is_ascii_digit()
        && chars[i + 1].1.is_ascii_digit()
}

fn punctuation_run_end(chars: &[(usize, char)], i: usize) -> usize {
    let mut end = i;
    while end + 1 < chars.len() && matches!(chars[end + 1].1, '.' | '!' | '?' | '。' | '！' | '？')
    {
        end += 1;
    }
    end
}

fn is_dotted_initialism(chars: &[(usize, char)], period: usize, following: usize) -> bool {
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

fn is_clock_suffix(chars: &[(usize, char)], period: usize) -> bool {
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

fn is_inner_dotted_abbreviation(chars: &[(usize, char)], period: usize) -> bool {
    period >= 1
        && period + 2 < chars.len()
        && chars[period - 1].1.is_ascii_alphabetic()
        && chars[period + 1].1.is_ascii_alphabetic()
        && chars[period + 2].1 == '.'
}

fn suppress_abbreviation(
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
    // Abbreviation sets live in `lexicon/orthography.json`: the common forms
    // apply to every language, the localized ones are per-language data.
    let abbreviations = &crate::lexicon::lexicon().orthography.abbreviations;
    let next = chars.get(following).map(|(_, c)| *c);
    if abbreviations
        .corporate
        .iter()
        .any(|corporate| corporate == &token)
    {
        return next.is_some_and(|c| c.is_lowercase());
    }
    let abbreviation = abbreviations
        .for_language(language)
        .any(|candidate| candidate == token.as_str());
    abbreviation && next.is_some_and(|c| c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts<'a>(source: &'a str, language: &str) -> Vec<&'a str> {
        split_sentences(source, language, None)
            .into_iter()
            .map(|span| span.text(source))
            .collect()
    }

    #[test]
    fn splits_on_terminators_and_keeps_the_surface() {
        let source = "Prima frase. Seconda frase! Terza?";
        assert_eq!(
            texts(source, "it"),
            vec!["Prima frase.", "Seconda frase!", "Terza?"]
        );
    }

    #[test]
    fn decimal_points_do_not_split() {
        assert_eq!(
            texts("Il valore è 3.5 punti.", "it"),
            vec!["Il valore è 3.5 punti."]
        );
    }

    #[test]
    fn abbreviations_and_clock_suffixes_do_not_split() {
        let source = "Il dott. Rossi arriva alle 10:30 a.m. con il team. Poi parte.";
        assert_eq!(
            texts(source, "it"),
            vec![
                "Il dott. Rossi arriva alle 10:30 a.m. con il team.",
                "Poi parte."
            ]
        );
        assert_eq!(
            texts("L'U.S.A. hanno firmato. Fine.", "it"),
            vec!["L'U.S.A. hanno firmato.", "Fine."]
        );
    }

    #[test]
    fn cjk_terminators_split_without_space() {
        assert_eq!(
            texts("第一句。第二句！", "ja"),
            vec!["第一句。", "第二句！"]
        );
    }

    #[test]
    fn max_words_bounds_without_changing_the_text() {
        let source = "uno due tre quattro cinque sei";
        let spans = split_sentences(source, "it", Some(2));
        assert_eq!(spans.len(), 3);
        for span in &spans {
            assert_eq!(
                source.get(span.start_byte..span.end_byte),
                Some(span.text(source))
            );
        }
        let joined: Vec<&str> = spans.iter().map(|span| span.text(source)).collect();
        assert_eq!(joined, vec!["uno due", "tre quattro", "cinque sei"]);
    }

    #[test]
    fn numbered_list_markers_stay_with_their_item_text() {
        assert_eq!(
            texts("1. La fondazione. 2. L'inaugurazione.", "it"),
            vec!["1. La fondazione.", "2. L'inaugurazione."]
        );
        // A four-digit year is not a list marker, so it still ends a sentence.
        assert_eq!(
            texts("Nel 1959. Poi arrivò il resto.", "it"),
            vec!["Nel 1959.", "Poi arrivò il resto."]
        );
    }

    #[test]
    fn empty_and_whitespace_only_input_yield_no_spans() {
        assert!(split_sentences("", "it", None).is_empty());
        assert!(split_sentences("   \n ", "it", None).is_empty());
    }
}
