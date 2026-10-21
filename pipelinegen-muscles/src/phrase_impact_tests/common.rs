use super::super::*;

/// The production phrase stop-word set for a language, read from the single
/// source of truth the Go adapter also injects (config/lexicons/<lang>/).
///
/// Reading the real files keeps these tests honest: the crate holds NO language
/// word list of its own, so a test that needs linguistic data must take it from
/// the same place production does. A language the lexicon does not enumerate
/// resolves to the repository's cross-linguistic `fallback` profile, exactly as
/// the registry does it in Go.
pub(super) fn lexicon_stopwords(language: &str) -> Vec<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/lexicons");
    let dir = if root.join(language).is_dir() {
        root.join(language)
    } else {
        root.join("fallback")
    };
    let mut words = Vec::new();
    for file in ["stopwords.txt", "function_words.txt"] {
        let Ok(raw) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        for line in raw.lines() {
            let word = line.trim().to_lowercase();
            if word.is_empty() || word.starts_with('#') {
                continue;
            }
            words.push(word);
        }
    }
    words
}

/// The injected set in the shape the extraction functions consume.
pub(super) fn lexicon_stopword_set(language: &str) -> HashSet<String> {
    phrase_stopwords(&lexicon_stopwords(language))
}

pub(super) fn vec3(x: f32, y: f32, z: f32) -> Vec<f32> {
    vec![x, y, z]
}

pub(super) fn request(lines: &[&str], embeddings: Vec<Vec<f32>>, options: Options) -> Request {
    Request {
        semantic_bullet_spans: None,
        transcript: lines.join(" "),
        language: "en".into(),
        embeddings,
        timings: Vec::new(),
        options,
        embedding_ms: 0.0,
        lexical_only: false,
        stopwords: lexicon_stopwords("en"),
    }
}

pub(super) fn chapter_test_options() -> ChapterOptions {
    ChapterOptions {
        min_sentences: 2,
        max_sentences: 8,
        min_words: 0,
        complexity_penalty: 2.0,
        ..ChapterOptions::default()
    }
}

pub(super) fn assert_chapter_manifest_contract(manifest: &ChapterManifest, sentences: &[String]) {
    assert_eq!(manifest.schema_version, "chapter_manifest.v1");
    assert!(!manifest.chapters.is_empty());
    assert_eq!(manifest.chapters.first().unwrap().start_sentence, 0);
    assert_eq!(
        manifest.chapters.last().unwrap().end_sentence,
        sentences.len()
    );
    for (index, chapter) in manifest.chapters.iter().enumerate() {
        assert!(!chapter.title.trim().is_empty());
        assert!(chapter.start_sentence < chapter.end_sentence);
        if index > 0 {
            assert_eq!(
                manifest.chapters[index - 1].end_sentence,
                chapter.start_sentence
            );
        }
        for bullet in &chapter.bullets {
            assert!(chapter.start_sentence <= bullet.start_sentence);
            assert!(bullet.start_sentence < bullet.end_sentence);
            assert!(bullet.end_sentence <= chapter.end_sentence);
            let source = sentences[bullet.start_sentence..bullet.end_sentence]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(bullet.text, source);
        }
        match (chapter.start_ms, chapter.end_ms) {
            (Some(start), Some(end)) => assert!(start >= 0 && end > start),
            (None, None) => {}
            _ => panic!("chapter timestamps must be both present or both absent"),
        }
    }
}

pub(super) fn orthogonal_embeddings(count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|index| {
            let mut vector = vec![0.0; count];
            vector[index] = 1.0;
            vector
        })
        .collect()
}

pub(super) fn labeled_test_sentences(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            format!(
                "Sentence {index} explains that the company reported operational details across its regional businesses during the current financial quarter ending in June."
            )
        })
        .collect()
}

pub(super) fn lexical_scene_request(transcript: &str) -> Request {
    Request {
        semantic_bullet_spans: None,
        transcript: transcript.into(),
        language: "en".into(),
        embeddings: Vec::new(),
        timings: Vec::new(),
        options: Options::default(),
        embedding_ms: 0.0,
        lexical_only: true,
        stopwords: lexicon_stopwords("en"),
    }
}

pub(super) fn lexical_scene_request_in(transcript: &str, language: &str) -> Request {
    Request {
        semantic_bullet_spans: None,
        transcript: transcript.into(),
        language: language.into(),
        embeddings: Vec::new(),
        timings: Vec::new(),
        options: Options::default(),
        embedding_ms: 0.0,
        lexical_only: true,
        stopwords: lexicon_stopwords(language),
    }
}

pub(super) fn generated_chapter_sentences(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            format!(
                "record{index} value{} detail{} marker{}.",
                index % 5,
                (index / 3) % 7,
                (index / 2) % 11
            )
        })
        .collect()
}

pub(super) fn lexical_chapter_request(sentences: &[String]) -> Request {
    Request {
        semantic_bullet_spans: None,
        transcript: sentences.join(" "),
        language: "und".into(),
        embeddings: Vec::new(),
        timings: Vec::new(),
        options: Options::default(),
        embedding_ms: 0.0,
        lexical_only: true,
        stopwords: lexicon_stopwords("und"),
    }
}
