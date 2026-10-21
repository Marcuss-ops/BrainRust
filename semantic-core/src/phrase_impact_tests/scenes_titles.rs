use super::super::*;
use super::common::*;

#[test]
fn topic_with_digits_validates_when_it_matches_scene_text() {
    let text = "The G7 summit gathered leaders to discuss industrial cooperation and trade.";
    let (title, status, source) =
        select_scene_title(text, Some("G7 summit"), &lexicon_stopword_set("en"));
    assert_eq!(status, "resolved");
    assert_eq!(source, "topic_validated");
    assert!(title.unwrap().to_lowercase().contains("g7"));
}

#[test]
fn scene_highlights_keep_distinct_offsets_for_repeated_text() {
    let scene = "The factory closed. Workers left the town.";
    let transcript = format!("{scene} {scene}");
    let first_end = scene.len();
    let inputs = vec![
        SceneInput {
            scene_id: "s1".into(),
            text: scene.into(),
            topic: None,
            start_byte: Some(0),
            end_byte: Some(first_end),
        },
        SceneInput {
            scene_id: "s2".into(),
            text: scene.into(),
            topic: None,
            start_byte: Some(first_end + 1),
            end_byte: Some(transcript.len()),
        },
    ];
    let doc = run_with_scene_inputs(
        lexical_scene_request(&transcript),
        &inputs,
        &ChapterOptions::default(),
    )
    .unwrap();
    assert_eq!(doc.scene_highlights.len(), 2);
    assert!(doc.scene_highlights.iter().all(|h| h.globally_indexed));
    // Repeated text without slices degrades to local-only, never first-match globals.
    let local = vec![
        SceneInput {
            scene_id: "s1".into(),
            text: scene.into(),
            topic: None,
            start_byte: None,
            end_byte: None,
        },
        SceneInput {
            scene_id: "s2".into(),
            text: scene.into(),
            topic: None,
            start_byte: None,
            end_byte: None,
        },
    ];
    let doc = run_with_scene_inputs(
        lexical_scene_request(&transcript),
        &local,
        &ChapterOptions::default(),
    )
    .unwrap();
    assert!(doc.scene_highlights.iter().all(|h| !h.globally_indexed));
}

#[test]
fn duplicate_scene_identity_is_a_contract_error() {
    let inputs = vec![
        SceneInput {
            scene_id: "s1".into(),
            text: "Alpha beta gamma delta. Epsilon zeta eta theta.".into(),
            topic: None,
            start_byte: None,
            end_byte: None,
        },
        SceneInput {
            scene_id: "s1".into(),
            text: "Other text here with enough words to analyze.".into(),
            topic: None,
            start_byte: None,
            end_byte: None,
        },
    ];
    let err = run_with_scene_inputs(
        lexical_scene_request("Alpha beta gamma delta. Other text here."),
        &inputs,
        &ChapterOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("duplicate"));
}

#[test]
fn utf8_offsets_validate_on_accented_text() {
    let scene = "La città è bellissima. L’industria cresce piano.";
    let transcript = scene.to_string();
    let inputs = vec![SceneInput {
        scene_id: "s1".into(),
        text: scene.into(),
        topic: None,
        start_byte: Some(0),
        end_byte: Some(transcript.len()),
    }];
    let doc = run_with_scene_inputs(
        lexical_scene_request(&transcript),
        &inputs,
        &ChapterOptions::default(),
    )
    .unwrap();
    assert_eq!(doc.scene_highlights.len(), 1);
    for h in &doc.scene_highlights {
        for sp in &h.highlights {
            assert!(transcript.is_char_boundary(sp.start_byte));
            assert!(transcript.is_char_boundary(sp.end_byte));
        }
    }
}

#[test]
fn incoherent_topic_is_rejected_for_a_reliable_candidate_or_nothing() {
    // R08: topic "Quantum computing" shares no token with a bakery scene.
    let text = "The bakery opened at dawn. Fresh bread filled the street with aroma. Customers queued around the block.";
    let (title, status, source) =
        select_scene_title(text, Some("Quantum computing"), &lexicon_stopword_set("en"));
    assert_ne!(
        source, "topic_validated",
        "incoherent topic must not validate"
    );
    assert_eq!(status, "resolved");
    let title = title.unwrap();
    assert!(!title.to_lowercase().contains("quantum"));
    // R08 coherent control: matching topic validates.
    let (title, status, source) = select_scene_title(
        text,
        Some("neighborhood bakery"),
        &lexicon_stopword_set("en"),
    );
    assert_eq!(
        (status.as_str(), source.as_str()),
        ("resolved", "topic_validated")
    );
    assert!(title.unwrap().to_lowercase().contains("bakery"));
}

