use super::*;

#[test]
fn embedded_tables_load_with_the_documented_shape() {
    let lexicon = Lexicon::global();
    assert!(lexicon
        .stopwords
        .generic_phrases
        .iter()
        .any(|p| p == "imagine"));
    assert_eq!(lexicon.object_terms("en").len(), 24);
    assert_eq!(lexicon.object_terms("it").len(), 31);
    // BCP-47 subtags resolve to the base language.
    assert_eq!(
        lexicon.object_terms("pt-BR").len(),
        lexicon.object_terms("pt").len()
    );
    assert!(lexicon.object_terms("xx").is_empty());
    assert_eq!(lexicon.process_phases.phases.len(), 4);
    assert_eq!(lexicon.relation.relation_markers.len(), 7);
}

#[test]
fn filler_phrases_match_as_consecutive_words_never_inside_a_word() {
    let stopwords = &Lexicon::global().stopwords;
    // The documented noise examples, matched inside a longer sentence and
    // across punctuation and case.
    assert!(stopwords.contains_filler_phrase("Non è così."));
    assert!(stopwords.contains_filler_phrase("... e ANCORA OGGI la città ricorda."));
    assert!(stopwords.contains_filler_phrase("Per questo, il caso resta aperto."));
    assert!(stopwords.contains_filler_phrase("encore aujourd'hui Paris est lì"));
    // Never inside a bigger word, never on a partial overlap.
    assert!(!stopwords.contains_filler_phrase("percussione del caso"));
    assert!(!stopwords.contains_filler_phrase("il caso resta aperto"));
    assert!(!stopwords.contains_filler_phrase(""));
}

#[test]
fn geonames_places_are_indexed_with_a_capitalization_guard() {
    let lexicon = Lexicon::global();
    assert!(lexicon.is_place("Pavia", "it"));
    assert!(lexicon.is_place("Garlasco", "it"));
    assert!(lexicon.is_place("Vigevano", "it"));
    // A caseless script has no uppercase initial: the index must still
    // type the place.
    assert!(lexicon.is_place("北京", "zh"));
    assert!(lexicon.is_place("東京", "ja"));
    assert!(lexicon.is_place("서울", "ko"));
    assert!(lexicon.is_place("القاهرة", "ar"));
    assert!(lexicon.is_place("मुंबई", "hi"));
    // German is cased even though nouns are capitalized: the lowercase
    // guard still applies.
    assert!(lexicon.is_place("Berlin", "de"));
    assert!(!lexicon.is_place("berlin", "de"));
    // Lowercase common nouns that collide with a GeoNames alias stay out.
    assert!(!lexicon.is_place("casa", "it"));
    assert!(!lexicon.is_place("mare", "it"));
    assert!(!lexicon.is_place("pavia", "it"));
    // "Corte" is both a French town and the Italian common noun "court".
    assert!(!lexicon.is_place("Corte", "it"));
    assert!(lexicon.is_place("Corte", "en"));
}

#[test]
fn language_profiles_own_the_per_language_conventions() {
    let profiles = &Lexicon::global().language_profiles;
    // Treebank: BCP-47 subtags resolve to the base language, unknown
    // languages have none and never borrow `default`.
    assert_eq!(profiles.treebank("it"), Some("italian-isdt"));
    assert_eq!(profiles.treebank("pt-BR"), Some("portuguese-gsd"));
    assert_eq!(profiles.treebank(" DE "), Some("german-gsd"));
    assert_eq!(profiles.treebank("zh"), Some("chinese-gsdsimp"));
    // A language without a certified model never borrows another one.
    assert_eq!(profiles.treebank("ko"), None);
    assert_eq!(profiles.treebank(""), None);

    // Date order and numeric separators are facts, not a `language == "en"`.
    assert_eq!(profiles.date_order("en"), DateOrder::MonthFirst);
    assert_eq!(profiles.date_order("it"), DateOrder::DayFirst);
    assert_eq!(profiles.date_order("xx"), DateOrder::DayFirst);
    assert_eq!(profiles.decimal_separator("en"), '.');
    assert_eq!(profiles.thousands_separator("en"), ',');
    assert_eq!(profiles.decimal_separator("it"), ',');
    assert_eq!(profiles.thousands_separator("it"), '.');
    assert_eq!(profiles.thousands_separator("xx"), '.');

    // Verbal morphology is certified per language, never inferred from a
    // suffix list that happens to be non-empty.
    assert!(profiles.verb_filter("it"));
    assert!(!profiles.verb_filter("de"));
    assert_eq!(
        profiles.infinitive_suffixes("it"),
        ["re".to_string()].as_slice()
    );
    assert!(profiles.infinitive_suffixes("de").is_empty());
}

