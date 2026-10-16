use super::*;

fn token(text: &str) -> Token {
    Token {
        text: text.to_string(),
        start: 0,
        end: text.len(),
        upos: None,
    }
}

fn tokens(words: &[&str]) -> Vec<Token> {
    words.iter().map(|word| token(word)).collect()
}

#[test]
fn clause_linker_plus_complement_is_a_predicate() {
    // "…e sorge nella pianura": linker left slot, morphology, locative right.
    let words = tokens(&["Lombardia", "e", "sorge", "nella", "pianura"]);
    assert!(is_verbal_predicate(&words, 2, 3, "it"));
    // "…e ospita la Mole": accusative determiner right slot.
    let words = tokens(&["Piemonte", "e", "ospita", "la", "Mole"]);
    assert!(is_verbal_predicate(&words, 2, 3, "it"));
    // "…e guarda il mare".
    let words = tokens(&["Puglia", "e", "guarda", "il", "mare"]);
    assert!(is_verbal_predicate(&words, 2, 3, "it"));
}

#[test]
fn clitic_plus_verb_phrases_are_predicates() {
    let words = tokens(&["Torino", "si", "trova", "in", "Piemonte"]);
    assert!(is_verbal_predicate(&words, 1, 3, "it"));
    let words = tokens(&["Napoli", "si", "affaccia", "sul", "golfo"]);
    assert!(is_verbal_predicate(&words, 1, 3, "it"));
}

#[test]
fn proper_noun_subject_slot_is_a_predicate() {
    // "Torino ospita la Mole": capitalized subject, accusative object.
    let words = tokens(&["Torino", "ospita", "la", "Mole"]);
    assert!(is_verbal_predicate(&words, 1, 2, "it"));
}

#[test]
fn nominal_slot_wins_over_verb_morphology() {
    // "La guarda del castello": determiner before forces the noun reading.
    let words = tokens(&["La", "guarda", "del", "castello"]);
    assert!(!is_verbal_predicate(&words, 1, 2, "it"));
    // "Il salto della quaglia".
    let words = tokens(&["Il", "salto", "della", "quaglia"]);
    assert!(!is_verbal_predicate(&words, 1, 2, "it"));
    // "capoluogo della Lombardia": preposition before.
    let words = tokens(&["della", "Lombardia"]);
    assert!(!is_verbal_predicate(&words, 1, 2, "it"));
}

#[test]
fn noun_coordination_survives_the_linker_slot() {
    // "…e vino." — end of text is not a verbal continuation.
    let words = tokens(&["pane", "e", "vino"]);
    assert!(!is_verbal_predicate(&words, 2, 3, "it"));
    // "…e vino del Chianti" — genitive complements are nominal.
    let words = tokens(&["pane", "e", "vino", "del", "Chianti"]);
    assert!(!is_verbal_predicate(&words, 2, 3, "it"));
    // "…e latte fresco" — an adjective is not a verbal continuation.
    let words = tokens(&["pane", "e", "latte", "fresco"]);
    assert!(!is_verbal_predicate(&words, 2, 3, "it"));
}

#[test]
fn unmistakable_noun_endings_are_never_verbal_morphology() {
    let words = tokens(&["Milano", "e", "università", "della", "città"]);
    assert!(!is_verbal_predicate(&words, 2, 3, "it"));
}

#[test]
fn proper_names_are_never_dropped() {
    // "Roma e Milano": conjunct name, not a predicate.
    let words = tokens(&["Roma", "e", "Milano", "sono", "città"]);
    assert!(!is_verbal_predicate(&words, 2, 3, "it"));
}

#[test]
fn other_languages_keep_their_behavior() {
    let words = tokens(&["and", "sorge", "in", "the", "valley"]);
    assert!(!is_verbal_predicate(&words, 1, 2, "en"));
}
