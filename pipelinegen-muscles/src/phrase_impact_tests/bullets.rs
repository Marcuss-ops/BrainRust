use super::super::*;
use super::common::*;

#[test]
fn globally_indexed_bullets_match_canonical_sentence_ranges() {
    // R09: every bullet must equal the concatenation of the canonical
    // global sentences in [sentence_start, sentence_end).
    let s1 = "Germany lost one hundred thousand industrial jobs in a single year.";
    let s2 = "Factory orders declined for the sixth consecutive quarter.";
    let s3 = "Unions demand shorter hours without any loss of pay.";
    let transcript = format!("{s1} {s2} {s3}");
    let inputs = vec![SceneInput {
        scene_id: "scene-03".into(),
        text: transcript.clone(),
        topic: Some("German industrial employment".into()),
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
    let h = &doc.scene_highlights[0];
    assert!(h.globally_indexed);
    let global = split_sentences(&transcript, "en");
    assert!(!h.bullets.is_empty());
    for b in &h.bullets {
        assert!(b.sentence_start < b.sentence_end);
        assert!(b.sentence_end <= global.len());
        let canon = global[b.sentence_start..b.sentence_end]
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(b.text, canon, "bullet must be verbatim canonical sentences");
    }
}

#[test]
fn scene_identity_is_unique_and_order_preserving() {
    // R10: ids unique, none lost, input order preserved.
    let t = "Alpha beta gamma delta epsilon zeta. Eta theta iota kappa lambda mu.";
    let transcript = format!("{t} {t} {t}");
    let starts = [0, t.len() + 1, 2 * t.len() + 2];
    let inputs = vec![
        SceneInput {
            scene_id: "s-a".into(),
            text: t.into(),
            topic: None,
            start_byte: Some(starts[0]),
            end_byte: Some(starts[0] + t.len()),
        },
        SceneInput {
            scene_id: "s-b".into(),
            text: t.into(),
            topic: None,
            start_byte: Some(starts[1]),
            end_byte: Some(starts[1] + t.len()),
        },
        SceneInput {
            scene_id: "s-c".into(),
            text: t.into(),
            topic: None,
            start_byte: Some(starts[2]),
            end_byte: Some(starts[2] + t.len()),
        },
    ];
    let doc = run_with_scene_inputs(
        lexical_scene_request(&transcript),
        &inputs,
        &ChapterOptions::default(),
    )
    .unwrap();
    let ids: Vec<&str> = doc
        .scene_highlights
        .iter()
        .map(|h| h.scene_id.as_str())
        .collect();
    assert_eq!(ids, vec!["s-a", "s-b", "s-c"]);
}

#[test]
fn two_concepts_in_one_scene_yield_distinct_bullets() {
    // R11: one scene, two concepts -> distinct bullets, no forced chapters.
    let text = "Germany lost one hundred thousand industrial jobs in a single year, and factory orders declined sharply across every major region. Economists warn that the downturn may persist through next winter unless exports recover soon. Meanwhile the national football team won the championship after a dramatic final match watched by millions. Celebrations filled the streets of Berlin until the early morning hours.";
    let doc = run_with_scene_inputs(
        lexical_scene_request(text),
        &[SceneInput {
            scene_id: "s1".into(),
            text: text.into(),
            topic: None,
            start_byte: Some(0),
            end_byte: Some(text.len()),
        }],
        &ChapterOptions::default(),
    )
    .unwrap();
    assert_eq!(
        doc.scene_highlights.len(),
        1,
        "one scene yields one highlight, not forced chapters"
    );
    let h = &doc.scene_highlights[0];
    assert!(
        h.bullets.len() >= 2,
        "two concepts deserve two bullets, got {}",
        h.bullets.len()
    );
    let joined = h
        .bullets
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
        .to_lowercase();
    let industry =
        joined.contains("jobs") || joined.contains("factory") || joined.contains("exports");
    let sport =
        joined.contains("football") || joined.contains("championship") || joined.contains("berlin");
    assert!(
        industry && sport,
        "bullets must span both concepts: {joined}"
    );
    for b in &h.bullets {
        assert!(text.contains(&b.text));
    }
}

#[test]
fn short_scene_invents_no_bullets_and_missing_title_is_unavailable() {
    let inputs = vec![SceneInput {
        scene_id: "s1".into(),
        text: "Ciao.".into(),
        topic: None,
        start_byte: None,
        end_byte: None,
    }];
    let doc = run_with_scene_inputs(
        lexical_scene_request("Ciao. Questa è una scena più lunga con abbastanza parole per l'analisi completa del sistema."),
        &inputs, &ChapterOptions::default(),
    ).unwrap();
    assert_eq!(doc.scene_highlights.len(), 1);
    assert!(
        doc.scene_highlights[0].bullets.is_empty(),
        "no invented bullets for a one-word scene"
    );
}

#[test]
fn bullet_substance_and_closure_helpers_reject_fragments_and_mid_sentence_tails() {
    assert!(!bullet_closes_sentence("con la tutela dei diritti di"));
    assert!(bullet_closes_sentence(
        "con la tutela dei diritti di chi è chiamato in giudizio."
    ));
    assert!(bullet_closes_sentence(
        "Il quadro processuale resta aperto.\""
    ));
    assert!(!bullet_has_substance("impronta 33."));
    assert!(!bullet_has_substance("del caso."));
    assert!(bullet_has_substance(
        "Il fascicolo resta aperto e segue il suo corso."
    ));
    // CJK has no spaces: the character floor keeps whole sentences eligible.
    assert!(bullet_has_substance(
        "これは完全な文であり、十分な長さがあります。"
    ));
    assert_eq!(
        bullet_identity("  Il 13 agosto 2007,\n Chiara Poggi fu uccisa a Garlasco. "),
        "il 13 agosto 2007, chiara poggi fu uccisa a garlasco."
    );
}

#[test]
fn scene_bullets_never_publish_a_truncated_mid_sentence_tail() {
    // Regression from a live 2k-word run (scene 14): a sentence longer than
    // MAX_SEGMENT_WORDS was chunked for analysis and the head chunk ended up
    // published as a bullet, stopping mid-sentence ("...dei diritti di").
    let long = format!(
        "È imprescindibile comprendere che {} e che il quadro processuale resti comunque completo.",
        (0..45)
            .map(|i| format!("elemento{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let filler =
        "Il fascicolo resta aperto e segue il suo corso ordinario davanti al giudice competente.";
    let text = format!("{long} {filler}");
    let inputs = vec![SceneInput {
        scene_id: "s1".into(),
        text: text.clone(),
        topic: None,
        start_byte: Some(0),
        end_byte: Some(text.len()),
    }];
    let doc = run_with_scene_inputs(
        lexical_scene_request_in(&text, "it"),
        &inputs,
        &ChapterOptions::default(),
    )
    .unwrap();
    let h = &doc.scene_highlights[0];
    assert!(
        !h.bullets.is_empty(),
        "the scene must still produce bullets"
    );
    assert!(
        h.bullets.iter().any(|bullet| bullet.text == long),
        "the long sentence must be quoted whole, never its 48-word head: {:?}",
        h.bullets
            .iter()
            .map(|bullet| bullet.text.as_str())
            .collect::<Vec<_>>()
    );
    for bullet in &h.bullets {
        assert!(
            bullet_closes_sentence(&bullet.text),
            "a bullet may not end mid-sentence: {:?}",
            bullet.text
        );
        assert!(text.contains(&bullet.text));
    }
}

#[test]
fn short_fragment_sentences_are_not_published_as_bullets() {
    // Regression from a live run: the model closed a paragraph with the
    // two-word tail "impronta 33." and the extractor published it as a bullet.
    let text = "Il fascicolo raccoglie gli atti principali del procedimento e li ordina con metodo rigoroso, distinguendo i fatti accertati dalle ipotesi investigative ancora aperte. impronta 33. La difesa contesta la lettura di quel reperto e chiede un esame tecnico completo davanti al giudice, mentre l'accusa insiste sulla qualità degli accertamenti eseguiti. Gli inquirenti hanno depositato nuova documentazione e il tribunale ha fissato le prossime udienze con attenzione alle garanzie processuali.";
    let inputs = vec![SceneInput {
        scene_id: "s1".into(),
        text: text.into(),
        topic: None,
        start_byte: None,
        end_byte: None,
    }];
    let doc = run_with_scene_inputs(
        lexical_scene_request_in(text, "it"),
        &inputs,
        &ChapterOptions::default(),
    )
    .unwrap();
    let h = &doc.scene_highlights[0];
    assert!(!h.bullets.is_empty());
    assert!(
        h.bullets
            .iter()
            .all(|bullet| bullet_has_substance(&bullet.text)),
        "fragments must never be bullets: {:?}",
        h.bullets
            .iter()
            .map(|bullet| bullet.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(!h
        .bullets
        .iter()
        .any(|bullet| bullet.text.trim() == "impronta 33."));
}

#[test]
fn global_bullets_never_publish_fragments_or_continuation_chunks() {
    // The anti-truncation and anti-fragment rule is a document-wide
    // extraction contract, not a scene-only one: the global bullet list may
    // not publish a two-word tail or the head chunk of a sentence longer than
    // MAX_SEGMENT_WORDS either.
    let long = format!(
        "La relazione annuale descrive {} e conclude che il quadro resta aperto.",
        (0..45)
            .map(|i| format!("indicatore{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let text = format!("{long} impronta 33. Il fascicolo raccoglie gli atti principali del procedimento e li ordina con metodo rigoroso. La difesa chiede un esame completo dei reperti davanti al giudice competente.");
    let mut request = lexical_scene_request_in(&text, "it");
    request.options.bullet_count = 10;
    let result = run(request).unwrap();
    let sources = split_sentences(&text, "it");
    assert!(
        !result.bullet_points.is_empty(),
        "complete sentences must remain eligible bullets"
    );
    for bullet in &result.bullet_points {
        assert!(
            sources.iter().any(|segment| segment.text == bullet.text),
            "bullet must be an exact source sentence: {:?}",
            bullet.text
        );
        assert!(
            bullet_closes_sentence(&bullet.text),
            "global bullet ended mid-sentence: {:?}",
            bullet.text
        );
        assert!(
            bullet_has_substance(&bullet.text),
            "fragment published as a global bullet: {:?}",
            bullet.text
        );
        assert!(
            !bullet.text.starts_with("La relazione annuale"),
            "a chunk of the long sentence leaked into the global bullets: {:?}",
            bullet.text
        );
    }
}

#[test]
fn chapter_bullets_never_publish_fragments_or_continuation_chunks() {
    // Two ADJACENT sentences longer than MAX_SEGMENT_WORDS are the shape that
    // breaks a chapter extractor without boundary completion: the head chunk of
    // the second one is a chapter-bullet candidate, so the published span both
    // starts mid-sentence and is cut short. One long sentence alone is not
    // enough — contextual expansion happens to widen it.
    let long_a = format!(
        "È imprescindibile comprendere che {} e che il quadro processuale resti comunque completo.",
        (0..45)
            .map(|i| format!("elemento{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let long_b = format!(
        "La relazione annuale descrive {} e conclude che il giudice deciderà sugli atti depositati.",
        (0..45)
            .map(|i| format!("reperto{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut sentences: Vec<String> =
        vec![long_a.clone(), long_b.clone(), "impronta 33.".to_string()];
    for index in 0..30 {
        sentences.push(format!(
            "Il capitolo numero {index} raccoglie i fatti principali del procedimento e li ordina con metodo rigoroso davanti al giudice."
        ));
    }
    let transcript = sentences.join(" ");
    let result = run(Request {
        transcript: transcript.clone(),
        language: "it".into(),
        embeddings: Vec::new(),
        timings: Vec::new(),
        options: Options::default(),
        embedding_ms: 0.0,
        lexical_only: true,
        stopwords: lexicon_stopwords("it"),
    })
    .unwrap();
    let split = split_sentences(&transcript, "it");
    let texts: Vec<&str> = split.iter().map(|part| part.text.as_str()).collect();
    let mut inspected = 0usize;
    for chapter in &result.chapter_manifest.chapters {
        for bullet in &chapter.bullets {
            inspected += 1;
            assert!(bullet.start_sentence < bullet.end_sentence);
            assert!(
                bullet.start_sentence == 0
                    || bullet_closes_sentence(texts[bullet.start_sentence - 1]),
                "chapter bullet started mid-sentence: {:?}",
                bullet.text
            );
            assert!(
                bullet_closes_sentence(&bullet.text) || bullet.end_sentence == texts.len(),
                "chapter bullet ended mid-sentence: {:?}",
                bullet.text
            );
            assert!(
                bullet_has_substance(&bullet.text),
                "fragment published as a chapter bullet: {:?}",
                bullet.text
            );
            for long in [&long_a, &long_b] {
                assert!(
                    !bullet.text.trim_start().starts_with(&long[..40]) || bullet.text == *long,
                    "a continuation chunk of a long sentence leaked into the chapter bullets: {:?}",
                    bullet.text
                );
            }
            assert!(!bullet.text.trim().is_empty());
        }
    }
    assert!(
        inspected > 0,
        "the fixture must produce chapter bullets to exercise the contract"
    );
}

#[test]
fn a_sentence_repeated_across_scenes_is_published_as_a_bullet_once() {
    // The payload mandates one opening sentence; the model repeated it at the
    // start of nearly every scene, so the document showed the same line 13
    // times. Verified scenes must publish it once.
    let shared = "Il 13 agosto 2007, Chiara Poggi fu uccisa nella sua abitazione a Garlasco.";
    let body_a = "Le prime indagini analizzano i reperti con metodo rigoroso e separano i fatti accertati dalle ipotesi investigative.";
    let body_b = "La Procura di Pavia ha focalizzato l'attenzione su Andrea Sempio, che respinge le accuse e resta indagato nel procedimento.";
    let text_a = format!("{shared} {body_a}");
    let text_b = format!("{shared} {body_b}");
    let transcript = format!("{text_a} {text_b}");
    let inputs = vec![
        SceneInput {
            scene_id: "s1".into(),
            text: text_a.clone(),
            topic: None,
            start_byte: Some(0),
            end_byte: Some(text_a.len()),
        },
        SceneInput {
            scene_id: "s2".into(),
            text: text_b.clone(),
            topic: None,
            start_byte: Some(text_a.len() + 1),
            end_byte: Some(text_a.len() + 1 + text_b.len()),
        },
    ];
    let doc = run_with_scene_inputs(
        lexical_scene_request_in(&transcript, "it"),
        &inputs,
        &ChapterOptions::default(),
    )
    .unwrap();
    let published: Vec<&str> = doc
        .scene_highlights
        .iter()
        .flat_map(|highlight| highlight.bullets.iter().map(|bullet| bullet.text.as_str()))
        .collect();
    let repeats = published
        .iter()
        .filter(|text| text.starts_with("Il 13 agosto 2007"))
        .count();
    assert!(
        repeats <= 1,
        "the mandated opening sentence was republished: {published:?}"
    );
    for highlight in &doc.scene_highlights {
        assert!(
            !highlight.bullets.is_empty(),
            "scene {} lost its bullets",
            highlight.scene_id
        );
    }
}
