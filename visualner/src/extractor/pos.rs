//! Optional UDPipe part-of-speech tagging and lemmatization for the
//! extractor.
//!
//! When a UDPipe model for the requested language exists in the model
//! directory, every ICU token receives a universal POS tag (`upostag`) and its
//! lemma. Candidate construction uses the tag instead of hand-written stopword
//! lists, and `verb.rs` uses the lemma as the verb evidence, so the extractor
//! never guesses a predicate from Italian suffixes. Without a model, tokens
//! keep `upos == None`/`lemma == None` and the curated stopword lists remain
//! the fallback, so extraction never fails because a model is missing.
//!
//! Model directory: `$VISUALNER_MODEL_DIR`, or `$HOME/.local/share/visualner/models`.
//! Models are not bundled; download them from the UDPipe model page.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use super::Token;

thread_local! {
    static MODELS: RefCell<HashMap<String, Option<Rc<udpipe_rs::Model>>>> =
        RefCell::new(HashMap::new());
}

/// Universal POS tags that are phrase material (nouns, proper nouns, numbers).
const CONTENT_UPOS: &[&str] = &["NOUN", "PROPN", "NUM"];

/// Universal POS tags that never start or extend a visual phrase.
const FUNCTION_UPOS: &[&str] = &[
    "DET", "ADP", "CCONJ", "SCONJ", "PART", "INTJ", "PUNCT", "SYM", "VERB",
];

/// How many content words a token may look ahead for its UDPipe form. A
/// contraction is at most a few words ("dell'arte" is three), so a small
/// window is enough; bounding it prevents a skipped form from consuming a
/// later occurrence and desynchronizing the rest of the text.
const ALIGNMENT_WINDOW: usize = 16;

/// is_stop_token reports whether a token must break a candidate phrase.
/// Tags decide for nouns, numbers and function words. Adjectives, verbs,
/// adverbs, auxiliaries and unknown words (X) keep the curated stopword list,
/// because UDPipe tags those inconsistently ("olive" in "olive oil" is ADJ,
/// "OpenAI" is often X). Without a tag, the list decides.
pub(super) fn is_stop_token(token: &Token, language: &str) -> bool {
    if super::stopwords::is_title_word(&token.text) {
        return true;
    }
    match token.upos.as_deref() {
        Some(upos) if FUNCTION_UPOS.contains(&upos) => true,
        // A pronoun is usually non-visual, but acronym/proper-name POS errors
        // are common (for example OpenAI can be tagged PRON). Exclude only
        // pronouns already covered by the language stopword list.
        Some("PRON") => super::stopwords::is_stop_word(&token.text, language),
        Some(upos) if CONTENT_UPOS.contains(&upos) => false,
        _ => super::stopwords::is_stop_word(&token.text, language),
    }
}

/// tag_tokens fills `upos` and `lemma` by aligning the UDPipe words to the ICU
/// tokens in source order. It is a no-op when no model is available for
/// `language`.
///
/// UDPipe and ICU tokenize contractions differently: ICU keeps "dell'arte" as
/// one token, while UDPipe emits "di", "l'" and "arte". Matching by byte offset
/// desynchronized on the first contraction (the standalone "di" was found much
/// later in the text and every intervening token stayed untagged). The
/// alignment here walks the model words with a bounded window and matches a
/// word when its form is the token itself or a part of it ("l'", "arte"), so
/// the contraction keeps a tag and the scan never jumps to a later occurrence.
pub(super) fn tag_tokens(source_text: &str, language: &str, tokens: &mut [Token]) {
    let Some(model) = load_model(language) else {
        return;
    };
    let Ok(words) = model.parse(source_text) else {
        return;
    };
    let mut word_index = 0;
    for token in tokens.iter_mut() {
        let surface = token.text.to_lowercase();
        let mut chosen: Option<(String, String)> = None;
        // A contraction can hide a finite verb after its clitic ("C'è" is
        // "ci" + "è"): the verb decides that the surface is a clause, and
        // the clitic's PRON tag would otherwise hide it.
        let mut verb_part: Option<(String, String)> = None;
        let mut consumed = word_index;
        let mut unmatched_content = 0;
        let mut scan = word_index;
        while scan < words.len() {
            let word = &words[scan];
            let form = word.form.to_lowercase();
            if word_matches_token(&surface, &form) {
                // The first matched word supplies the tag; a PROPN inside the
                // token wins over the elided article ("dell'Abruzzo",
                // "L'Aquila") while "dell'arte" keeps the function reading.
                if chosen.is_none() || word.upostag == "PROPN" {
                    chosen = Some((word.upostag.clone(), word.lemma.clone()));
                }
                if matches!(word.upostag.as_str(), "VERB" | "AUX") {
                    verb_part = Some((word.upostag.clone(), word.lemma.clone()));
                }
                consumed = scan + 1;
            } else if word.upostag == "PUNCT" {
                // Consume punctuation only directly after a matched word.
                // Advancing for far punctuation left the scan at a later
                // position and skipped every token in between.
                if chosen.is_some() {
                    consumed = consumed.max(scan + 1);
                }
            } else if chosen.is_some() && token_tail_matches(&surface, &form) {
                // "dell'attenzione" is one ICU token but UDPipe emits
                // "di", "l'", "attenzione": consume the remainder of
                // the same token instead of leaving it for the next one.
                consumed = scan + 1;
                break;
            } else if chosen.is_some() {
                // The token is tagged and this is the next token's content
                // word: stop instead of consuming every later occurrence of
                // the same surface (the second "the" must stay for its own
                // token).
                break;
            } else {
                unmatched_content += 1;
                if unmatched_content > ALIGNMENT_WINDOW {
                    break;
                }
            }
            scan += 1;
        }
        // An articulated preposition ("del", "nel", "sul") is split by
        // UDPipe into preposition + article, so neither form is a substring
        // of the ICU token. Consume the preposition as this token's tag; the
        // article is skipped when the next content token matches its own
        // word, which keeps the alignment local instead of jumping ahead.
        if chosen.is_none() && consumed == word_index {
            if let Some(word) = words.get(word_index) {
                let contraction = semantic_core::lexicon::lexicon()
                    .stopwords
                    .articulated_prepositions
                    .iter()
                    .any(|preposition| preposition == &surface);
                if contraction && matches!(word.upostag.as_str(), "ADP" | "DET") {
                    chosen = Some((word.upostag.clone(), word.lemma.clone()));
                    consumed = word_index + 1;
                }
            }
        }
        word_index = consumed;
        if let Some((upos, lemma)) = verb_part.or(chosen) {
            token.upos = Some(upos);
            token.lemma = Some(lemma);
        }
    }
}

