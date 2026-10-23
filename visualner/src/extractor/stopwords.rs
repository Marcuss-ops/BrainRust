//! Candidate-exclusion vocabularies: stopwords, stop phrases, title roles
//! and narration boilerplate. Word data lives in the lexicon, this module owns
//! only the matching semantics.
//!
//! The filter is data only: a generated language pack plus the curated title
//! nouns. There is no cross-language fallback table — a language without a
//! pack simply has no word-class filter (tier C), and adding one is a data
//! directory, never a Rust change.

use semantic_core::lexicon::lexicon;

pub(super) fn is_stop_word(word: &str, language: &str) -> bool {
    let lower = word.to_lowercase();
    if semantic_core::language_packs::get(language).is_some_and(|pack| pack.is_stopword(&lower)) {
        return true;
    }
    // Titles name an office, not an entity, in every language that lists them.
    is_title_word(&lower)
}

/// Role and title nouns. UDPipe tags them as ordinary nouns, but they name
/// an office, not a visible object, so they stay excluded even when a POS
/// tag is available.
pub(super) fn is_title_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    lexicon().stopwords.title_words.iter().any(|s| s == &lower)
}

/// Stop phrases: multi-word surfaces that are generic even when none of
/// their tokens are individually stopwords.
pub(super) fn is_stop_phrase(normalized: &str) -> bool {
    lexicon()
        .stopwords
        .stop_phrases
        .iter()
        .any(|s| s == normalized)
}

/// Narration boilerplate ("Imagine the", "Get ready", ...) that must never
/// become an entity. The seeds come from observed voiceover scripts and are
/// curated in the lexicon, not in code.
pub(super) fn is_generic_phrase(normalized: &str) -> bool {
    lexicon()
        .stopwords
        .generic_phrases
        .iter()
        .any(|s| s == normalized)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_and_italian_stop_words_are_excluded() {
        assert!(is_stop_word("the", "en"));
        assert!(is_stop_word("The", "en"));
        assert!(is_stop_word("della", "it"));
        assert!(is_stop_word("dopo", "it"));
        assert!(!is_stop_word("tifosi", "it"));
        assert!(!is_stop_word("feta", "en"));
    }

    #[test]
    fn accented_stop_words_match_in_their_own_language() {
        assert!(is_stop_word("è", "it"));
        assert!(is_stop_word("président", "fr"));
        assert!(is_stop_word("präsident", "de"));
    }

    #[test]
    fn boilerplate_phrases_never_become_entities() {
        assert!(is_stop_phrase("imagine the"));
        assert!(is_generic_phrase("get ready"));
        assert!(!is_generic_phrase("stadio"));
    }

    #[test]
    fn title_roles_are_excluded() {
        assert!(is_title_word("presidente"));
        assert!(!is_title_word("stadio"));
    }
}
