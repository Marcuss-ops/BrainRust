//! Model-based verbal-predicate detection (GEN-01).
//!
//! The UDPipe model installed for the language supplies both the universal POS
//! tag and the lemma for every token. The lemma is the authority: an inflected
//! surface whose lemma is the infinitive ("guarda" -> "guardare") is a verb,
//! while a noun that happens to look like a verb keeps its own lemma
//! ("guarda" -> "guarda") and stays a visual concept. Using the lemma removes
//! the per-language suffix tables (`VERB_ENDINGS`, `NOUN_ENDINGS`) and the
//! contextual slot heuristics that tried to guess the same decision from
//! endings.
//!
//! The tag itself is still accepted ("VERB"/"AUX"), because UDPipe is
//! authoritative there; the lemma adds recall when the tagger picks an
//! ambiguous tag (ADJ/X) for an inflected verb.
//!
//! Scope: Italian (`it`), the language whose lexical behavior is certified.
//! Without a model there is no lemma, so the extractor keeps its model-free
//! behavior (no predicate filtering) instead of guessing from suffixes.

use super::Token;

/// is_verbal_predicate reports whether the candidate phrase spanning
/// `tokens[phrase_start..phrase_end]` is a verbal predicate in context and
/// must not become a VISUAL_CONCEPT entity.
pub(super) fn is_verbal_predicate(
    tokens: &[Token],
    phrase_start: usize,
    phrase_end: usize,
    language: &str,
) -> bool {
    if !is_italian(language) || phrase_start >= phrase_end {
        return false;
    }
    // Proper names are never dropped by the predicate rule: "E Milano" is a
    // conjunct name, not the verb reading of the same surface.
    if super::starts_uppercase(&tokens[phrase_start].text) {
        return false;
    }
    // A clitic plus its verb ("si trova") is one predicate; the model supplies
    // the lemma for the verb token, so the whole span is checked.
    tokens[phrase_start..phrase_end].iter().any(is_verb_token)
}

/// is_verbal_continuation reports whether the token at `index` continues a
/// predicate instead of the noun phrase being built ("I tifosi **festeggiano**
/// sugli spalti"): a verb breaks the phrase and is never candidate material.
pub(super) fn is_verbal_continuation(tokens: &[Token], index: usize, language: &str) -> bool {
    if !is_italian(language) {
        return false;
    }
    let Some(token) = tokens.get(index) else {
        return false;
    };
    !super::starts_uppercase(&token.text) && is_verb_token(token)
}

fn is_italian(language: &str) -> bool {
    matches!(language.split(['-', '_']).next(), Some("it"))
}

/// is_verb_token reads the model evidence. A `VERB`/`AUX` tag is decisive; a
/// lemma that is an infinitive different from the surface (`sorge` ->
/// `sorgere`) is the fallback for an ambiguous tag. A noun whose lemma equals
/// its surface (`guarda` -> `guarda`, `università` -> `università`) is never a
/// predicate, which is why the suffix list is no longer needed.
fn is_verb_token(token: &Token) -> bool {
    matches!(token.upos.as_deref(), Some("VERB" | "AUX")) || lemma_is_infinitive(token)
}

fn lemma_is_infinitive(token: &Token) -> bool {
    let Some(lemma) = token.lemma.as_deref() else {
        return false;
    };
    let lemma = lemma.to_lowercase();
    let surface = token.text.to_lowercase();
    // Every Italian infinitive ends in "re"; requiring a different, purely
    // alphabetic lemma rejects nouns that also end in "re" ("mare", "fiore")
    // and the dictionary form of an infinitive used as a noun.
    lemma != surface && lemma.ends_with("re") && lemma.chars().all(char::is_alphabetic)
}

#[cfg(test)]
#[path = "verb_tests.rs"]
mod tests;
