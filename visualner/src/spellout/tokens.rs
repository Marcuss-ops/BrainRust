//! UTF-8 token spans used by the spellout scanner.

use semantic_core::phrase_select::is_combining_mark;

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