#[test]
fn writing_systems_decide_whether_case_is_name_evidence() {
    let profiles = &Lexicon::global().language_profiles;
    // Cased scripts.
    assert!(profiles.is_cased("en"));
    assert!(profiles.is_cased("ru"));
    // Caseless scripts have no uppercase initial to test.
    assert!(!profiles.is_cased("zh"));
    assert!(!profiles.is_cased("ja"));
    assert!(!profiles.is_cased("ar"));
    assert!(!profiles.is_cased("th"));
    // Unknown languages keep the historical cased behavior.
    assert!(profiles.is_cased("xx"));
    // German capitalizes common nouns: the case test carries no signal.
    assert!(profiles.is_cased("de"));
    assert!(profiles.capitalized_nouns("de"));
    assert!(!profiles.uppercase_is_name_evidence("de"));
    assert!(profiles.uppercase_is_name_evidence("it"));
    assert!(!profiles.uppercase_is_name_evidence("zh"));
    // The certified Spanish and Portuguese models lowercase PROPN lemmas.
    assert!(profiles.lowercase_propn_lemmas("es"));
    assert!(profiles.lowercase_propn_lemmas("pt"));
    assert!(!profiles.lowercase_propn_lemmas("it"));
}

#[test]
fn minor_place_aliases_do_not_override_person_names() {
    let lexicon = Lexicon::global();
    // "João Ribeiro" is an alias of a 14k-population town.
    assert!(!lexicon.is_place("João Ribeiro", "pt"));
    // A primary name, a seed city and a Cyrillic seed city stay places.
    assert!(lexicon.is_place("Coimbra", "pt"));
    assert!(lexicon.is_place("New York", "en"));
    assert!(lexicon.is_place("Пермь", "ru"));
}

#[test]
fn embedded_language_profiles_pass_their_own_validation() {
    let problems = Lexicon::global().language_profiles.validate();
    assert!(
        problems.is_empty(),
        "invalid language profile table: {problems:#?}"
    );
}

#[test]
fn profile_validation_names_each_broken_entry() {
    let broken: LanguageProfiles = serde_json::from_value(serde_json::json!({
        "profiles": {
            "de": { "script": "klingon" },
            "it": { "verb_filter": true, "script": "latin" },
            "xx": { "treebank": "  " }
        },
        "scripts": { "latin": { "cased": true }, "broken": {} }
    }))
    .expect("the shape parses so validation can name the problems");
    let joined = broken.validate().join("\n");
    assert!(joined.contains("unknown script \"klingon\""), "{joined}");
    assert!(joined.contains("missing mandatory `script`"), "{joined}");
    assert!(joined.contains("empty `treebank`"), "{joined}");
    assert!(
        joined.contains("needs at least one `infinitive_suffixes`"),
        "{joined}"
    );
    assert!(joined.contains("missing mandatory `cased`"), "{joined}");
}

#[test]
fn orthography_tables_supply_abbreviations_and_eras() {
    let orthography = &Lexicon::global().orthography;
    let italian: Vec<&str> = orthography.abbreviations.for_language("it").collect();
    assert!(italian.contains(&"dott."));
    assert!(
        italian.contains(&"etc."),
        "common abbreviations apply to every language"
    );
    let german: Vec<&str> = orthography.abbreviations.for_language("de").collect();
    assert!(german.contains(&"z.b."));
    assert!(!german.contains(&"dott."));
    assert!(orthography
        .era_markers
        .iter()
        .any(|marker| marker.marker == "b.c." && marker.era == "bce"));
}

#[test]
fn discourse_cues_fall_back_to_the_default_language() {
    let lexicon = Lexicon::global();
    assert_eq!(
        lexicon.discourse.content_cues_for("it"),
        lexicon.discourse.content_cues.get("it").unwrap().as_slice()
    );
    let fallback = lexicon.discourse.content_cues_for("xx");
    let default = lexicon.discourse.content_cues.get("default").unwrap();
    assert_eq!(fallback, default.as_slice());
}

#[test]
fn order_sensitve_tables_keep_their_data_order() {
    // Longer markers must precede their fragments ("a causa di" before
    // "a causa"), so the first match carries the sharper meaning.
    let lexicon = Lexicon::global();
    assert_eq!(lexicon.discourse.causal_after[0], ["a", "causa", "di"]);
    let legal = crate::legal_packs::get("it").expect("Italian legal pack");
    assert_eq!(legal.patterns[0].normalized, "not_convicted");
}