/// word_matches_token accepts an exact token, or a multi-character model form
/// contained in the token. Single characters only match exactly: otherwise a
/// model word "a" would match every token that contains the letter "a".
fn word_matches_token(surface: &str, form: &str) -> bool {
    if form.is_empty() {
        return false;
    }
    if surface == form {
        return true;
    }
    // Only an elided form is a part of a longer ICU token: UDPipe splits
    // "dell'arte" into "di" + "l'" + "arte". A plain short word is never a
    // part of another word ("con" is not inside "connessione"), which is what
    // used to desynchronize the whole alignment and leave a scene untagged.
    let elided = form.ends_with('\'') || form.ends_with('\u{2019}');
    (elided && form.chars().count() >= 2 && surface.contains(form))
        || token_tail_matches(surface, form)
}

/// True when `form` is the part of an elided ICU token after its apostrophe
/// ("dell'attenzione" -> "attenzione").
fn token_tail_matches(surface: &str, form: &str) -> bool {
    surface
        .rsplit_once(['\'', '\u{2019}'])
        .is_some_and(|(_, tail)| !tail.is_empty() && tail == form)
}

fn load_model(language: &str) -> Option<Rc<udpipe_rs::Model>> {
    let key = language.trim().to_lowercase();
    MODELS.with(|cache| {
        if let Some(entry) = cache.borrow().get(&key) {
            return entry.clone();
        }
        let loaded = match model_path(&key) {
            Some(path) => match udpipe_rs::Model::load(&path) {
                Ok(model) => Some(Rc::new(model)),
                Err(error) => {
                    eprintln!(
                        "visualner: UDPipe model {} failed to load for {key}: {error}; continuing with the model-free fallback",
                        path.display()
                    );
                    None
                }
            },
            None => {
                eprintln!(
                    "visualner: no UDPipe model installed for {key}; POS evidence and verb filtering are disabled for this language"
                );
                None
            }
        };
        cache.borrow_mut().insert(key, loaded.clone());
        loaded
    })
}

/// model_path resolves the UDPipe file for a language tag inside the model
/// directory. The treebank per language is data (`language_profiles.json`),
/// so adding a language is an entry, never a recompile. A language without a
/// certified entry has no model and keeps the model-free fallback.
pub(super) fn model_path(language: &str) -> Option<PathBuf> {
    let dir = match std::env::var_os("VISUALNER_MODEL_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/share/visualner/models"),
    };
    let treebank = semantic_core::lexicon::lexicon()
        .language_profiles
        .treebank(language)?;
    let file = udpipe_rs::model_filename(treebank);
    let path = dir.join(file);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, upos: Option<&str>) -> Token {
        Token {
            text: text.to_string(),
            start: 0,
            end: text.len(),
            upos: upos.map(str::to_string),
            lemma: None,
        }
    }

    #[test]
    fn pos_tag_overrides_stopword_list() {
        // "company" is in the stopword list, but a NOUN tag makes it content.
        assert!(is_stop_token(&token("company", None), "en"));
        assert!(!is_stop_token(&token("company", Some("NOUN")), "en"));
        assert!(!is_stop_token(&token("Jordan", Some("PROPN")), "en"));
        assert!(is_stop_token(&token("the", Some("DET")), "en"));
        // Verbs and adjectives defer to the stopword list.
        assert!(is_stop_token(&token("imagine", Some("VERB")), "en"));
        assert!(is_stop_token(&token("si", Some("PRON")), "it"));
        assert!(!is_stop_token(&token("olive", Some("ADJ")), "en"));
    }

    #[test]
    fn fallback_uses_stopword_list_without_tags() {
        assert!(is_stop_token(&token("the", None), "en"));
        assert!(!is_stop_token(&token("feta", None), "en"));
    }

    #[test]
    fn contraction_parts_match_inside_one_icu_token() {
        assert!(word_matches_token("dell'arte", "l'"));
        assert!(word_matches_token("dell'arte", "arte"));
        assert!(!word_matches_token("dell'arte", "di"));
        assert!(word_matches_token("l'aquila", "aquila"));
        // A one-character form never matches inside a longer word.
        assert!(!word_matches_token("casa", "a"));
        assert!(word_matches_token("a", "a"));
        assert!(!word_matches_token("corte", ""));
    }
}
