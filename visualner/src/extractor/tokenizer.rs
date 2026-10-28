//! ICU word segmentation and Token assembly.

use super::location::starts_uppercase;
use super::pos::tag_tokens;

/// Token is a maximal run of letters (ASCII + accented Latin) and
/// apostrophes/hyphens inside a word, with its byte offsets into the
/// source text. Non-letter chars are separators.
#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub(crate) text: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Universal POS tag from UDPipe, when a model for the language is installed.
    pub(crate) upos: Option<String>,
    /// Dictionary form from the same UDPipe model, when it is installed.
    pub(crate) lemma: Option<String>,
}

/// tokenize segments `source_text` into words with ICU's word-break rules for
/// `language`. ICU handles scripts without spaces (Chinese, Japanese, Thai)
/// and language-specific apostrophes, so no per-language lists are needed.
/// Hyphenated and apostrophe-joined compounds (Jean-Luc) become one token.
/// ICU offsets are UTF-16 units; they are converted to UTF-8 byte offsets.
pub(crate) fn tokenize(source_text: &str, language: &str) -> Result<Vec<Token>, String> {
    let mut tokens = tokenize_untyped(source_text, language)?;
    tag_tokens(source_text, language, &mut tokens);
    Ok(tokens)
}

/// Segments text with the same ICU rules as [`tokenize`] but without the
/// optional UDPipe POS/lemma pass. Callers that only need word boundaries,
/// offsets and capitalisation (the script-wide name context) must use this:
/// the full parse costs seconds on a long scene and cannot change the run
/// of capitalised tokens they read.
pub(crate) fn tokenize_untyped(source_text: &str, language: &str) -> Result<Vec<Token>, String> {
    let breaks = rust_icu_ubrk::UBreakIterator::try_new(
        rust_icu_sys::UBreakIteratorType::UBRK_WORD,
        language,
        source_text,
    )
    .map_err(|error| format!("ICU word segmentation failed for {language:?}: {error}"))?;

    let mut utf16_to_byte: Vec<usize> = Vec::with_capacity(source_text.len() + 1);
    for (offset, ch) in source_text.char_indices() {
        for _ in 0..ch.len_utf16() {
            utf16_to_byte.push(offset);
        }
    }
    utf16_to_byte.push(source_text.len());

    // (byte_start, byte_end, is_word) for every ICU segment, in order.
    let mut segments: Vec<(usize, usize, bool)> = Vec::new();
    let mut start16 = breaks.first();
    loop {
        let end16 = breaks.following(start16);
        if end16 < 0 {
            break;
        }
        // UBRK_WORD_NONE_LIMIT is 100: rule status >= 100 marks word-like text.
        let is_word = breaks.get_rule_status() >= 100;
        segments.push((
            utf16_to_byte[start16 as usize],
            utf16_to_byte[end16 as usize],
            is_word,
        ));
        start16 = end16;
    }

    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < segments.len() {
        let (start, end, is_word) = segments[index];
        index += 1;
        if !is_word {
            continue;
        }
        let mut token_end = end;
        // Join word, joiner, word where the joiner is a hyphen or apostrophe.
        while index + 1 < segments.len() {
            let (_, joiner_end, joiner_word) = segments[index];
            let (next_start, next_end, next_word) = segments[index + 1];
            let joiner = &source_text[token_end..joiner_end];
            let is_joiner = joiner == "-" || joiner == "'" || joiner == "\u{2019}";
            if is_joiner && !joiner_word && next_word && next_start == joiner_end {
                token_end = next_end;
                index += 2;
            } else {
                break;
            }
        }
        tokens.push(Token {
            text: source_text[start..token_end].to_string(),
            start,
            end: token_end,
            upos: None,
            lemma: None,
        });
    }
    Ok(split_elided_clitics(source_text, tokens))
}

/// split_elided_clitics separates an elided preposition/article from the
/// proper name it is attached to: ICU keeps "dell'Abruzzo", "dell'Umbria"
/// and "sull'Aterno" as single word tokens, so the name can never be a
/// candidate and the geographic cue never sees its preposition. Splitting
/// yields the cue token ("dell'") and the place ("Abruzzo"), both still
/// verbatim spans of the source. A one-letter prefix stays attached
/// ("L'Aquila", "d'Artagnan", "O'Brien"), because there the prefix is part
/// of the name; a lower-case remainder keeps the compound ("dell'arte",
/// "l'arte").
fn split_elided_clitics(source_text: &str, tokens: Vec<Token>) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len() + 4);
    for token in tokens {
        match elided_clitic_split(source_text, &token) {
            Some((clitic_end, name_start)) => {
                let mut clitic = token.clone();
                clitic.text = source_text[token.start..clitic_end].to_string();
                clitic.end = clitic_end;
                let mut name = token;
                name.text = source_text[name_start..name.end].to_string();
                name.start = name_start;
                out.push(clitic);
                out.push(name);
            }
            None => out.push(token),
        }
    }
    out
}

/// elided_clitic_split returns the byte end of the elided prefix (the
/// apostrophe included) and the byte start of the proper name, when the
/// token is `<two-or-more ASCII letters>'<Uppercase...>`.
fn elided_clitic_split(source_text: &str, token: &Token) -> Option<(usize, usize)> {
    let text = source_text.get(token.start..token.end)?;
    if text != token.text {
        return None;
    }
    // Only the clitic itself has to be ASCII letters; the name it precedes may
    // carry any script ("dell'Università", "dell'Abruzzo", "d'Italia" is the
    // one-letter case that stays attached). Rejecting the whole token on a
    // non-ASCII remainder left the name fused to its article and invisible to
    // every name rule.
    let apostrophe = text.find(['\'', '\u{2019}'])?;
    let prefix = &text[..apostrophe];
    if prefix.len() < 2 || !prefix.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let remainder_start = token.start + apostrophe + text[apostrophe..].chars().next()?.len_utf8();
    if !starts_uppercase(source_text.get(remainder_start..token.end)?) {
        return None;
    }
    Some((token.start + apostrophe + 1, remainder_start))
}
