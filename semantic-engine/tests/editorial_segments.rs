//! Bullets, rankings, segment windows and fallback behavior.

mod common;

use common::*;
use semantic_engine::analyze_script;

#[test]
fn one_script_batch_can_return_phrase_impact_editorial_products() {
    let text = "Il gruppo prepara il progetto e controlla ogni passaggio. Gli operatori registrano i risultati e verificano tutte le procedure. La squadra completa il lavoro e pubblica il rapporto finale.";
    let mut req = request(vec![scene("editorial", text)]);
    req.editorial = Some(
        serde_json::from_value(serde_json::json!({
            "request": {
                "transcript": text,
                "language": "it",
                "embeddings": [],
                "lexical_only": true,
                "stopwords": []
            },
            "scenes": [{"scene_id":"editorial", "text":text, "start_byte":0, "end_byte":text.len()}],
        }))
        .expect("editorial request follows the shared Phrase Impact contract"),
    );

    let manifest = analyze_script(&req).expect("combined analysis must succeed");
    let editorial = manifest
        .editorial
        .expect("editorial result belongs to same manifest");
    assert!(!editorial.summary.is_empty());
    assert_eq!(
        editorial.chapter_manifest.schema_version,
        "chapter_manifest.v1"
    );
    assert!(editorial
        .scene_highlights
        .iter()
        .any(|highlight| highlight.scene_id == "editorial"));
    let semantic_bullet_texts: std::collections::BTreeSet<_> = manifest.scenes[0]
        .bullets
        .iter()
        .map(|bullet| bullet.text.as_str())
        .collect();
    assert!(editorial
        .bullet_points
        .iter()
        .all(|bullet| semantic_bullet_texts.contains(bullet.text.as_str())));
}

