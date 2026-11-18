//! Canonical token primitives shared by every matcher in the workspace.
//!
//! Phrase alignment (`phrase_align`), the editorial planner (`overlay-compiler`)
//! and the chapter-bullet projection all answer the same question: *is this
//! source phrase the same sequence of words as that timing/annotation stream?*
//! When each call site carries its own folder/trim/numeric rules, a phrase that
//! matches on one path silently fails on another — for example `«Garlasco»`
//! normalizes to `garlasco` here but kept its guillemets in the planner, so the
//! same source produced a located phrase and a dropped one.
//!
//! This module is the single implementation of:
//!
//! - NFC normalization, lowercasing and apostrophe folding;
//! - the token edge-punctuation policy;
//! - whitespace/dash phrase splitting;
//! - numeric-token detection.
//!
//! Call sites keep their own *matching strategy* (document order, overlap,
//! elided-article tolerance); they must not keep their own normalization.

use unicode_normalization::UnicodeNormalization;

/// Folds the apostrophe variants produced by typographic sources onto ASCII
/// `'` so `l’uomo` and `l'uomo` are the same token.
pub fn fold_apostrophe(character: char) -> char {
    match character {
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{02BC}' => '\'',
        other => other,
    }
}

/// One normalized token, or `None` when the fragment carries no content.
///
/// The edge policy is deliberately apostrophe-aware: every punctuation mark is
/// trimmed except `'`, which is kept inside the token and stripped only at the
/// trailing edge (an elision such as `l'` is not a word). Leading apostrophes
/// survive because they are part of the word (`'ndrangheta`).
pub fn normalize_token(fragment: &str) -> Option<String> {
    let nfc: String = fragment.nfc().collect();
    let mut folded = String::with_capacity(nfc.len());
    for character in nfc.chars().flat_map(char::to_lowercase) {
        folded.push(fold_apostrophe(character));
    }
    let trimmed = folded.trim_matches(is_edge_punctuation);
    let trimmed = trimmed.trim_end_matches('\'');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Splits one whitespace-free field on dash separators and normalizes each
/// fragment. Dash-joined script words (`Nord-Est`) align with separate TTS
/// boundaries, so both sides go through this function.
pub fn fragment_tokens(field: &str) -> Vec<String> {
    field
        .split(|character: char| character.is_whitespace() || is_dash_separator(character))
        .filter_map(normalize_token)
        .collect()
}

/// Normalized tokens of a phrase: whitespace fields, each split on dashes.
pub fn phrase_tokens(phrase: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for field in phrase.split_whitespace() {
        tokens.extend(fragment_tokens(field));
    }
    tokens
}

/// True when the token is a plain number: digits with at most one `.`/`,`
/// separator. Used to tolerate an elided article in front of a number
/// (`all'18` timing vs `18` phrase).
pub fn is_numeric_token(token: &str) -> bool {
    let mut seen_separator = false;
    let mut seen_digit = false;
    for character in token.chars() {
        match character {
            '0'..='9' => seen_digit = true,
            '.' | ',' if !seen_separator => seen_separator = true,
            _ => return false,
        }
    }
    seen_digit
}

/// Dash characters that separate words rather than belong to them.
pub fn is_dash_separator(character: char) -> bool {
    matches!(
        character,
        '-' | '\u{2010}'
            | '\u{2011}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{2015}'
            | '\u{2212}'
    )
}

/// Punctuation allowed at a token edge. `'` is excluded because
/// [`normalize_token`] handles it separately.
///
/// Rust's std does not expose a stable Unicode general-category predicate, so
/// this covers the ASCII/Latin/GPP/CJK/currency ranges the pipeline receives.
/// One predicate for every call site is the point: an ASCII-only variant
/// silently failed to trim `«»`, `“”`, `…` and the CJK marks.
pub fn is_edge_punctuation(character: char) -> bool {
    if character == '\'' {
        return false;
    }
    character.is_ascii_punctuation()
        || matches!(
            character,
            '\u{00A1}'
                | '\u{00AB}'
                | '\u{00B7}'
                | '\u{00BB}'
                | '\u{00BF}'
                | '\u{2010}'..='\u{2027}'
                | '\u{2030}'..='\u{205E}'
                | '\u{3001}'..='\u{303F}'
                | '\u{FF01}'..='\u{FF0F}'
                | '\u{FF1A}'..='\u{FF20}'
                | '\u{FF3B}'..='\u{FF40}'
                | '\u{FF5B}'..='\u{FF65}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_case_and_typographic_apostrophes_fold_to_one_token() {
        // U+2019 right single quote, NFD input for "città".
        assert_eq!(
            normalize_token("L\u{2019}UOMO"),
            Some("l'uomo".to_string()),
            "typographic apostrophe folds and case folds"
        );
        assert_eq!(
            normalize_token("citta\u{0300}"),
            Some("città".to_string()),
            "NFD input normalizes to the same NFC token"
        );
    }

    #[test]
    fn edge_punctuation_is_trimmed_consistently_across_script_families() {
        assert_eq!(normalize_token("«Garlasco»").as_deref(), Some("garlasco"));
        assert_eq!(
            normalize_token("\u{201C}Garlasco\u{201D}").as_deref(),
            Some("garlasco")
        );
        assert_eq!(normalize_token("—Garlasco…").as_deref(), Some("garlasco"));
        assert_eq!(
            normalize_token("『ガルラスコ』").as_deref(),
            Some("ガルラスコ")
        );
    }

    #[test]
    fn currency_and_math_edges_trim_but_internal_apostrophe_survives() {
        assert_eq!(normalize_token("$5").as_deref(), Some("5"));
        assert_eq!(normalize_token("+3").as_deref(), Some("3"));
        assert_eq!(normalize_token("l'uomo").as_deref(), Some("l'uomo"));
        // A leading apostrophe belongs to the word; a trailing one is elision.
        assert_eq!(
            normalize_token("'ndrangheta").as_deref(),
            Some("'ndrangheta")
        );
        assert_eq!(normalize_token("l'").as_deref(), Some("l"));
    }

    #[test]
    fn punctuation_only_fragments_have_no_token() {
        assert_eq!(normalize_token("«»"), None);
        assert_eq!(normalize_token("—"), None);
        assert_eq!(normalize_token(""), None);
    }

    #[test]
    fn dash_joined_words_split_on_the_same_separators_as_timing_words() {
        assert_eq!(
            phrase_tokens("Nord-Est e Sud\u{2014}Ovest"),
            vec![
                "nord".to_string(),
                "est".to_string(),
                "e".to_string(),
                "sud".to_string(),
                "ovest".to_string()
            ]
        );
        // A non-breaking hyphen is a separator, not word content.
        assert_eq!(phrase_tokens("d\u{2011}accordo"), vec!["d", "accordo"]);
    }

    #[test]
    fn numeric_tokens_accept_one_separator_and_require_a_digit() {
        for accepted in ["18", "3.5", "3,5", "1.234"] {
            assert!(is_numeric_token(accepted), "{accepted} is numeric");
        }
        for rejected in ["", "tre", "3.5.1", "3,5.1", "-3", "3%"] {
            assert!(!is_numeric_token(rejected), "{rejected:?} is not numeric");
        }
    }
}
