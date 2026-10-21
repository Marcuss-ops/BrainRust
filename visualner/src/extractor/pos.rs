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
const FUNCTION_UPOS: &[&str] = &[
    "DET", "ADP", "CCONJ", "SCONJ", "PART", "INTJ", "PUNCT", "SYM", "VERB",
];

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
        // A pronoun is usually non-visual, but acronym/proper-name POS errors
        // are common (for example OpenAI can be tagged PRON). Exclude only
        // pronouns already covered by the language stopword list.
        Some("PRON") => super::stopwords::is_stop_word(&token.text),
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
    // forward scan recovers each word's byte offset. The scan only accepts a
    // word occurrence: a purely alphanumeric form must begin a word, so a
    // function word can never be matched inside an unrelated longer word
    // ("di" of the contraction "della" inside "Lombardia" used to re-tag
    // Lombardia as ADP and delete the place from the extraction).
    let mut cursor = 0;
    let mut word_starts: Vec<(usize, String)> = Vec::new();
    for word in words {
        if word.form.is_empty() {
            continue;
        }
        let Some(start) = locate_word(source_text, &word.form, cursor) else {
            continue;
        };
        cursor = start + word.form.len();
        word_starts.push((start, word.upostag));
    }
    // One ICU token can cover several UDPipe words ("dell'arte" becomes
    // "di" + "l'" + "arte"), so the first word that starts inside the
    // token's byte span decides its tag. Word starts are ascending, so a
    // binary search finds it without scanning every token. A PROPN inside the
    // token overrides that first word: the contraction in "dell'Abruzzo" and
    // "L'Aquila" must not downgrade a proper name to its elided article,
    // while the common noun in "dell'arte" keeps the contract that makes the
    // whole contraction a function token.
    for token in tokens.iter_mut() {
        let index = word_starts.partition_point(|(start, _)| *start < token.start);
        let mut chosen: Option<&str> = None;
        for (start, upos) in word_starts.iter().skip(index) {
            if *start >= token.end {
                break;
            }
            if chosen.is_none() {
                chosen = Some(upos.as_str());
            }
            if upos == "PROPN" {
                chosen = Some(upos.as_str());
                break;
            }
        }
        if let Some(upos) = chosen {
            token.upos = Some(upos.to_string());
        }
    }
}

/// locate_word returns the first byte offset at or after `from` where `form`
/// occurs as a source word. A form made only of alphanumeric characters must
/// start a word (the preceding character is not alphanumeric), which is what
/// keeps a function-word component of a contraction from matching inside an
/// unrelated word. A form that carries punctuation (the elided clitic "l'")
/// is accepted wherever it matches, because ICU does not break the word it is
/// attached to.
fn locate_word(source_text: &str, form: &str, from: usize) -> Option<usize> {
    let word_bounded = form.chars().all(char::is_alphanumeric);
    let mut cursor = from;
    while let Some(relative) = source_text.get(cursor..)?.find(form) {
        let start = cursor + relative;
        let begins_word = source_text[..start]
            .chars()
            .next_back()
            .is_none_or(|previous| !previous.is_alphanumeric());
        if !word_bounded || begins_word {
            return Some(start);
        }
        cursor = start + form.len();
    }
    None
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
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/share/visualner/models"),
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
        assert!(is_stop_token(&token("si", Some("PRON"))));
        assert!(!is_stop_token(&token("olive", Some("ADJ"))));
    }
    #[test]
    fn fallback_uses_stopword_list_without_tags() {
        assert!(is_stop_token(&token("the", None)));
        assert!(!is_stop_token(&token("feta", None)));
    }

    #[test]
    fn locate_word_skips_matches_inside_a_longer_word() {
        // "di" (the first component of the Italian contraction "della") occurs
        // inside "Lombardia"; only the standalone occurrence is a word.
        let source = "capoluogo della Lombardia";
        assert_eq!(locate_word(source, "di", 0), None);
        assert_eq!(
            locate_word(source, "Lombardia", 0),
            source.find("Lombardia")
        );
        // A clitic carrying punctuation is matched where it is attached.
        let contraction = "dell'arte";
        assert_eq!(
            locate_word(contraction, "l'", 0),
            Some(contraction.find("l'").unwrap())
        );
    }
}
