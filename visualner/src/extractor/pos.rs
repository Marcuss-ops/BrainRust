//! Optional UDPipe part-of-speech tagging for the extractor.
//!
//! When a UDPipe model for the requested language exists in the model
//! directory, every ICU token receives a universal POS tag (`upostag`), and
//! candidate construction uses that tag instead of hand-written stopword
//! lists. Without a model, tokens keep `upos == None` and the curated
//! stopword lists remain the fallback, so extraction never fails because a
//! model is missing.
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
const FUNCTION_UPOS: &[&str] = &["DET", "ADP", "CCONJ", "SCONJ", "PART", "INTJ", "PUNCT", "SYM", "VERB"];

/// is_stop_token reports whether a token must break a candidate phrase.
/// Tags decide for nouns, numbers and function words. Adjectives, verbs,
/// adverbs, auxiliaries and unknown words (X) keep the curated stopword list,
/// because UDPipe tags those inconsistently ("olive" in "olive oil" is ADJ,
/// "OpenAI" is often X). Without a tag, the list decides.
pub(super) fn is_stop_token(token: &Token) -> bool {
    if super::stopwords::is_title_word(&token.text) {
        return true;
    }
    match token.upos.as_deref() {
        Some(upos) if FUNCTION_UPOS.contains(&upos) => true,
        Some(upos) if CONTENT_UPOS.contains(&upos) => false,
        _ => super::stopwords::is_stop_word(&token.text),
    }
}

/// tag_tokens fills `upos` for every token whose start offset matches a
/// word that UDPipe tokenized from `source_text`. It is a no-op when no model
/// is available for `language`.
pub(super) fn tag_tokens(source_text: &str, language: &str, tokens: &mut [Token]) {
    let Some(model) = load_model(language) else {
        return;
    };
    let Ok(words) = model.parse(source_text) else {
        return;
    };
    // UDPipe forms are verbatim substrings of the source, in order, so a
    // forward scan recovers each word's byte offset.
    let mut cursor = 0;
    let mut word_starts: Vec<(usize, String)> = Vec::new();
    for word in words {
        if word.form.is_empty() {
            continue;
        }
        let Some(relative) = source_text[cursor..].find(&word.form) else {
            continue;
        };
        let start = cursor + relative;
        cursor = start + word.form.len();
        word_starts.push((start, word.upostag));
    }
    // One ICU token can cover several UDPipe words ("dell'arte" becomes
    // "di" + "l'" + "arte"), so the first word that starts inside the
    // token's byte span decides its tag. Word starts are ascending, so a
    // binary search finds it without scanning every token.
    for token in tokens.iter_mut() {
        let index = word_starts.partition_point(|(start, _)| *start < token.start);
        if let Some((start, upos)) = word_starts.get(index) {
            if *start < token.end {
                token.upos = Some(upos.clone());
            }
        }
    }
}

fn load_model(language: &str) -> Option<Rc<udpipe_rs::Model>> {
    let key = language.trim().to_lowercase();
    MODELS.with(|cache| {
        if let Some(entry) = cache.borrow().get(&key) {
            return entry.clone();
        }
        let loaded = model_path(&key)
            .and_then(|path| udpipe_rs::Model::load(path).ok())
            .map(Rc::new);
        cache.borrow_mut().insert(key, loaded.clone());
        loaded
    })
}

/// model_path resolves the UDPipe file for a language tag inside the model
/// directory. Only languages with a known file name are supported.
fn model_path(language: &str) -> Option<PathBuf> {
    let dir = match std::env::var_os("VISUALNER_MODEL_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?)
            .join(".local/share/visualner/models"),
    };
    // Treebank per language, matching the codes in config/multilingual.yaml.
    let treebank = match language.split(['-', '_']).next()? {
        "en" => "english-ewt",
        "it" => "italian-isdt",
        "pl" => "polish-pdb",
        "ru" => "russian-syntagrus",
        "de" => "german-gsd",
        "es" => "spanish-gsd",
        "pt" => "portuguese-gsd",
        "fr" => "french-gsd",
        "tr" => "turkish-imst",
        "id" => "indonesian-gsd",
        _ => return None,
    };
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
        }
    }

    #[test]
    fn pos_tag_overrides_stopword_list() {
        // "company" is in the stopword list, but a NOUN tag makes it content.
        assert!(is_stop_token(&token("company", None)));
        assert!(!is_stop_token(&token("company", Some("NOUN"))));
        assert!(!is_stop_token(&token("Jordan", Some("PROPN"))));
        assert!(is_stop_token(&token("the", Some("DET"))));
        // Verbs and adjectives defer to the stopword list.
        assert!(is_stop_token(&token("imagine", Some("VERB"))));
        assert!(!is_stop_token(&token("olive", Some("ADJ"))));
    }

    #[test]
    fn fallback_uses_stopword_list_without_tags() {
        assert!(is_stop_token(&token("the", None)));
        assert!(!is_stop_token(&token("feta", None)));
    }
}
