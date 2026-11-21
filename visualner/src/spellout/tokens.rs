//! UTF-8 token spans used by the spellout scanner.

#[derive(Debug, Clone)]
pub(super) struct WordToken {
    pub(super) start: usize,
    pub(super) end: usize,
}

pub(super) fn word_tokens(source: &str) -> Vec<WordToken> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut previous_was_word = false;
    for (offset, character) in source.char_indices() {
        let word = character.is_alphanumeric() || is_combining_mark(character);
        let connector = matches!(
            character,
            '-' | '\u{2010}' | '\u{2011}' | '\u{00AD}' | '\'' | '\u{2019}'
        ) && previous_was_word
            && source[offset + character.len_utf8()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if word || connector {
            start.get_or_insert(offset);
            previous_was_word = word;
        } else {
            if let Some(token_start) = start.take() {
                tokens.push(WordToken {
                    start: token_start,
                    end: offset,
                });
            }
            previous_was_word = false;
        }
    }
    if let Some(token_start) = start {
        tokens.push(WordToken {
            start: token_start,
            end: source.len(),
        });
    }
    tokens
}

fn is_combining_mark(character: char) -> bool {
    matches!(character as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF |
        0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}
