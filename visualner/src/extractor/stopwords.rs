//! Candidate-exclusion vocabularies: stopwords, stop phrases, title roles
//! and narration boilerplate. Every word lives in the shared lexicon
//! (`stopwords.json`); this module owns only the matching semantics, so a
//! vocabulary change is a data change and never a code change.

use semantic_core::lexicon::lexicon;

pub(super) fn is_stop_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    let table = &lexicon().stopwords;
    table.stop_words.iter().any(|s| s == &lower)
        || word
            .chars()
            .any(|ch| matches!(ch, 'à' | 'è' | 'é' | 'ì' | 'ò' | 'ù' | 'ü' | 'ö' | 'ä'))
            && table.non_ascii_stop_words.iter().any(|s| s == &lower)
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
        assert!(is_stop_word("the"));
        assert!(is_stop_word("The"));
        assert!(is_stop_word("della"));
        assert!(is_stop_word("dopo"));
        assert!(!is_stop_word("tifosi"));
        assert!(!is_stop_word("feta"));
    }

    #[test]
    fn non_ascii_stop_words_match_their_accented_forms() {
        assert!(is_stop_word("è"));
        assert!(is_stop_word("président"));
        assert!(is_stop_word("präsident"));
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
