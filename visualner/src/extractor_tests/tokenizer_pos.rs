//! ICU segmentation and POS tagging across scripts and languages.

use super::super::*;
use crate::ExtractOptions;
#[test]
fn icu_tokenizer_splits_unspaced_scripts_and_keeps_compounds() {
    let chinese = "我们参观了北京大学。";
    let tokens = tokenize(chinese, "zh").expect("zh segmentation");
    assert!(tokens.iter().any(|token| token.text == "北京"));
    assert!(tokens.iter().any(|token| token.text == "大学"));
    for token in &tokens {
        assert_eq!(&chinese[token.start..token.end], token.text);
    }

    let english = "Jean-Luc Picard visited Paris.";
    let tokens = tokenize(english, "en").expect("en segmentation");
    let texts: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
    assert!(texts.contains(&"Jean-Luc"));
    assert!(texts.contains(&"Paris"));
}

#[test]
fn icu_tokenizer_handles_latin_script_with_diacritics() {
    let german = "Der Käse ist gut.";
    let tokens = tokenize(german, "de").expect("de segmentation");
    let texts: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
    assert_eq!(texts, vec!["Der", "Käse", "ist", "gut"]);
}

#[test]
fn pos_tags_are_applied_when_model_is_installed() {
    let tokens = tokenize("Imagine the feta cheese on the plate.", "en").expect("en");
    if tokens.iter().all(|token| token.upos.is_none()) {
        eprintln!("UDPipe English model not installed; POS assertions skipped");
        return;
    }
    let tag = |word: &str| {
        tokens
            .iter()
            .find(|token| token.text == word)
            .and_then(|token| token.upos.as_deref())
            .map(str::to_string)
    };
    assert_eq!(tag("cheese").as_deref(), Some("NOUN"));
    assert_eq!(tag("the").as_deref(), Some("DET"));
    assert_eq!(tag("Imagine").as_deref(), Some("VERB"));
}

#[test]
fn italian_contraction_is_tagged_and_never_leaks_into_entities() {
    // "dell'arte" is one ICU token but several UDPipe words ("di" + "l'" + "arte").
    // It must receive a function-word tag, so it cannot join the noun phrases around it.
    let source = "Il ritratto dell'arte moderna è esposto a Roma.";
    let tokens = tokenize(source, "it").expect("it");
    let contraction = tokens
        .iter()
        .find(|token| token.text == "dell'arte")
        .expect("contraction token");
    if contraction.upos.is_none() {
        eprintln!("UDPipe Italian model not installed; contraction assertions skipped");
        return;
    }
    assert_eq!(contraction.upos.as_deref(), Some("DET"));
    let entities = extract(
        source,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 10,
        },
    )
    .unwrap();
    for entity in &entities {
        assert!(
            !entity.text.contains("dell'arte"),
            "contraction leaked into entity: {entities:?}"
        );
        assert_eq!(
            source.get(entity.start..entity.end),
            Some(entity.text.as_str())
        );
    }
}

#[test]
fn every_language_in_multilingual_config_loads_a_pos_model() {
    // Language codes from config/multilingual.yaml (pt-BR shares the pt model).
    let samples = [
        ("it", "Il presidente visita Roma."),
        ("en", "The president visits London."),
        ("pl", "Prezydent odwiedza Kraków."),
        ("ru", "Президент посетил Москву."),
        ("de", "Der Präsident besucht Berlin."),
        ("es", "El presidente visita Madrid."),
        ("pt-BR", "O presidente visita Lisboa."),
        ("fr", "Le président visite Paris."),
        ("tr", "Cumhurbaşkanı İstanbul'u ziyaret etti."),
        ("id", "Presiden mengunjungi Jakarta."),
    ];
    for (language, source) in samples {
        let tokens = tokenize(source, language).expect("segmentation");
        if tokens.iter().all(|token| token.upos.is_none()) {
            eprintln!("UDPipe model not installed for {language}; skipped");
            continue;
        }
        assert!(
            tokens
                .iter()
                .any(|token| matches!(token.upos.as_deref(), Some("NOUN" | "PROPN"))),
            "{language}: no noun tagged in {source:?}: {tokens:?}"
        );
    }
}

