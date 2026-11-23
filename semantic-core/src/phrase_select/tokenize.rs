//! Token and sentence boundary primitives for the phrase selector.

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub(super) lower: String,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) group: usize,
}

#[derive(Clone, Debug)]
pub(super) struct SentenceSpan {
    pub(super) byte_start: usize,
    pub(super) byte_end: usize,
    pub(super) token_start: usize,
    pub(super) token_end: usize,
}

pub(super) fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut group = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let (start, c) = chars[i];
        if !is_word_char(c) {
            i += 1;
            continue;
        }
        let mut end = start + c.len_utf8();
        i += 1;
        loop {
            if i >= chars.len() {
                break;
            }
            let (pos, ch) = chars[i];
            if is_word_char(ch) {
                end = pos + ch.len_utf8();
                i += 1;
                continue;
            }
            if (ch == '\'' || ch == '’') && i + 1 < chars.len() && is_word_char(chars[i + 1].1) {
                end = pos + ch.len_utf8();
                i += 1;
                continue;
            }
            break;
        }
        if let Some(previous) = tokens.last() {
            if phrase_boundary_between(&text[previous.end..start]) {
                group += 1;
            }
        }
        tokens.push(Token {
            lower: text[start..end].to_lowercase(),
            start,
            end,
            group,
        });
    }
    tokens
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || is_combining_mark(c)
}

fn is_combining_mark(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

fn phrase_boundary_between(gap: &str) -> bool {
    gap.chars().any(|c| {
        matches!(
            c,
            '.' | '!' | '?' | ';' | ',' | ':' | '—' | '–' | '\n' | '\r'
        )
    })
}

fn sentence_boundary_between(gap: &str) -> bool {
    gap.chars()
        .any(|c| matches!(c, '.' | '!' | '?' | '\n' | '\r'))
}

pub(super) fn sentence_token_spans(text: &str, tokens: &[Token]) -> Vec<SentenceSpan> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut sentences = Vec::new();
    let mut start = 0usize;
    for next in 1..tokens.len() {
        let gap_start = tokens[next - 1].end;
        let gap = &text[gap_start..tokens[next].start];
        if !sentence_boundary_between(gap) {
            continue;
        }
        sentences.push(SentenceSpan {
            byte_start: tokens[start].start,
            byte_end: sentence_boundary_end_byte(gap, gap_start),
            token_start: start,
            token_end: next - 1,
        });
        start = next;
    }
    let mut end_byte = text.len();
    while end_byte > tokens[tokens.len() - 1].end {
        let Some(c) = text[..end_byte].chars().next_back() else {
            break;
        };
        if !c.is_whitespace() {
            break;
        }
        end_byte -= c.len_utf8();
    }
    sentences.push(SentenceSpan {
        byte_start: tokens[start].start,
        byte_end: end_byte,
        token_start: start,
        token_end: tokens.len() - 1,
    });
    sentences
}

fn sentence_boundary_end_byte(gap: &str, absolute_gap_start: usize) -> usize {
    for (offset, c) in gap.char_indices() {
        if !matches!(c, '.' | '!' | '?' | '\n' | '\r') {
            continue;
        }
        let mut end = offset + c.len_utf8();
        while end < gap.len() {
            let Some(next) = gap[end..].chars().next() else {
                break;
            };
            if matches!(next, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»') {
                end += next.len_utf8();
                continue;
            }
            break;
        }
        return absolute_gap_start + end;
    }
    absolute_gap_start + gap.len()
}

pub(super) fn is_terminated_sentence_text(value: &str) -> bool {
    let mut value = value.trim();
    loop {
        let Some(c) = value.chars().next_back() else {
            return false;
        };
        if matches!(c, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»') {
            value = value[..value.len() - c.len_utf8()].trim();
            continue;
        }
        return matches!(c, '.' | '!' | '?');
    }
}
