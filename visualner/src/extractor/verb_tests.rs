use super::*;

fn token(text: &str, upos: Option<&str>, lemma: Option<&str>) -> Token {
    Token {
        text: text.to_string(),
        start: 0,
        end: text.len(),
        upos: upos.map(str::to_string),
        lemma: lemma.map(str::to_string),
    }
}

fn tokens(words: &[(&str, Option<&str>, Option<&str>)]) -> Vec<Token> {
    words
        .iter()
        .map(|(text, upos, lemma)| token(text, *upos, *lemma))
        .collect()
}

#[test]
fn tagged_verbs_are_predicates() {
    // "…e sorge nella pianura": the model tags the verb.
    let words = tokens(&[
        ("Lombardia", Some("PROPN"), Some("Lombardia")),
        ("e", Some("CCONJ"), Some("e")),
        ("sorge", Some("VERB"), Some("sorgere")),
        ("nella", Some("ADP"), Some("in")),
        ("pianura", Some("NOUN"), Some("pianura")),
    ]);
    assert!(is_verbal_predicate(&words, 2, 3, "it"));
}

#[test]
fn infinitive_lemma_catches_an_ambiguous_tag() {
    // The tagger can label an inflected verb ADJ/X; the lemma still says
    // "ospitare", so the candidate is a predicate.
    let words = tokens(&[
        ("Piemonte", Some("PROPN"), Some("Piemonte")),
        ("e", Some("CCONJ"), Some("e")),
        ("ospita", Some("ADJ"), Some("ospitare")),
        ("la", Some("DET"), Some("la")),
        ("Mole", Some("PROPN"), Some("Mole")),
    ]);
    assert!(is_verbal_predicate(&words, 2, 3, "it"));
    // Without any tag, the lemma alone is still evidence.
    let words = tokens(&[("e", None, None), ("guarda", None, Some("guardare"))]);
    assert!(is_verbal_predicate(&words, 1, 2, "it"));
}

#[test]
fn verb_filter_and_suffixes_are_data_per_language() {
    // The same tagged predicate is filtered in Italian (certified) and kept
    // in German (not certified yet): the gate is the profile entry, not a
    // hardcoded list of languages.
    let words = tokens(&[("baut", Some("VERB"), Some("bauen"))]);
    assert!(is_verbal_predicate(&words, 0, 1, "it"));
    assert!(!is_verbal_predicate(&words, 0, 1, "de"));
    // German declares no infinitive suffixes, so a lemma alone cannot decide
    // a predicate there.
    let words = tokens(&[("baut", Some("X"), Some("bauen"))]);
    assert!(!is_verbal_predicate(&words, 0, 1, "de"));
}

#[test]
fn clitic_plus_verb_is_one_predicate() {
    let words = tokens(&[
        ("Torino", Some("PROPN"), Some("Torino")),
        ("si", Some("PRON"), Some("si")),
        ("trova", Some("VERB"), Some("trovare")),
        ("in", Some("ADP"), Some("in")),
        ("Piemonte", Some("PROPN"), Some("Piemonte")),
    ]);
    assert!(is_verbal_predicate(&words, 1, 3, "it"));
}

#[test]
fn nominal_lemma_keeps_the_concept_reading() {
    // "La guarda del castello": the noun's lemma equals its surface.
    let words = tokens(&[
        ("La", Some("DET"), Some("la")),
        ("guarda", Some("NOUN"), Some("guarda")),
        ("del", Some("ADP"), Some("di")),
        ("castello", Some("NOUN"), Some("castello")),
    ]);
    assert!(!is_verbal_predicate(&words, 1, 2, "it"));
    // "Il salto della quaglia": same surface, nominal lemma.
    let words = tokens(&[
        ("Il", Some("DET"), Some("il")),
        ("salto", Some("NOUN"), Some("salto")),
        ("della", Some("ADP"), Some("di")),
        ("quaglia", Some("NOUN"), Some("quaglia")),
    ]);
    assert!(!is_verbal_predicate(&words, 1, 2, "it"));
    let words = tokens(&[
        ("università", Some("NOUN"), Some("università")),
        ("della", Some("ADP"), Some("di")),
        ("città", Some("NOUN"), Some("città")),
    ]);
    assert!(!is_verbal_predicate(&words, 0, 1, "it"));
}

#[test]
fn proper_names_are_never_dropped() {
    // "Roma e Milano": conjunct name, not a predicate, even if the tagger
    // returns an odd lemma for the capitalized surface.
    let words = tokens(&[
        ("Roma", Some("PROPN"), Some("Roma")),
        ("e", Some("CCONJ"), Some("e")),
        ("Milano", Some("PROPN"), Some("Milano")),
    ]);
    assert!(!is_verbal_predicate(&words, 2, 3, "it"));
}

#[test]
fn no_model_never_guesses_from_the_surface() {
    // No tag and no lemma: the extractor keeps the model-free behavior
    // instead of reintroducing suffix heuristics.
    let words = tokens(&[("e", None, None), ("sorge", None, None)]);
    assert!(!is_verbal_predicate(&words, 1, 2, "it"));
}

#[test]
fn other_languages_keep_their_behavior() {
    let words = tokens(&[
        ("and", Some("CCONJ"), Some("and")),
        ("sorge", Some("VERB"), Some("sorgere")),
        ("in", Some("ADP"), Some("in")),
        ("the", Some("DET"), Some("the")),
        ("valley", Some("NOUN"), Some("valley")),
    ]);
    assert!(!is_verbal_predicate(&words, 1, 2, "en"));
}

#[test]
fn a_finite_verb_breaks_a_noun_phrase() {
    let words = tokens(&[
        ("I", Some("DET"), Some("il")),
        ("tifosi", Some("NOUN"), Some("tifoso")),
        ("festeggiano", Some("VERB"), Some("festeggiare")),
        ("sugli", Some("ADP"), Some("su")),
        ("spalti", Some("NOUN"), Some("spalto")),
    ]);
    assert!(is_verbal_continuation(&words, 2, "it"));
    assert!(!is_verbal_continuation(&words, 1, "it"));
}
