//! Locale-scoped inflected place resolution and its named negative list.

use semantic_core::lexicon::lexicon;

#[test]
fn four_measured_slavic_surfaces_resolve_to_their_canonical_place() {
    for (surface, language, canonical) in [
        ("Poznaniu", "pl", "Poznań"),
        ("Krakowie", "pl", "Kraków"),
        ("Казани", "ru", "Казань"),
        ("Самаре", "ru", "Самара"),
    ] {
        let lexicon = lexicon();
        assert!(
            lexicon.is_place(surface, language),
            "{surface} must be a place in {language}"
        );
        assert_eq!(
            lexicon.canonical_place(surface, language),
            Some(canonical),
            "{surface} must resolve to {canonical}"
        );
    }
}

#[test]
fn query_form_is_language_scoped_not_global() {
    let lexicon = lexicon();
    assert_eq!(
        lexicon.canonical_query_place("Poznaniu", "pl"),
        Some("Poznań"),
        "Polish uses the nominative apposition in the query"
    );
    assert_eq!(
        lexicon.canonical_query_place("Казани", "ru"),
        None,
        "Russian keeps the source case in the query"
    );
}

#[test]
fn named_negative_words_never_become_places() {
    for (surface, language) in [
        ("Daniu", "pl"),
        ("Staniu", "pl"),
        ("Potem", "pl"),
        ("Banie", "ru"),
        ("Маме", "ru"),
        ("Папе", "ru"),
    ] {
        assert!(
            !lexicon().is_place(surface, language),
            "{surface} is an ordinary word in {language}, not a place"
        );
    }
}

#[test]
fn inflections_are_not_global_fuzzy_matches() {
    let lexicon = lexicon();
    assert!(
        !lexicon.is_place("Poznaniu", "de"),
        "a Polish locative is not a German place"
    );
    assert!(
        !lexicon.is_place("krakowie", "pl"),
        "a lowercase common word never becomes a place"
    );
    assert!(
        !lexicon.is_place("Krakowianin", "pl"),
        "a derived demonym is not the alias"
    );
}
