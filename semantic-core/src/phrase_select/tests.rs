use super::*;

fn set(words: &[&str]) -> HashSet<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

fn profile(
    stop: &[&str],
    function: &[&str],
    verbs: &[&str],
    min: usize,
    max: usize,
    reject_verbs_when_all: bool,
) -> PhraseProfile {
    PhraseProfile {
        stop_words: set(stop),
        function_words: set(function),
        visual_verbs: set(verbs),
        policy: PhraseExtractionPolicy {
            min_words: min,
            max_words: max,
            max_results: 5,
            reject_verbs_when_all,
        },
    }
}

fn texts(selected: &[SelectedPhrase]) -> Vec<&str> {
    selected.iter().map(|phrase| phrase.text.as_str()).collect()
}

#[test]
fn ranks_configured_actions_and_skips_blocked_names() {
    let profile = profile(
        &["they"],
        &["they", "later"],
        &["built", "fix"],
        2,
        16,
        true,
    );
    let text = "Ada Lovelace built props. Later, they fix costumes.";
    let selected = select_phrases(text, &[(0, 12)], 5, Some(&profile), &[], 0, 0);
    assert_eq!(texts(&selected), vec!["built props", "fix costumes"]);
    let limited = select_phrases(text, &[(0, 12)], 1, Some(&profile), &[], 0, 0);
    assert_eq!(texts(&limited), vec!["built props"]);
}

#[test]
fn blocked_spans_are_exclusive_and_rune_based() {
    let profile = profile(&[], &[], &[], 2, 16, true);
    let text = "Ada Lovelace built props.";
    let ungrounded = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
    assert_eq!(texts(&ungrounded), vec!["Lovelace built props"]);
    let name_blocked = select_phrases(text, &[(0, 12)], 5, Some(&profile), &[], 0, 0);
    assert_eq!(texts(&name_blocked), vec!["built props"]);
    let action_blocked = select_phrases(text, &[(13, 24)], 5, Some(&profile), &[], 0, 0);
    assert!(action_blocked.is_empty());
}

#[test]
fn corpus_recurrence_boosts_an_equal_candidate() {
    let profile = profile(&[], &[], &[], 2, 16, true);
    let text = "Gamma delta. Alpha beta.";
    let plain = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
    assert_eq!(texts(&plain), vec!["Gamma delta.", "Alpha beta."]);
    let corpus = vec![text.to_string(), "Alpha beta.".to_string()];
    let boosted = select_phrases(text, &[], 5, Some(&profile), &corpus, 0, 0);
    assert_eq!(texts(&boosted), vec!["Alpha beta.", "Gamma delta."]);
}

#[test]
fn keeps_a_natural_six_word_phrase() {
    let profile = profile(&["o", "da"], &["o", "da"], &[], 2, 6, true);
    let phrase = "O maior arrependimento da minha vida";
    let selected = select_phrases(phrase, &[], 5, Some(&profile), &[], 0, 0);
    assert_eq!(
        selected.first().map(|phrase| phrase.text.as_str()),
        Some(phrase)
    );
}

#[test]
fn hard_caps_fragments_at_sixteen_words() {
    let profile = profile(&[], &[], &[], 2, 24, true);
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";
    let selected = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
    assert!(!selected.is_empty());
    for phrase in &selected {
        assert!(phrase.text.split_whitespace().count() <= 16, "{phrase:?}");
    }
}

#[test]
fn includes_complete_sentence_and_two_sentence_card_without_overlap() {
    let profile = profile(&[], &[], &[], 2, 6, true);
    let text = "We built a garden for the neighbors. The garden brought people together. Everyone found a place.";
    let selected = select_phrases(text, &[], 5, Some(&profile), &[], 0, 0);
    assert_eq!(
        texts(&selected),
        vec![
            "We built a garden for the neighbors. The garden brought people together.",
            "Everyone found a place."
        ]
    );
}

#[test]
fn keeps_a_long_complete_sentence_to_the_terminator() {
    let profile = profile(&[], &[], &[], 2, 6, true);
    let sentence = "Nella provincia di Pavia, la giovane vittima viene ritrovata senza vita nella sua abitazione il tredici agosto del duemilasette in una vicenda che segna la memoria pubblica italiana.";
    let selected = select_phrases(sentence, &[], 5, Some(&profile), &[], 0, 0);
    assert_eq!(
        selected.first().map(|phrase| phrase.text.as_str()),
        Some(sentence)
    );
}

#[test]
fn rejects_function_word_density_above_one_third() {
    let profile = profile(&["of", "my", "in"], &["of", "my", "in"], &[], 6, 6, true);
    let selected = select_phrases(
        "greatest of my in entire life",
        &[],
        5,
        Some(&profile),
        &[],
        0,
        0,
    );
    assert!(selected.is_empty());
}

#[test]
fn proper_name_pairs_ignore_acronym_runs() {
    for (input, want) in [
        ("Ada Lovelace", true),
        ("built props", false),
        ("the Dolly Parton", true),
        ("fix costumes", false),
        ("", false),
        ("USA NATO", false),
        ("AI ML models", false),
        ("the AI act", false),
        ("IBM GmbH", false),
        ("John F Kennedy", true),
        ("New York", true),
    ] {
        assert_eq!(contains_proper_name_pair(input), want, "{input:?}");
    }
}

#[test]
fn words_are_grounded_in_selected_phrases() {
    let profile = profile(&["the"], &["the"], &[], 2, 16, true);
    let selected = select_phrases(
        "The city builds bridges and parks.",
        &[],
        2,
        Some(&profile),
        &[],
        0,
        0,
    );
    let phrases: Vec<String> = selected.into_iter().map(|phrase| phrase.text).collect();
    assert_eq!(
        important_words(&phrases, 3, Some(&profile)),
        vec!["city", "builds", "bridges"]
    );
}