#[test]
fn extracted_scene_title_never_ends_on_a_function_word_in_any_language() {
    // Regression: 2-4 gram candidates were built from every non-stopword token,
    // so a title could terminate mid-phrase on a closed-class word the crate's
    // own English list had omitted. Real production output looked like
    // "Behemoths Did More Than", "Actual Value Generated By" and "Across
    // Germany Remain Among" — fragments, not titles.
    //
    // The contract pinned here is narrow and checkable: an EXTRACTED title
    // (never a topic-validated one) must begin and end on a content word, where
    // "content" is defined by the stop set the CALLER injected. It is asserted
    // for Italian too — a language the crate contains no knowledge of — using
    // the repository's own lexicon data, so a regression back to any in-code
    // English list cannot satisfy it.
    let cases: [(&str, [&str; 3]); 2] = [
        (
            "en",
            [
                "The simple arithmetic became undeniable, the escalating cost of keeping these facilities running exceeded the actual value generated by their output.",
                "On one front, exports destined for Asia have seen their momentum wane as local manufacturers scale up production.",
                // A content run longer than the token budget: the title may be
                // trimmed, but never onto a stop word and never mid-run in a way
                // that keeps a function word as its last token.
                "These essential inputs are procured by competing international facilities at significantly lower costs, placing German producers in an immediate structural disadvantage.",
            ],
        ),
        (
            "it",
            [
                "La semplice aritmetica divenne innegabile, il costo crescente di mantenere questi impianti superava il valore effettivo prodotto dalla loro attività.",
                "Su un fronte, le esportazioni destinate all'Asia hanno visto il loro slancio affievolirsi mentre i produttori locali aumentano la capacità.",
                "Questi input essenziali vengono acquistati da impianti internazionali concorrenti a costi decisamente inferiori, ponendo i produttori tedeschi in un immediato svantaggio strutturale.",
            ],
        ),
    ];
    for (language, sentences) in cases {
        let stopwords = lexicon_stopword_set(language);
        assert!(
            !stopwords.is_empty(),
            "{language}: the injected set must not be empty"
        );
        for text in sentences {
            let (title, status, source) = select_scene_title(text, None, &stopwords);
            assert_eq!(status, "resolved", "a coherent scene must resolve a title");
            assert_ne!(source, "topic_validated", "no topic was supplied here");
            let title = title.expect("resolved implies a title");
            let tokens: Vec<String> = title.split_whitespace().map(str::to_lowercase).collect();
            assert!(!tokens.is_empty(), "title must not be empty: {title:?}");
            assert!(
                tokens.len() <= MAX_TITLE_TOKENS,
                "{language}: title {title:?} exceeds the {MAX_TITLE_TOKENS}-token overlay budget"
            );
            // The strong, structural contract: NO title token comes from the
            // injected set, so a title can neither start nor end on a function
            // word nor have one buried mid-phrase.
            for token in &tokens {
                assert!(
                    !stopwords.contains(token),
                    "{language}: title {title:?} contains injected stop word {token:?}"
                );
            }
        }
    }
}

#[test]
fn over_long_content_run_prefers_the_tail_of_the_phrase() {
    // A content run longer than the title budget cannot be quoted whole, so it
    // must be trimmed. The TAIL is what survives: the head of the same run is
    // penalised, so a title never starts mid-phrase.
    //
    // The fixture is deliberately symmetric — six tokens, each occurring exactly
    // once — so the head-5 and the tail-5 spans have identical length, identical
    // token frequency, identical document frequency and identical rarity. Their
    // scores were therefore exactly equal and the winner fell through to the
    // alphabetical tie-break: this text used to be titled "Alpha Beta Gamma
    // Delta Epsilon", i.e. by accident of the alphabet.
    let text = "alpha beta gamma delta epsilon zeta";
    let (title, status, source) = select_scene_title(text, None, &lexicon_stopword_set("en"));
    assert_eq!(status, "resolved", "a coherent scene must resolve a title");
    assert_ne!(source, "topic_validated", "no topic was supplied here");
    assert_eq!(
        title.unwrap(),
        "Beta Gamma Delta Epsilon Zeta",
        "the tail of an over-long content run must win, not the head"
    );
}

#[test]
fn content_run_of_exactly_the_title_budget_is_kept_whole() {
    // Real production scene text, and a trap for anyone reading the tokenizer
    // too loosely: this function splits on EVERY non-alphanumeric character, so
    // the ", " after "spikes" ends the content run. The run is therefore
    // exactly MAX_TITLE_TOKENS long, nothing is trimmed, and the published
    // title "Natural Gas Experienced Sharp Spikes" is the WHOLE phrase rather
    // than the head of a longer one.
    let text = "When the price of natural gas experienced sharp spikes, multiple facilities were compelled to reduce operational shifts.";
    let (title, status, source) = select_scene_title(text, None, &lexicon_stopword_set("en"));
    assert_eq!(status, "resolved", "a coherent scene must resolve a title");
    assert_ne!(source, "topic_validated", "no topic was supplied here");
    assert_eq!(
        title.unwrap(),
        "Natural Gas Experienced Sharp Spikes",
        "a run that fits the budget must be quoted whole"
    );
}

#[test]
fn extracted_title_follows_the_injected_stop_set() {
    // The crate owns no English (or any other) word list: what survives in a
    // title is decided solely by the set the caller injects. Changing the set
    // must therefore change the extracted title.
    let text = "The simple arithmetic became undeniable, the escalating cost of keeping these facilities running exceeded the actual value generated by their output.";
    let wide = lexicon_stopword_set("en");
    let narrow: HashSet<String> = ["the", "of", "these", "their", "a", "an"]
        .iter()
        .map(|word| (*word).to_string())
        .collect();
    let (wide_title, _, _) = select_scene_title(text, None, &wide);
    let (narrow_title, _, _) = select_scene_title(text, None, &narrow);
    let wide_title = wide_title.expect("the wide set still resolves a title");
    let narrow_title = narrow_title.expect("the narrow set still resolves a title");
    assert!(
        !wide_title
            .split_whitespace()
            .any(|word| wide.contains(&word.to_lowercase())),
        "no title token may come from the injected set: {wide_title:?}"
    );
    assert_ne!(
        wide_title, narrow_title,
        "the injected stop set, not a built-in list, must decide the extracted title"
    );
}
