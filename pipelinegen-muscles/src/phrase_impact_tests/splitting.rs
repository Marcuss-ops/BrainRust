use super::super::*;
use super::common::*;

#[test]
fn sentence_split_plain_and_difficult_cases_preserve_surface() {
    let source =
        "Apple launched a new phone. Sales increased by 20%. Investors reacted positively.";
    let parts = split_sentences(source, "en");
    assert_eq!(
        parts
            .iter()
            .map(|part| part.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Apple launched a new phone.",
            "Sales increased by 20%.",
            "Investors reacted positively."
        ]
    );
    assert!(parts
        .iter()
        .all(|part| source[part.start_byte..part.end_byte] == part.text));
    let difficult =
        "Dr. Smith joined Apple Inc. in 2025. Revenue reached $2.5 billion. U.S. sales increased.";
    assert_eq!(
        split_sentences(difficult, "en")
            .iter()
            .map(|part| part.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Dr. Smith joined Apple Inc. in 2025.",
            "Revenue reached $2.5 billion.",
            "U.S. sales increased."
        ]
    );
}

#[test]
fn localized_splitting_and_unpunctuated_fallback() {
    for (language, text, expected) in [
        ("it", "La dott. Rossi è arrivata. Poi ha parlato.", 2),
        ("es", "La Dra. García llegó. Después habló.", 2),
        ("pt", "O Dr. Silva chegou. Depois falou.", 2),
        ("fr", "M. Dupont est arrivé. Ensuite il a parlé.", 2),
        ("de", "Dr. Müller kam an. Danach sprach er.", 2),
        ("ja", "これは文です。次の文です！", 2),
    ] {
        assert_eq!(
            split_sentences(text, language).len(),
            expected,
            "{language}: {text}"
        );
    }
    assert_eq!(
        split_sentences("Revenue grew.Next the board met.", "en").len(),
        2
    );
    let source = (0..95)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let chunks = split_sentences(&source, "en");
    assert!(
        chunks.len() >= 2
            && chunks
                .iter()
                .all(|part| word_count(&part.text) <= MAX_SEGMENT_WORDS)
    );
    assert_eq!(
        chunks
            .iter()
            .flat_map(|part| part.text.split_whitespace())
            .collect::<Vec<_>>(),
        source.split_whitespace().collect::<Vec<_>>()
    );
}

#[test]
fn synthetic_transcript_preserves_long_sentences_abbreviations_and_utf8_offsets() {
    let transcript = include_str!("../../fixtures/elon_musk_synthetic_transcript_it.txt");
    let segments = split_sentences(transcript, "it");
    let segment_texts: Vec<&str> = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    for expected in [
        "Il secondo punto chiave fu l'annuncio sintetico secondo cui Tesla Energy stava valutando, sempre in questo scenario inventato, un investimento da €750 milioni in un impianto di accumulo vicino a Berlino con una decisione finale prevista per il 2 aprile 2026.",
        "Il settimo punto ad alta importanza dichiarò che Tesla non avrebbe annunciato licenziamenti durante l'evento sintetico e che, al contrario, il piano operativo ipotizzava 1,500 nuove assunzioni tecniche tra Texas e Germania entro diciotto mesi.",
        "La quarta frase pesante stabilì che SpaceX avrebbe programmato, nello scenario sintetico, una finestra di prova di Starship il 21 giugno 2026 alle 9:15 a.m., con un massimo di tre tentativi tecnici distribuiti nella stessa settimana.",
        "J. R. Collins, personaggio fittizio introdotto apposta per il test, chiese se l'uso delle iniziali puntate potesse confondere un segmentatore di frasi o un modello che non gestisce correttamente i nomi abbreviati.",
        "Alle 10:45 p.m. la sessione terminò e i partecipanti lasciarono la sala, mentre il sistema di registrazione salvò la trascrizione completa per i test successivi di segmentazione, entity extraction, ranking e summarization.",
        "Un ulteriore blocco di controllo descrive una riunione successiva del 18 marzo 2026 a Milan, Italy, nella quale Luca Bianchi confronta i nove passaggi principali del benchmark e segnala che alcune frasi molto specifiche possono risultare semanticamente lontane dal centroide pur essendo essenziali per il video.",
    ] {
        assert!(segment_texts.contains(&expected), "sentence was split or altered: {expected}");
    }
    for segment in &segments {
        assert!(segment.end_byte <= transcript.len());
        assert!(transcript.is_char_boundary(segment.start_byte));
        assert!(transcript.is_char_boundary(segment.end_byte));
        assert_eq!(
            &transcript[segment.start_byte..segment.end_byte],
            segment.text
        );
    }
    assert!(segments
        .iter()
        .all(|segment| word_count(&segment.text) <= MAX_SEGMENT_WORDS));
}

#[test]
fn shared_splitter_handles_spanish_abbreviations_and_utf8_offsets() {
    let transcript = "La Dra. García llegó. Después habló de café.";
    let spans = split_sentence_spans(transcript, "es-MX", None);
    assert_eq!(spans.len(), 2);
    for span in spans {
        assert!(transcript.is_char_boundary(span.start_byte));
        assert!(transcript.is_char_boundary(span.end_byte));
        assert_eq!(&transcript[span.start_byte..span.end_byte], span.text);
    }
}

#[test]
fn whisper_like_unpunctuated_transcript_keeps_numbers_and_all_surface_text() {
    let transcript = "well you know the company announced 20,000 layoffs uh investors reacted quickly the board will review the plan next year";
    let segments = split_sentences(transcript, "en");
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].text, transcript);
    let result = run(Request {
        transcript: transcript.into(),
        language: "en".into(),
        embeddings: vec![vec3(1., 0., 0.)],
        timings: Vec::new(),
        options: Options {
            min_heavy: 1,
            max_heavy: 1,
            summary_length: SummaryLength::Short,
            bullet_count: 1,
            ..Options::default()
        },
        embedding_ms: 0.0,
        lexical_only: false,
        stopwords: lexicon_stopwords("en"),
    })
    .unwrap();
    assert_eq!(result.heavy_sentences[0].text, transcript);
    assert!(result.summary.contains("20,000 layoffs"));
    assert!(result.bullet_points[0].text.contains("20,000 layoffs"));
}