#[test]
fn ten_thousand_word_script_keeps_every_canonical_sentence_candidate() {
    let mut text = String::with_capacity(100_000);
    for index in 0..2_500 {
        if index > 0 {
            text.push(' ');
        }
        text.push_str("La storia continua oggi.");
    }
    assert_eq!(text.split_whitespace().count(), 10_000);
    let value = manifest_value(vec![scene("copione-lungo", &text)]);
    let scene = &value["scenes"][0];
    let phrases = scene["short_phrases"].as_array().unwrap();
    assert_eq!(phrases.len(), 2_500, "the full script must be represented");
    for (index, phrase) in phrases.iter().enumerate() {
        assert_eq!(phrase["sentence_index"], index);
        let start = phrase["start_byte"].as_u64().unwrap() as usize;
        let end = phrase["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), phrase["evidence"].as_str());
    }
}

#[test]
fn explicit_top_ten_reports_incomplete_lists_without_filling_them() {
    let mut incomplete = String::from("Top 10 momenti importanti:\n");
    for rank in (5..=10).rev() {
        incomplete.push_str(&format!("{rank}. Momento {rank} descritto nel copione.\n"));
    }
    let value = manifest_value(vec![scene("top-10-incompleta", &incomplete)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["group_type"], "ranking");
    assert_eq!(groups[0]["target_count"], 10);
    assert_eq!(groups[0]["complete"], false);
    assert_eq!(groups[0]["members"].as_array().unwrap().len(), 6);

    let mut complete = String::from("Top 10 momenti importanti:\n");
    for rank in (1..=10).rev() {
        complete.push_str(&format!("{rank}. Momento {rank} descritto nel copione.\n"));
    }
    let value = manifest_value(vec![scene("top-10-completa", &complete)]);
    let group = &value["scenes"][0]["semantic_groups"][0];
    assert_eq!(group["complete"], true);
    assert_eq!(group["members"].as_array().unwrap().len(), 10);
}

#[test]
fn inline_numbered_top_ten_is_one_complete_grounded_ranking() {
    let items: Vec<String> = (1..=10)
        .map(|rank| format!("{rank}. Momento numero {rank} del racconto."))
        .collect();
    let text = format!("Top 10 momenti importanti: {}", items.join(" "));
    let value = manifest_value(vec![scene("top-10-inline", &text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let ranking = groups
        .iter()
        .find(|group| group["group_type"] == "ranking")
        .unwrap_or_else(|| panic!("inline ranking expected: {groups:?}"));
    assert_eq!(ranking["target_count"], 10);
    assert_eq!(ranking["complete"], true);
    let members = ranking["members"].as_array().unwrap();
    assert_eq!(members.len(), 10);
    for (index, member) in members.iter().enumerate() {
        let rank = index + 1;
        assert_eq!(member["rank"], rank);
        let start = member["start_byte"].as_u64().unwrap() as usize;
        let end = member["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(text.get(start..end), member["evidence"].as_str());
        assert!(
            member["text"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{rank}.")),
            "each item keeps its own rank marker: {}",
            member["text"]
        );
        if rank < 10 {
            assert!(
                !member["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!(" {}. ", rank + 1)),
                "an item must not absorb the next one: {}",
                member["text"]
            );
        }
    }
}

#[test]
fn inline_numbered_list_without_a_declared_top_ten_stays_an_enumeration() {
    let text = "Le fasi sono: 1. Progettazione. 2. Costruzione. 3. Collaudo.";
    let value = manifest_value(vec![scene("elenco-inline", text)]);
    let groups = value["scenes"][0]["semantic_groups"].as_array().unwrap();
    let enumeration = groups
        .iter()
        .find(|group| group["group_type"] == "enumeration")
        .unwrap_or_else(|| panic!("enumeration expected: {groups:?}"));
    assert_eq!(enumeration["members"].as_array().unwrap().len(), 3);
    assert!(enumeration.get("complete").is_none());
}

#[test]
fn bullet_candidates_carry_the_shared_semantic_salience() {
    let text =
        "Torino. Lo Stadio Aurora di Torino ha 10.000 posti e appartiene alla famiglia Rossi.";
    let value = manifest_value(vec![scene("bullets-score", text)]);
    let bullets = value["scenes"][0]["bullets"].as_array().unwrap();
    assert_eq!(bullets.len(), 2, "both entity sentences are candidates");
    assert!(bullets.iter().all(|bullet| {
        let score = bullet["score"].as_f64().unwrap();
        (0.0..=1.0).contains(&score) && bullet["provenance"] == "semantic_core_bullets"
    }));
    let sparse = bullets[0]["score"].as_f64().unwrap();
    let rich = bullets[1]["score"].as_f64().unwrap();
    assert!(
        rich > sparse,
        "the evidence-rich sentence must outrank the fragment: {bullets:?}"
    );
}

#[test]
fn filler_sentences_are_never_bullets() {
    // A bullet needs a grounded signal: discourse noise stays out of the
    // editorial bullets while the named sentence remains the only candidate.
    let text = "Alberto Stasi viveva a Pavia. Non è così. Lui. Lei.";
    let value = manifest_value(vec![scene("bullets-rumore", text)]);
    let bullets = value["scenes"][0]["bullets"].as_array().unwrap();
    assert_eq!(bullets.len(), 1, "only the grounded sentence: {bullets:?}");
    assert_eq!(bullets[0]["text"], "Alberto Stasi viveva a Pavia.");
}

#[test]
fn important_segments_stay_on_linguistic_boundaries() {
    // Every qualifying passage is 2–5 sentences, grounded, retained (no
    // candidate cap) and cut at canonical linguistic boundaries.
    let text = "Chiara Poggi viveva a Pavia. Il 13 agosto 2007 il caso cambiò. \
                La Procura di Pavia aprì un'indagine. Alberto Stasi fu rinviato \
                a giudizio. Non è così. Il processo iniziò nel 2009.";
    let value = manifest_value(vec![scene("segmenti", text)]);
    let scene_value = &value["scenes"][0];
    let segments = scene_value["important_segments"].as_array().unwrap();
    assert!(
        segments.len() > 3,
        "no candidate cap: overlapping passages stay available to the planner: {segments:?}"
    );

    let spans = semantic_core::split_sentences(text, "it", None);
    let mut ends: Vec<(usize, usize)> = Vec::new();
    for segment in segments {
        let start = segment["sentence_index_start"].as_u64().unwrap() as usize;
        let end = segment["sentence_index_end"].as_u64().unwrap() as usize;
        assert!(
            end >= start && end - start >= 1 && end - start < 5,
            "2..=5 sentences expected: {segment:?}"
        );
        // Linguistic boundaries: the span starts at the first sentence's start
        // and ends at the last sentence's end.
        assert_eq!(
            segment["start_byte"].as_u64().unwrap() as usize,
            spans[start].start_byte
        );
        assert_eq!(
            segment["end_byte"].as_u64().unwrap() as usize,
            spans[end].end_byte
        );
        // Verbatim source slice (NO EVIDENCE → NO CANDIDATE).
        let start_byte = segment["start_byte"].as_u64().unwrap() as usize;
        let end_byte = segment["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(
            segment["evidence"].as_str().unwrap(),
            &text[start_byte..end_byte]
        );
        assert_eq!(segment["kind"], "important_segment");
        ends.push((start_byte, end_byte));
    }
    // Every passage is unique and emitted in source order.
    let unique: std::collections::BTreeSet<_> = ends.iter().copied().collect();
    assert_eq!(unique.len(), ends.len(), "duplicate passages: {ends:?}");
    assert!(ends.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    // Deterministic replay: the same request yields the same segments.
    let again = manifest_value(vec![scene("segmenti", text)]);
    assert_eq!(
        again["scenes"][0]["important_segments"],
        scene_value["important_segments"]
    );
}

#[test]
fn a_scene_without_signals_keeps_no_segments() {
    // A successful analysis with no grounded signals stays empty instead of
    // inventing passages.
    let value = manifest_value(vec![scene(
        "vuota-di-segnali",
        "Non è così. Non era così. Non sarà così.",
    )]);
    let scene = &value["scenes"][0];
    assert_eq!(scene["status"], "valid");
    assert!(
        scene["important_segments"].as_array().unwrap().is_empty(),
        "no signals, no segments: {}",
        scene["important_segments"]
    );
}