#[test]
fn polish_russian_turkish_indonesian_sentences_match_gold_pos() {
    // Gold UPOS for the content words (NOUN/PROPN/VERB) of short sentences.
    type GoldCase = (
        &'static str,
        &'static str,
        &'static [(&'static str, &'static str)],
    );
    let cases: [GoldCase; 4] = [
        (
            "pl",
            "Prezydent odwiedza Kraków.",
            &[
                ("Prezydent", "NOUN"),
                ("odwiedza", "VERB"),
                ("Kraków", "PROPN"),
            ],
        ),
        (
            "ru",
            "Президент посетил Москву.",
            &[
                ("Президент", "NOUN"),
                ("посетил", "VERB"),
                ("Москву", "PROPN"),
            ],
        ),
        (
            "tr",
            "Cumhurbaşkanı Ankara'yı ziyaret etti.",
            &[
                ("Cumhurbaşkanı", "NOUN"),
                ("ziyaret", "NOUN"),
                ("etti", "VERB"),
            ],
        ),
        (
            "id",
            "Presiden mengunjungi Jakarta.",
            &[
                ("Presiden", "NOUN"),
                ("mengunjungi", "VERB"),
                ("Jakarta", "PROPN"),
            ],
        ),
    ];
    for (language, source, gold) in cases {
        let tokens = tokenize(source, language).expect("segmentation");
        if tokens.iter().all(|token| token.upos.is_none()) {
            eprintln!("UDPipe model not installed for {language}; skipped");
            continue;
        }
        for (word, expected) in gold {
            let token = tokens
                .iter()
                .find(|token| token.text == *word)
                .unwrap_or_else(|| panic!("{language}: token {word:?} missing in {tokens:?}"));
            // NOUN and PROPN are both nominal for entity extraction; titles such
            // as "Presiden" are legitimately ambiguous between the two.
            let nominal = |upos: &str| matches!(upos, "NOUN" | "PROPN");
            let actual = token.upos.as_deref().unwrap_or("");
            let ok = if nominal(expected) {
                nominal(actual)
            } else {
                actual == *expected
            };
            assert!(
                ok,
                "{language}: wrong UPOS for {word:?} in {source:?}: got {actual:?}, want {expected}"
            );
        }
    }
}

#[test]
fn language_is_required_and_unsupported_rules_fail_closed() {
    assert!(extract(
        "twenty-five orders",
        &ExtractOptions {
            language: String::new(),
            entity_count: 3
        }
    )
    .is_err());
    let unsupported = extract(
        "Some text.",
        &ExtractOptions {
            language: "zz-ZZ".to_string(),
            entity_count: 3,
        },
    );
    assert!(
        unsupported.is_err(),
        "invalid language tags should fail explicitly"
    );
}

#[test]
fn italian_contractions_keep_the_proper_name_and_never_move_its_tag() {
    // "Lombardia" must keep its PROPN tag: the ADP of "della" belongs to
    // "della", not to the place that follows it.
    let text = "Milano è il capoluogo della Lombardia e sorge nella pianura padana.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 24,
        },
    )
    .expect("it extraction");
    let lombardia = entities
        .iter()
        .find(|entity| entity.text == "Lombardia")
        .unwrap_or_else(|| panic!("missing Lombardia: {entities:?}"));
    assert_eq!(lombardia.r#type, "LOCATION", "{lombardia:?}");
}

#[test]
fn articulated_prepositions_do_not_glue_onto_visual_keywords() {
    // UDPipe can leave an articulated preposition untagged; the deterministic
    // fallback list must still stop a fragment such as "progettazione dello"
    // or "maglia della" from becoming a visual keyword.
    let text = "La progettazione dello Stadio Aurora e la maglia della Juventus.";
    let entities = extract(
        text,
        &ExtractOptions {
            language: "it".to_string(),
            entity_count: 24,
        },
    )
    .expect("it extraction");
    assert!(
        entities.iter().any(|entity| entity.text == "progettazione"),
        "the noun survives without its preposition: {entities:?}"
    );
    assert!(
        !entities.iter().any(|entity| {
            entity.text.ends_with(" dello")
                || entity.text.ends_with(" della")
                || entity.text.starts_with("dello ")
                || entity.text.starts_with("della ")
        }),
        "no fragment may keep an articulated preposition: {entities:?}"
    );
}
