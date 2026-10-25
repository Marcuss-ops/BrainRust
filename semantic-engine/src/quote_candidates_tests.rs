use super::*;

#[test]
fn quote_candidates_cover_every_explicit_delimiter_pair() {
    let cases = [
        ("Ha detto “prima frase” oggi.", "prima frase"),
        ("Ha detto «seconda frase» oggi.", "seconda frase"),
        ("Ha detto „terza frase“ oggi.", "terza frase"),
        ("Ha detto \"quarta frase\" oggi.", "quarta frase"),
    ];
    for (source, expected) in cases {
        let sentences = semantic_core::split_sentences(source, "it", None);
        let quotes = quote_candidates(source, &sentences);
        assert!(
            quotes
                .iter()
                .any(|candidate| candidate.text.contains(expected)),
            "quote pair for {expected:?} produced no QUOTE candidate: {quotes:?}"
        );
        assert!(quotes.iter().all(|candidate| candidate.kind == "QUOTE"));
    }
}

#[test]
fn single_apostrophes_are_not_quote_delimiters() {
    let source = "L'arte di Marco Rossi non è 'una citazione'.";
    let sentences = semantic_core::split_sentences(source, "it", None);
    let quotes = quote_candidates(source, &sentences);
    assert!(
        quotes.is_empty(),
        "apostrophes/single quotes must never delimit a QUOTE candidate: {quotes:?}"
    );
}
