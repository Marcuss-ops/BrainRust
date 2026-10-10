//! Unit tests for phrase_impact (extracted from phrase_impact.rs to keep the
//! production file reviewable; compiled only under cfg(test)).

use super::*;


/// The production phrase stop-word set for a language, read from the single
/// source of truth the Go adapter also injects (config/lexicons/<lang>/).
///
/// Reading the real files keeps these tests honest: the crate holds NO language
/// word list of its own, so a test that needs linguistic data must take it from
/// the same place production does. A language the lexicon does not enumerate
/// resolves to the repository's cross-linguistic `fallback` profile, exactly as
/// the registry does it in Go.
fn lexicon_stopwords(language: &str) -> Vec<String> {
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
fn lexicon_stopword_set(language: &str) -> HashSet<String> {
    phrase_stopwords(&lexicon_stopwords(language))
}

fn vec3(x: f32, y: f32, z: f32) -> Vec<f32> {
    vec![x, y, z]
}
fn request(lines: &[&str], embeddings: Vec<Vec<f32>>, options: Options) -> Request {
    Request {
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

fn chapter_test_options() -> ChapterOptions {
    ChapterOptions {
        min_sentences: 2,
        max_sentences: 8,
        min_words: 0,
        complexity_penalty: 2.0,
        ..ChapterOptions::default()
    }
}

fn assert_chapter_manifest_contract(manifest: &ChapterManifest, sentences: &[String]) {
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

fn orthogonal_embeddings(count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|index| {
            let mut vector = vec![0.0; count];
            vector[index] = 1.0;
            vector
        })
        .collect()
}

fn labeled_test_sentences(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            format!(
                "Sentence {index} explains that the company reported operational details across its regional businesses during the current financial quarter ending in June."
            )
        })
        .collect()
}

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
        .all(|part| &source[part.start_byte..part.end_byte] == part.text));
    let difficult = "Dr. Smith joined Apple Inc. in 2025. Revenue reached $2.5 billion. U.S. sales increased.";
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

fn generated_chapter_sentences(count: usize) -> Vec<String> {
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

fn lexical_chapter_request(sentences: &[String]) -> Request {
    Request {
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

#[test]
fn scene_metadata_is_optional_and_partition_always_covers_generated_input() {
    let sentences = generated_chapter_sentences(17);
    let baseline = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &chapter_test_options(),
    )
    .unwrap();
    let scenes = sentences
        .chunks(4)
        .map(|chunk| chunk.join(" "))
        .collect::<Vec<_>>();
    let topics = scenes
        .iter()
        .enumerate()
        .map(|(index, _)| format!("label{index}"))
        .collect::<Vec<_>>();
    let with_context = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &scenes,
        &topics,
        &chapter_test_options(),
    )
    .unwrap();
    let invalid_context = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &["unrelated input".into()],
        &["unrelated label".into()],
        &chapter_test_options(),
    )
    .unwrap();

    assert_chapter_manifest_contract(&baseline.chapter_manifest, &sentences);
    assert_chapter_manifest_contract(&with_context.chapter_manifest, &sentences);
    assert_chapter_manifest_contract(&invalid_context.chapter_manifest, &sentences);
    assert_eq!(baseline.summary, invalid_context.summary);
    assert_eq!(baseline.bullet_points, invalid_context.bullet_points);
    assert_eq!(baseline.heavy_sentences, invalid_context.heavy_sentences);
    assert_eq!(baseline.chapter_manifest, invalid_context.chapter_manifest);
}

#[test]
fn complexity_profile_metamorphism_does_not_increase_partition_count() {
    let sentences = generated_chapter_sentences(29);
    let request = lexical_chapter_request(&sentences);
    let analyze = |complexity_penalty| {
        run_with_chapter_options(
            request.clone(),
            &[],
            &[],
            &ChapterOptions {
                min_sentences: 2,
                max_sentences: 10,
                min_words: 0,
                complexity_penalty,
                ..chapter_test_options()
            },
        )
        .unwrap()
    };
    let low_penalty = analyze(0.25);
    let high_penalty = analyze(3.0);
    assert_chapter_manifest_contract(&low_penalty.chapter_manifest, &sentences);
    assert_chapter_manifest_contract(&high_penalty.chapter_manifest, &sentences);
    assert!(
        high_penalty.chapter_manifest.chapters.len()
            <= low_penalty.chapter_manifest.chapters.len()
    );
}

#[test]
fn repeated_input_is_deterministic_and_preserves_full_coverage() {
    let block = generated_chapter_sentences(7);
    let mut repeated = block.clone();
    repeated.extend(block.iter().cloned());
    let request = lexical_chapter_request(&repeated);
    let first =
        run_with_chapter_options(request.clone(), &[], &[], &chapter_test_options()).unwrap();
    let second = run_with_chapter_options(request, &[], &[], &chapter_test_options()).unwrap();
    assert_chapter_manifest_contract(&first.chapter_manifest, &repeated);
    assert_eq!(first.chapter_manifest, second.chapter_manifest);
    assert_eq!(first.summary, second.summary);
    assert_eq!(first.bullet_points, second.bullet_points);
    assert_eq!(first.heavy_sentences, second.heavy_sentences);
}

#[test]
fn appended_input_remains_fully_represented_in_partition() {
    let original = generated_chapter_sentences(13);
    let mut extended = original.clone();
    extended.extend(
        generated_chapter_sentences(5)
            .into_iter()
            .map(|sentence| sentence.replace("record", "extension")),
    );
    let analyze = |sentences: &[String]| {
        run_with_chapter_options(
            lexical_chapter_request(sentences),
            &[],
            &[],
            &chapter_test_options(),
        )
        .unwrap()
    };
    let baseline = analyze(&original);
    let grown = analyze(&extended);
    assert_chapter_manifest_contract(&baseline.chapter_manifest, &original);
    assert_chapter_manifest_contract(&grown.chapter_manifest, &extended);
}

#[test]
fn chapter_analysis_scales_to_large_generated_transcript_without_losing_coverage() {
    let sentences = generated_chapter_sentences(1000);
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &ChapterOptions {
            max_sentences: 100,
            min_sentences: 10,
            min_words: 0,
            ..chapter_test_options()
        },
    )
    .unwrap();
    assert_chapter_manifest_contract(&result.chapter_manifest, &sentences);
}

fn behavioral_chapter_options() -> ChapterOptions {
    ChapterOptions {
        min_sentences: 2,
        max_sentences: 6,
        min_words: 0,
        context_sentences: 2,
        complexity_penalty: 1.0,
        bullet_count: 3,
        ..ChapterOptions::default()
    }
}

fn analyze_chapter_text(text: &str, language: &str) -> Result<ResultDocument, String> {
    let mut request = lexical_chapter_request(
        &split_sentences(text, language)
            .into_iter()
            .map(|part| part.text)
            .collect::<Vec<_>>(),
    );
    request.language = language.into();
    run_with_chapter_options(request, &[], &[], &behavioral_chapter_options())
}

fn boundary_indices(manifest: &ChapterManifest) -> Vec<usize> {
    manifest
        .chapters
        .iter()
        .skip(1)
        .map(|chapter| chapter.start_sentence)
        .collect()
}

fn assert_manifest_is_contiguous(manifest: &ChapterManifest, sentence_count: usize) {
    assert_eq!(manifest.schema_version, "chapter_manifest.v1");
    assert!(!manifest.chapters.is_empty());
    assert_eq!(manifest.chapters[0].start_sentence, 0);
    assert_eq!(
        manifest.chapters.last().unwrap().end_sentence,
        sentence_count
    );
    for pair in manifest.chapters.windows(2) {
        assert_eq!(pair[0].end_sentence, pair[1].start_sentence);
    }
}

#[test]
fn chapter_behavior_detects_three_unrelated_subject_blocks_without_fixed_titles() {
    let groups = [
        [
            "GPU servers process video frames using graphics memory.",
            "CPU transfers add latency to GPU workloads.",
            "Efficient buffers improve rendering throughput.",
        ],
        [
            "Home construction requires building permits.",
            "Zoning rules constrain architectural plans.",
            "Concrete and timber materials affect construction costs.",
        ],
        [
            "Video platforms publish media to online audiences.",
            "Application APIs automate content uploads.",
            "Scheduled releases simplify channel management.",
        ],
    ];
    let sentences = groups
        .into_iter()
        .flatten()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    let manifest = &result.chapter_manifest;
    assert_manifest_is_contiguous(manifest, sentences.len());
    assert!(
        manifest.chapters.len() >= 2 && manifest.chapters.len() <= 4,
        "unrelated blocks should be separated without prescribing a title: {:?}",
        manifest.chapters
    );
    let boundaries = boundary_indices(manifest);
    assert!(
        boundaries.iter().any(|&b| (2..=4).contains(&b)),
        "expected a boundary near the first thematic transition, got {boundaries:?}"
    );
    assert!(
        boundaries.iter().any(|&b| (5..=7).contains(&b)),
        "expected a boundary near the second thematic transition, got {boundaries:?}"
    );
    for chapter in &manifest.chapters {
        let source = sentences[chapter.start_sentence..chapter.end_sentence]
            .join(" ")
            .to_lowercase();
        let lowered_title = chapter.title.to_lowercase();
        let title_terms = lowered_title
            .split_whitespace()
            .filter(|term| term.chars().any(char::is_alphabetic))
            .collect::<Vec<_>>();
        assert!(!title_terms.is_empty());
        assert!(
            title_terms.iter().any(|term| source.contains(term)),
            "title must be grounded in its chapter source"
        );
    }
}

#[test]
fn chapter_behavior_separates_different_topics_with_overlapping_vocabulary() {
    let sentences = vec![
        "Banks manage cash flow for small businesses.".to_string(),
        "Current accounts track money moving through the bank.".to_string(),
        "Lenders review credit before approving commercial loans.".to_string(),
        "River banks guide water flow during seasonal floods.".to_string(),
        "Strong currents move sediment along the river channel.".to_string(),
        "Flood barriers protect nearby towns from rising water.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    let boundaries = boundary_indices(&result.chapter_manifest);
    assert!(
        boundaries
            .iter()
            .any(|&boundary| (2..=4).contains(&boundary)),
        "the lexical overlap must not hide the shift in intended subject: {boundaries:?}"
    );
}

#[test]
fn chapter_behavior_does_not_split_repeated_subject_only_because_scene_changes() {
    let sentences = vec![
        "Video platforms distribute creator content online.".to_string(),
        "Streaming services deliver video to audiences.".to_string(),
        "Creators publish clips through media channels.".to_string(),
        "Online video services organize audience subscriptions.".to_string(),
    ];
    let text = sentences.join(" ");
    let request = lexical_chapter_request(&sentences);
    let plain =
        run_with_chapter_options(request.clone(), &[], &[], &behavioral_chapter_options())
            .unwrap();
    let scenes = sentences
        .iter()
        .map(|sentence| sentence.clone())
        .collect::<Vec<_>>();
    let scene_ids = (0..sentences.len())
        .map(|i| format!("scene-{i}"))
        .collect::<Vec<_>>();
    let contextual =
        run_with_chapter_options(request, &scenes, &scene_ids, &behavioral_chapter_options())
            .unwrap();
    assert_eq!(
        plain.chapter_manifest.chapters.len(),
        contextual.chapter_manifest.chapters.len(),
        "scene boundaries alone must not multiply chapters for a stable topic"
    );
    assert_eq!(
        plain
            .chapter_manifest
            .chapters
            .iter()
            .map(|chapter| (chapter.start_sentence, chapter.end_sentence))
            .collect::<Vec<_>>(),
        contextual
            .chapter_manifest
            .chapters
            .iter()
            .map(|chapter| (chapter.start_sentence, chapter.end_sentence))
            .collect::<Vec<_>>()
    );
    assert_eq!(text, sentences.join(" "));
}

#[test]
fn chapter_behavior_finds_theme_changes_without_scene_metadata() {
    let text = "GPU memory transfers affect rendering speed. Graphics buffers reduce frame-processing overhead. Video kernels improve throughput. Building permits regulate residential construction. Zoning plans shape house design. Timber and concrete affect construction cost.";
    let result = analyze_chapter_text(text, "en").unwrap();
    let boundaries = boundary_indices(&result.chapter_manifest);
    assert!(
        boundaries
            .iter()
            .any(|&boundary| (2..=4).contains(&boundary)),
        "theme change without scene metadata should have a plausible boundary: {boundaries:?}"
    );
}

#[test]
fn chapter_behavior_keeps_a_returning_theme_in_separate_chronological_blocks() {
    let sentences = vec![
        "Video platforms distribute clips to online audiences.".to_string(),
        "Creators schedule media releases across channels.".to_string(),
        "Building permits regulate residential construction.".to_string(),
        "Zoning rules constrain architectural projects.".to_string(),
        "Hydropower turbines convert river flow into electricity.".to_string(),
        "Reservoir operators manage water levels during drought.".to_string(),
        "Streaming platforms organize video subscriptions.".to_string(),
        "Online channels recommend new creator clips.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    let boundaries = boundary_indices(&result.chapter_manifest);
    assert!(
        boundaries.iter().any(|&b| (1..=3).contains(&b)),
        "first thematic transition missing: {boundaries:?}; chapters={:?}",
        result.chapter_manifest.chapters
    );
    assert!(
        boundaries.iter().any(|&b| (5..=7).contains(&b)),
        "returning theme transition missing: {boundaries:?}; chapters={:?}",
        result.chapter_manifest.chapters
    );
    assert!(
        result
            .chapter_manifest
            .chapters
            .windows(2)
            .all(|pair| pair[0].end_sentence == pair[1].start_sentence),
        "returning themes remain separate chronological spans"
    );
}

#[test]
fn chapter_behavior_keeps_paraphrased_same_subject_in_one_thematic_run() {
    let sentences = vec![
        "Solar panels transform daylight into usable electricity.".to_string(),
        "Photovoltaic cells convert sunlight into electrical current.".to_string(),
        "Renewable modules generate clean power during bright hours.".to_string(),
        "Sun-powered arrays supply energy to nearby homes.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    assert!(
        result.chapter_manifest.chapters.len() <= 2,
        "lexical variation within a coherent subject should not fragment it: {:?}",
        result.chapter_manifest.chapters
    );
}

#[test]
fn chapter_behavior_handles_gradual_transition_without_excessive_fragmentation() {
    let sentences = vec![
        "Solar arrays generate electricity from sunlight.".to_string(),
        "Renewable power flows from photovoltaic panels.".to_string(),
        "Battery systems store electricity produced by solar arrays.".to_string(),
        "Grid operators balance stored energy with local demand.".to_string(),
        "Electric utilities coordinate renewable supply and distribution.".to_string(),
        "Transmission networks deliver power to regional consumers.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    assert!((1..=3).contains(&result.chapter_manifest.chapters.len()), "a gradual topical transition should not cause sentence-by-sentence fragmentation: {:?}", result.chapter_manifest.chapters);
    assert!(boundary_indices(&result.chapter_manifest)
        .iter()
        .all(|&boundary| boundary > 0 && boundary < sentences.len()));
}

#[test]
fn chapter_behavior_partitions_long_subtopics_with_bounded_balanced_spans() {
    let mut sentences = Vec::new();
    for topic in [
        "coastal ecology",
        "urban transit",
        "archive preservation",
        "renewable power",
    ] {
        for index in 0..12 {
            sentences.push(format!("Research on {topic} examines observation {} and evidence {} across regional studies.", index % 4, (index / 3) % 5));
        }
    }
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &ChapterOptions {
            min_sentences: 3,
            max_sentences: 12,
            min_words: 0,
            complexity_penalty: 0.5,
            ..behavioral_chapter_options()
        },
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    assert!(
        result.chapter_manifest.chapters.len() > 1,
        "long input must partition rather than collapse into one chapter"
    );
    for chapter in &result.chapter_manifest.chapters {
        let length = chapter.end_sentence - chapter.start_sentence;
        assert!(
            length <= 12,
            "chapter span exceeded configured bound: {length}"
        );
        assert!(
            length >= 3 || chapter.end_sentence == sentences.len(),
            "only a terminal short tail may fall below the minimum"
        );
    }
}

#[test]
fn chapter_behavior_uses_distinct_source_titles_for_related_subtopics() {
    let sentences = vec![
        "Solar panels generate electricity during daylight hours.".to_string(),
        "Photovoltaic cells convert sunlight into clean power.".to_string(),
        "Rooftop arrays supply renewable energy to homes.".to_string(),
        "Battery storage preserves electricity after sunset.".to_string(),
        "Grid operators balance stored power with evening demand.".to_string(),
        "Energy networks coordinate charging and regional distribution.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &ChapterOptions {
            complexity_penalty: 0.5,
            ..behavioral_chapter_options()
        },
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    if result.chapter_manifest.chapters.len() >= 2 {
        let first = result.chapter_manifest.chapters[0].title.to_lowercase();
        let last = result
            .chapter_manifest
            .chapters
            .last()
            .unwrap()
            .title
            .to_lowercase();
        assert_ne!(first, last, "separate spans on one macro-topic should derive their titles from distinct local content");
    }
}

#[test]
fn chapter_behavior_avoids_duplicate_bullets_for_repeated_source_sentences() {
    let repeated = "Researchers measured coastal water temperatures across the region.";
    let sentences = vec![
        repeated.to_string(),
        repeated.to_string(),
        "Marine teams compared salinity readings from several shoreline stations.".to_string(),
        "The report describes seasonal changes in local ocean conditions.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    let mut unique = std::collections::HashSet::new();
    for bullet in result
        .chapter_manifest
        .chapters
        .iter()
        .flat_map(|chapter| &chapter.bullets)
    {
        assert!(
            unique.insert(bullet.text.as_str()),
            "duplicate extractive bullet selected: {}",
            bullet.text
        );
    }
}

#[test]
fn chapter_behavior_keeps_major_boundaries_stable_under_paraphrase() {
    let original = vec![
        "Solar panels convert sunlight into electrical energy.".to_string(),
        "Photovoltaic cells deliver renewable power to the grid.".to_string(),
        "Battery storage balances supply after sunset.".to_string(),
        "Rail operators coordinate passenger timetables between stations.".to_string(),
        "Regional trains connect local services across the network.".to_string(),
        "Transit planners adjust routes to serve growing communities.".to_string(),
    ];
    let paraphrased = vec![
        "Sunlight becomes electricity through rooftop solar arrays.".to_string(),
        "Renewable current from panels enters the public network.".to_string(),
        "Stored power supports demand when daylight ends.".to_string(),
        "Train companies organize departure schedules at terminals.".to_string(),
        "Intercity services link neighborhood lines throughout the region.".to_string(),
        "Transport designers revise paths for expanding towns.".to_string(),
    ];
    let run = |lines: &[String]| {
        run_with_chapter_options(
            lexical_chapter_request(lines),
            &[],
            &[],
            &behavioral_chapter_options(),
        )
        .unwrap()
    };
    let first = run(&original);
    let second = run(&paraphrased);
    let a = boundary_indices(&first.chapter_manifest);
    let b = boundary_indices(&second.chapter_manifest);
    assert!(
        a.iter()
            .any(|boundary| b.iter().any(|other| boundary.abs_diff(*other) <= 1)),
        "major topic boundary should remain near its position under paraphrase: {a:?} vs {b:?}"
    );
}

#[test]
fn chapter_behavior_extracts_grounded_conceptual_titles_from_numeric_chapters() {
    let sentences = vec![
        "In 2021 the system processed 480 frames per second.".to_string(),
        "The GPU used 24 gigabytes of memory for the workload.".to_string(),
        "Latency fell by 17 percent after the buffer redesign.".to_string(),
        "Throughput reached 960 operations during the final test.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    let chapter = &result.chapter_manifest.chapters[0];
    assert!(!chapter.title.is_empty());
    assert!(
        chapter.title.chars().any(char::is_alphabetic),
        "a numeric passage should receive a conceptual rather than numeric-only title"
    );
    assert!(
        chapter
            .title
            .chars()
            .all(|character| !character.is_ascii_digit()),
        "title should not merely repeat a statistic"
    );
    let source = sentences.join(" ").to_lowercase();
    assert!(
        chapter
            .title
            .to_lowercase()
            .split_whitespace()
            .any(|term| source.contains(term)),
        "title terms should be grounded in source"
    );
}

#[test]
fn chapter_behavior_bullets_preserve_negation_and_contiguous_context() {
    let sentences = vec![
        "The committee did not approve the proposal.".to_string(),
        "It instead requested a revised budget.".to_string(),
        "The revised plan does not include new borrowing.".to_string(),
        "Members approved the timetable after the changes.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    let mut saw_negation = false;
    for chapter in &result.chapter_manifest.chapters {
        for bullet in &chapter.bullets {
            let expected = sentences[bullet.start_sentence..bullet.end_sentence].join(" ");
            assert_eq!(bullet.text, expected);
            if bullet.text.contains("not") {
                saw_negation = true;
            }
        }
    }
    assert!(
        saw_negation,
        "extractive bullets must retain at least one selected negative statement"
    );
}

#[test]
fn chapter_behavior_can_keep_dependent_sentences_in_one_contiguous_bullet() {
    let sentences = vec![
        "Coastal gauges recorded a rapid increase in the water level.".to_string(),
        "This rise continued through the afternoon along the shoreline.".to_string(),
        "Observers compared the readings with measurements from nearby stations.".to_string(),
        "The report describes seasonal changes in local ocean conditions.".to_string(),
    ];
    let result = run_with_chapter_options(
        lexical_chapter_request(&sentences),
        &[],
        &[],
        &behavioral_chapter_options(),
    )
    .unwrap();
    assert_manifest_is_contiguous(&result.chapter_manifest, sentences.len());
    assert!(
        result.chapter_manifest.chapters.iter().any(|chapter| {
            chapter.bullets.iter().any(|bullet| {
                bullet.end_sentence - bullet.start_sentence >= 2
                    && bullet.text
                        == sentences[bullet.start_sentence..bullet.end_sentence].join(" ")
            })
        }),
        "when context is selected, bullets must be contiguous source groups: {:?}",
        result.chapter_manifest.chapters
    );
}

#[test]
fn chapter_behavior_is_stable_under_proper_name_substitution() {
    let first = vec![
        "Mira studied coastal currents and ocean salinity.".to_string(),
        "Researchers measured marine temperatures near the shore.".to_string(),
        "Tomas mapped river sediment after seasonal flooding.".to_string(),
        "Hydrologists tracked freshwater levels across the basin.".to_string(),
        "Mira compared ocean samples with earlier measurements.".to_string(),
        "Marine scientists published their coastal observations.".to_string(),
    ];
    let second = first
        .iter()
        .map(|line| line.replace("Mira", "Asha").replace("Tomas", "Rui"))
        .collect::<Vec<_>>();
    let run = |lines: &[String]| {
        run_with_chapter_options(
            lexical_chapter_request(lines),
            &[],
            &[],
            &behavioral_chapter_options(),
        )
        .unwrap()
    };
    let a = run(&first);
    let b = run(&second);
    assert_eq!(
        a.chapter_manifest
            .chapters
            .iter()
            .map(|chapter| (chapter.start_sentence, chapter.end_sentence))
            .collect::<Vec<_>>(),
        b.chapter_manifest
            .chapters
            .iter()
            .map(|chapter| (chapter.start_sentence, chapter.end_sentence))
            .collect::<Vec<_>>()
    );
}

#[test]
fn chapter_behavior_has_cross_language_coverage_and_repeatability() {
    let cases = [
        (
            "it",
            vec![
                "I server video elaborano fotogrammi usando memoria grafica.",
                "I trasferimenti CPU GPU aumentano la latenza dei processi.",
                "Buffer efficienti migliorano il rendering dei video.",
                "I permessi edilizi regolano la costruzione delle abitazioni.",
                "Le norme urbanistiche definiscono i vincoli dei progetti.",
                "Legno e cemento cambiano i costi dei cantieri.",
            ],
        ),
        (
            "en",
            vec![
                "Video servers process frames using graphics memory.",
                "CPU transfers increase workload latency.",
                "Efficient buffers improve rendering throughput.",
                "Building permits regulate home construction.",
                "Zoning rules define project constraints.",
                "Timber and concrete change construction costs.",
            ],
        ),
        (
            "pt",
            vec![
                "Servidores de vídeo processam quadros usando memória gráfica.",
                "Transferências da CPU aumentam a latência do trabalho.",
                "Buffers eficientes melhoram o desempenho de renderização.",
                "Licenças de construção regulam obras residenciais.",
                "Regras de zoneamento definem limites do projeto.",
                "Madeira e concreto alteram custos da obra.",
            ],
        ),
    ];
    let mut counts = Vec::new();
    for (language, lines) in cases {
        let sentences = lines.into_iter().map(str::to_string).collect::<Vec<_>>();
        let request = || {
            let mut request = lexical_chapter_request(&sentences);
            request.language = language.into();
            request
        };
        let first =
            run_with_chapter_options(request(), &[], &[], &behavioral_chapter_options())
                .unwrap();
        let second =
            run_with_chapter_options(request(), &[], &[], &behavioral_chapter_options())
                .unwrap();
        assert_eq!(first.chapter_manifest, second.chapter_manifest);
        assert_manifest_is_contiguous(&first.chapter_manifest, sentences.len());
        counts.push(first.chapter_manifest.chapters.len());
    }
    assert!(
        counts.iter().all(|count| (1..=4).contains(count)),
        "cross-language partition should remain bounded and nonempty: {counts:?}"
    );
    let minimum = counts.iter().min().copied().unwrap_or_default();
    let maximum = counts.iter().max().copied().unwrap_or_default();
    assert!(
        maximum - minimum <= 1,
        "translations of one thematic structure should yield comparable chapter counts: {counts:?}"
    );
}

#[test]
fn chapter_behavior_is_deterministic_across_one_hundred_runs() {
    let sentences = vec![
        "Solar panels convert sunlight into electrical power.".to_string(),
        "Inverters synchronize photovoltaic current with the grid.".to_string(),
        "Battery storage balances renewable energy after sunset.".to_string(),
        "Rail operators coordinate passenger routes between stations.".to_string(),
        "Timetables connect local trains with regional services.".to_string(),
    ];
    let request = lexical_chapter_request(&sentences);
    let first =
        run_with_chapter_options(request.clone(), &[], &[], &behavioral_chapter_options())
            .unwrap();
    let json = serde_json::to_string(&first.chapter_manifest).unwrap();
    for _ in 1..100 {
        let repeated =
            run_with_chapter_options(request.clone(), &[], &[], &behavioral_chapter_options())
                .unwrap();
        assert_eq!(
            serde_json::to_string(&repeated.chapter_manifest).unwrap(),
            json
        );
    }
}

#[test]
fn chapter_behavior_lexical_fallback_is_explicit_and_deterministic() {
    let sentences = vec![
        "Solar panels generate renewable electricity.".to_string(),
        "Inverters connect photovoltaic systems to the grid.".to_string(),
        "Building permits regulate residential construction.".to_string(),
    ];
    let request = lexical_chapter_request(&sentences);
    let first =
        run_with_chapter_options(request.clone(), &[], &[], &behavioral_chapter_options())
            .unwrap();
    let second =
        run_with_chapter_options(request, &[], &[], &behavioral_chapter_options()).unwrap();
    assert_eq!(first.chapter_manifest, second.chapter_manifest);
    assert!(first
        .ranked
        .iter()
        .all(|sentence| sentence.importance.is_finite()));
    assert!(!first.ranked.is_empty());
}

#[test]
fn chapter_behavior_returns_errors_for_empty_and_malformed_inputs_and_accepts_short_text() {
    let mut empty = lexical_chapter_request(&[]);
    empty.transcript.clear();
    assert!(run_with_chapter_options(empty, &[], &[], &behavioral_chapter_options()).is_err());
    let mut malformed = lexical_chapter_request(&["A complete sentence.".into()]);
    malformed.embeddings = vec![vec![f32::NAN]];
    malformed.lexical_only = false;
    assert!(
        run_with_chapter_options(malformed, &[], &[], &behavioral_chapter_options()).is_err()
    );
    let short = analyze_chapter_text("Rain falls.", "en").unwrap();
    assert_eq!(short.chapter_manifest.chapters.len(), 1);
    assert_eq!(short.chapter_manifest.chapters[0].start_sentence, 0);
    assert_eq!(short.chapter_manifest.chapters[0].end_sentence, 1);
}

#[test]
fn synthetic_transcript_preserves_long_sentences_abbreviations_and_utf8_offsets() {
    let transcript = include_str!("../fixtures/elon_musk_synthetic_transcript_it.txt");
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

fn is_extractive(summary: &str, segments: &[Segment]) -> bool {
    let mut remaining = summary;
    let mut matched = 0;
    for segment in segments {
        let Some(tail) = remaining.strip_prefix(&segment.text) else {
            continue;
        };
        remaining = tail.strip_prefix(' ').unwrap_or(tail);
        matched += 1;
        if remaining.is_empty() {
            return matched > 0;
        }
    }
    false
}

#[test]
fn synthetic_transcript_summarization_keeps_negation_and_source_spans() {
    #[derive(Deserialize)]
    struct GroundTruth {
        sentences: Vec<LabeledSentence>,
    }
    #[derive(Deserialize)]
    struct LabeledSentence {
        id: String,
        text: String,
        label: String,
        #[serde(default)]
        must_preserve_negation: Option<String>,
    }

    let transcript = include_str!("../fixtures/elon_musk_synthetic_transcript_it.txt");
    let truth: GroundTruth = serde_json::from_str(include_str!(
        "../fixtures/elon_musk_synthetic_ground_truth.json"
    ))
    .unwrap();
    let result = run(Request {
        transcript: transcript.to_string(),
        language: "it".into(),
        embeddings: Vec::new(),        timings: Vec::new(),
        options: Options {
            summary_length: SummaryLength::Short,
            bullet_count: 10,
            min_heavy: 15,
            max_heavy: 15,
            top_fraction: Some(0.125),
        },
        embedding_ms: 0.0,
        lexical_only: true,
        stopwords: lexicon_stopwords("it"),
    })
    .unwrap();
    assert!(truth
        .sentences
        .iter()
        .all(|sentence| matches!(sentence.label.as_str(), "heavy" | "trap")));
    let segments = split_sentences(transcript, "it");
    let segment_texts: std::collections::HashSet<&str> = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    for sentence in &truth.sentences {
        assert!(
            segment_texts.contains(sentence.text.as_str()),
            "fixture sentence {} not found in source split",
            sentence.id
        );
    }
    let heavy_count = truth
        .sentences
        .iter()
        .filter(|sentence| sentence.label == "heavy")
        .count();
    assert_eq!(heavy_count, 9);
    let id_by_text: std::collections::HashMap<&str, &str> = truth
        .sentences
        .iter()
        .map(|sentence| (sentence.text.as_str(), sentence.id.as_str()))
        .collect();
    let heavy_ids: std::collections::HashSet<&str> = truth
        .sentences
        .iter()
        .filter(|sentence| sentence.label == "heavy")
        .map(|sentence| sentence.id.as_str())
        .collect();
    let mut ranking_metrics = Vec::new();
    for cutoff in [5, 10] {
        let top = result.ranked.iter().take(cutoff).collect::<Vec<_>>();
        let hits = top
            .iter()
            .filter(|sentence| {
                id_by_text
                    .get(sentence.text.as_str())
                    .is_some_and(|id| heavy_ids.contains(id))
            })
            .count();
        let precision = hits as f64 / top.len() as f64;
        assert!(precision.is_finite() && (0.0..=1.0).contains(&precision));
        ranking_metrics.push((cutoff, precision));
    }
    assert_eq!(ranking_metrics.len(), 2);
    let negation = truth
        .sentences
        .iter()
        .find_map(|sentence| sentence.must_preserve_negation.as_deref())
        .unwrap();
    let summary_contains_claim = result.summary.to_lowercase().contains("licenziamenti");
    let summary_preserves_negation = result
        .summary
        .to_lowercase()
        .contains(&negation.to_lowercase());
    if summary_contains_claim {
        assert!(
            summary_preserves_negation,
            "summary mentioned layoffs but dropped negation: {}",
            result.summary
        );
    }
    assert!(
        is_extractive(&result.summary, &segments),
        "summary must only contain complete source sentences: {}",
        result.summary
    );
    for bullet in &result.bullet_points {
        assert!(
            segments.iter().any(|segment| segment.text == bullet.text),
            "bullet must be a source sentence: {}",
            bullet.text
        );
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

#[test]
fn end_to_end_outputs_preserve_sentences_in_supported_languages() {
    let cases = [
        (
            "en",
            [
                "The company announced layoffs.",
                "Revenue increased this year.",
            ],
        ),
        (
            "it",
            [
                "L'azienda ha annunciato licenziamenti.",
                "I ricavi sono aumentati quest'anno.",
            ],
        ),
        (
            "es",
            [
                "La empresa anunció despidos.",
                "Los ingresos aumentaron este año.",
            ],
        ),
        (
            "pt",
            [
                "A empresa anunciou demissões.",
                "A receita aumentou este ano.",
            ],
        ),
        (
            "fr",
            [
                "L'entreprise a annoncé des licenciements.",
                "Les revenus ont augmenté cette année.",
            ],
        ),
        (
            "de",
            [
                "Das Unternehmen kündigte Entlassungen an.",
                "Der Umsatz stieg in diesem Jahr.",
            ],
        ),
    ];
    for (language, lines) in cases {
        let result = run(Request {
            transcript: lines.join(" "),
            language: language.into(),
            embeddings: orthogonal_embeddings(lines.len()),
            timings: Vec::new(),
            options: Options {
                summary_length: SummaryLength::Short,
                min_heavy: 1,
                max_heavy: 2,
                top_fraction: Some(1.0),
                bullet_count: 2,
                ..Options::default()
            },
            embedding_ms: 0.0,
            lexical_only: false,
            stopwords: lexicon_stopwords(language),
        })
        .unwrap();
        assert_eq!(result.ranked.len(), 2, "language: {language}");
        assert_eq!(result.heavy_sentences.len(), 2, "language: {language}");
        assert_eq!(result.bullet_points.len(), 2, "language: {language}");
        for line in lines {
            assert!(result.summary.contains(line), "language: {language}");
            assert!(
                result
                    .bullet_points
                    .iter()
                    .any(|bullet| bullet.text == line),
                "language: {language}"
            );
        }
    }
}

#[test]
fn ranked_timing_and_content_fields_match_the_public_json_contract() {
    let lines = [
        "Apple reported record revenue.",
        "20,000 workers were laid off.",
    ];
    let result = run(request(
        &lines,
        vec![vec3(1., 0., 0.), vec3(0., 1., 0.)],
        Options {
            min_heavy: 2,
            max_heavy: 2,
            ..Default::default()
        },
    ))
    .unwrap();
    let json = serde_json::to_value(result).unwrap();
    assert!(json["ranked"][0]["index"].is_number());
    assert!(json["ranked"][0]["start"].is_null());
    assert!(json["ranked"][0]["importance"].is_number());
    assert!(json["ranked"][0]["centrality"].is_number());
    assert!(json["ranked"][0]["novelty"].is_number());
    assert!(json["ranked"][0]["text"].is_string());
    assert!(json["heavy_sentences"][0]["index"].is_number());
    assert!(json["heavy_sentences"][0]["importance"].is_number());
    assert!(json["heavy_sentences"][0]["centrality"].is_number());
    assert!(json["heavy_sentences"][0]["novelty"].is_number());
    assert!(json["heavy_sentences"][0]["text"].is_string());
    assert!(json["timeline"][0]["index"].is_number());
    assert!(json["bullet_points"][0]["sentence_index"].is_number());
    assert!(json["bullet_points"][0]["text"].is_string());
    assert!(json["summary"].is_string());
    assert!(json["bullet_points"].is_array());
    assert!(json["heavy_sentences"].is_array());
    assert!(json["timeline"].is_array());
    for stage in [
        "split_ms",
        "embedding_ms",
        "similarity_ms",
        "ranking_ms",
        "summary_ms",
        "bullet_ms",
        "total_ms",
    ] {
        assert!(json["timings"][stage].is_number(), "missing timing {stage}");
    }
}

#[test]
fn event_cues_outrank_numeric_only_and_filler_heavy_sentences() {
    assert_eq!(content_signal("The company announced revenue.", "en"), 1.0);
    assert_eq!(content_signal("The meeting had 20 attendees.", "en"), 0.25);
    for numeric_fact in [
        "Profits reached $14.8 billion.",
        "Revenue grew by 35%.",
        "20,000 workers were affected.",
    ] {
        assert_eq!(content_signal(numeric_fact, "en"), 0.25, "{numeric_fact}");
    }
    assert_eq!(
        content_signal("The first result in 30 years was announced in 2026.", "en"),
        1.0
    );
    let clean_score = apply_noise_penalty("The company announced layoffs.", 0.8);
    let filler_score =
        apply_noise_penalty("Well, you know, um, the company announced layoffs.", 0.8);
    assert!(filler_score < clean_score);

    let lines = [
        "Well, you know, the company basically, uh, reported revenue growth.",
        "But the really important thing is they announced 5,000 layoffs.",
        "Executives discussed the plan.",
    ];
    let result = run(request(
        &lines,
        vec![vec3(1., 0., 0.), vec3(1., 0., 0.), vec3(1., 0., 0.)],
        Options {
            min_heavy: 3,
            max_heavy: 3,
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(result.ranked[0].text, lines[1]);

    let event_lines = [
        "The company held a meeting.",
        "Employees discussed the new strategy.",
        "Then the CEO announced that 20,000 employees would be laid off.",
        "The announcement shocked investors.",
        "The meeting ended later that afternoon.",
    ];
    let event_result = run(request(
        &event_lines,
        vec![
            vec3(0., 0., 1.),
            vec3(0., 1., 0.),
            vec3(1., 0., 0.),
            vec3(0.99, 0.01, 0.),
            vec3(0., 0., 1.),
        ],
        Options {
            min_heavy: 2,
            max_heavy: 2,
            top_fraction: Some(0.4),
            ..Options::default()
        },
    ))
    .unwrap();
    assert!(event_result.heavy_sentences.iter().any(|sentence| {
        sentence.text == "Then the CEO announced that 20,000 employees would be laid off."
    }));
    assert!(event_result.ranked.iter().any(|sentence| {
        sentence.text == "The announcement shocked investors." && sentence.importance >= 0.9
    }));
}

#[test]
fn relevant_event_beats_novelty_outlier_and_duplicates_are_removed() {
    let lines = [
        "Apple reported record revenue.",
        "iPhone sales increased.",
        "The company expanded in Europe.",
        "My neighbor owns three cats.",
        "Apple expects more growth next year.",
    ];
    let result = run(request(
        &lines,
        vec![
            vec3(1.0, 0.0, 0.0),
            vec3(0.98, 0.1, 0.0),
            vec3(0.96, 0.2, 0.0),
            vec3(0.0, 1.0, 0.0),
            vec3(0.97, 0.15, 0.0),
        ],
        Options {
            min_heavy: 1,
            max_heavy: 5,
            top_fraction: Some(0.2),
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(!result
        .heavy_sentences
        .iter()
        .any(|sentence| sentence.text.contains("neighbor")));
    let duplicate_lines = [
        "Apple reported record revenue.",
        "Apple reported record revenue.",
        "Apple reported record revenue.",
        "Sales increased strongly.",
        "The company announced a major acquisition.",
    ];
    let duplicates = run(request(
        &duplicate_lines,
        vec![
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(0.9, 0.1, 0.),
            vec3(0., 1., 0.),
        ],
        Options {
            min_heavy: 1,
            max_heavy: 5,
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(
        duplicates
            .heavy_sentences
            .iter()
            .filter(|sentence| sentence.text == duplicate_lines[0])
            .count(),
        1
    );

    let paraphrases = [
        "Apple reported record revenue.",
        "Apple posted record revenue.",
        "Apple announced record sales.",
        "Sales increased strongly.",
    ];
    let semantic_duplicates = run(request(
        &paraphrases,
        vec![
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(1., 0., 0.),
            vec3(0., 1., 0.),
        ],
        Options {
            min_heavy: 1,
            max_heavy: 4,
            top_fraction: Some(1.0),
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(
        semantic_duplicates
            .heavy_sentences
            .iter()
            .filter(|sentence| paraphrases[..3].contains(&sentence.text.as_str()))
            .count()
            <= 1
    );
}

#[test]
fn summary_and_bullets_are_extractively_faithful_and_distinct_from_rank() {
    let lines = [
        "Tesla opened a new factory in Mexico.",
        "The factory will produce electric vehicles.",
        "Production is expected to begin next year.",
        "The project will employ 5,000 workers.",
    ];
    let result = run(request(
        &lines,
        vec![
            vec3(1., 0., 0.),
            vec3(0., 1., 0.),
            vec3(0., 0., 1.),
            vec3(-1., 0., 0.),
        ],
        Options {
            summary_length: SummaryLength::Short,
            min_heavy: 2,
            max_heavy: 2,
            bullet_count: 3,
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(result.summary.contains("factory") && result.summary.contains("Mexico"));
    assert!(!result.summary.to_lowercase().contains("largest"));
    assert_ne!(
        result.summary,
        result
            .heavy_sentences
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert!(result.bullet_points.len() <= 3);
    assert!(result
        .bullet_points
        .iter()
        .all(|bullet| lines.contains(&bullet.text.as_str())));
    let importance_by_index: Vec<f64> = (0..lines.len())
        .map(|index| {
            result
                .ranked
                .iter()
                .find(|sentence| sentence.index == index)
                .unwrap()
                .importance
        })
        .collect();
    assert!(result.bullet_points.windows(2).all(|pair| {
        importance_by_index[pair[0].sentence_index]
            >= importance_by_index[pair[1].sentence_index]
    }));
    let polarity = ["The company did not declare bankruptcy."];
    let result = run(request(
        &polarity,
        vec![vec3(1., 0., 0.)],
        Options {
            min_heavy: 1,
            max_heavy: 1,
            summary_length: SummaryLength::Short,
            ..Default::default()
        },
    ))
    .unwrap();
    assert!(result.summary.contains("did not declare bankruptcy"));
}

#[test]
fn extractive_summary_keeps_entity_associations_and_sentence_polarity() {
    let lines = [
        "Elon Musk discussed Tesla during the interview.",
        "Tim Cook discussed Apple during the interview.",
        "The company did not declare bankruptcy.",
    ];
    let result = run(request(
        &lines,
        orthogonal_embeddings(lines.len()),
        Options {
            summary_length: SummaryLength::Short,
            min_heavy: 1,
            max_heavy: 3,
            bullet_count: 3,
            ..Options::default()
        },
    ))
    .unwrap();
    assert!(result
        .summary
        .split_inclusive('.')
        .all(|sentence| lines.contains(&sentence.trim())));
    assert!(result.summary.contains("Elon Musk discussed Tesla"));
    assert!(result.summary.contains("Tim Cook discussed Apple"));
    assert!(result.summary.contains("did not declare bankruptcy"));
    assert!(result
        .bullet_points
        .iter()
        .all(|bullet| lines.contains(&bullet.text.as_str())));
}

#[test]
fn timeline_is_chronological_and_ranked_is_descending() {
    let lines = [
        "Sentence at time five.",
        "Sentence at time twelve.",
        "Sentence at time twenty.",
    ];
    // The fixture states its own lexical input: this ordering contract must not
    // depend on which per-language stop-word data happens to be installed. An
    // empty set keeps every fixture token a content token, so the three
    // sentences differ only in their one distinct word — the
    // equal-pairwise-similarity shape this check was written against.
    let fixture_stopwords: HashSet<String> = HashSet::new();
    let result = run(Request {
        transcript: lines.join(" "),
        language: "en".into(),
        embeddings: lines
            .iter()
            .map(|line| lexical_vector(line, &fixture_stopwords))
            .map(|vector| vector.into_iter().map(|value| value as f32).collect())
            .collect(),
        stopwords: Vec::new(),
        timings: vec![
            Timing {
                start_us: 5_000_000,
                end_us: 6_000_000,
            },
            Timing {
                start_us: 12_000_000,
                end_us: 13_000_000,
            },
            Timing {
                start_us: 20_000_000,
                end_us: 21_000_000,
            },
        ],
        options: Options {
            min_heavy: 3,
            max_heavy: 3,
            ..Default::default()
        },
        embedding_ms: 5.0,
        lexical_only: false,
    })
    .unwrap();
    assert_eq!(result.ranked[0].index, 1);
    assert!(result
        .ranked
        .windows(2)
        .all(|pair| pair[0].importance >= pair[1].importance));
    assert_eq!(
        result
            .timeline
            .iter()
            .map(|sentence| sentence.start_sec.unwrap())
            .collect::<Vec<_>>(),
        [5., 12., 20.]
    );
    assert!((result.timings.total_ms - 5.0) >= result.timings.split_ms);
    assert!(
        result.timings.total_ms
            >= 5.0
                + result.timings.split_ms
                + result.timings.similarity_ms
                + result.timings.ranking_ms
                + result.timings.summary_ms
                + result.timings.bullet_ms
    );
}

#[test]
fn deterministic_across_one_hundred_runs_and_invalid_vectors_fail_closed() {
    let lines = [
        "The board approved a major acquisition.",
        "Revenue grew by 35% this year.",
        "The company expects further growth.",
        "The board met again later.",
    ];
    let input = request(
        &lines,
        vec![
            vec3(1., 0., 0.),
            vec3(0.9, 0.1, 0.),
            vec3(0.95, 0.05, 0.),
            vec3(0.8, 0.2, 0.),
        ],
        Options::default(),
    );
    let first = run(input.clone()).unwrap();
    for _ in 0..99 {
        let next = run(input.clone()).unwrap();
        assert_eq!(next.summary, first.summary);
        assert_eq!(next.ranked, first.ranked);
        assert_eq!(next.heavy_sentences, first.heavy_sentences);
        assert_eq!(next.bullet_points, first.bullet_points);
        assert_eq!(next.timeline, first.timeline);
    }

    let invalid = request(&lines, vec![vec3(0., 0., 0.); 4], Options::default());
    assert!(run(invalid).unwrap_err().contains("zero norm"));

    let invalid_cases = [
        (
            Request {
                transcript: "Sentence one. Sentence two.".into(),
                language: "en".into(),
                embeddings: vec![vec3(1., 0., 0.)],
                timings: Vec::new(),
                options: Options::default(),
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "embedding count",
        ),
        (
            Request {
                transcript: "Sentence one.".into(),
                language: "en".into(),
                embeddings: vec![vec3(f32::NAN, 0., 0.)],
                timings: Vec::new(),
                options: Options::default(),
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "non-finite",
        ),
        (
            Request {
                transcript: "Sentence one.".into(),
                language: "en".into(),
                embeddings: vec![vec3(1., 0., 0.)],
                timings: vec![Timing {
                    start_us: 2_000_000,
                    end_us: 1_000_000,
                }],
                options: Options::default(),
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "invalid/nonchronological",
        ),
        (
            Request {
                transcript: "Sentence one.".into(),
                language: "en".into(),
                embeddings: vec![vec3(1., 0., 0.)],
                timings: Vec::new(),
                options: Options {
                    top_fraction: Some(1.1),
                    ..Options::default()
                },
                embedding_ms: 0.0,
                lexical_only: false,
                stopwords: lexicon_stopwords("en"),
            },
            "top_fraction",
        ),
    ];
    for (invalid, expected_error) in invalid_cases {
        assert!(
            run(invalid).unwrap_err().contains(expected_error),
            "expected validation error containing {expected_error}"
        );
    }
}

#[test]
fn heavy_selection_obeys_fraction_and_minimum_maximum_for_transcript_sizes() {
    for (count, expected) in [(10, 3), (25, 4), (50, 7), (100, 13), (200, 15)] {
        let lines = labeled_test_sentences(count);
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let result = run(request(
            &borrowed,
            orthogonal_embeddings(count),
            Options {
                min_heavy: 3,
                max_heavy: 15,
                top_fraction: Some(0.125),
                ..Options::default()
            },
        ))
        .unwrap();
        assert_eq!(
            result.heavy_sentences.len(),
            expected,
            "unexpected selection count for {count} sentences"
        );
        assert!(result.heavy_sentences.len() <= 15);
        assert!(result
            .heavy_sentences
            .windows(2)
            .all(|pair| pair[0].importance >= pair[1].importance));
    }
}

#[test]
fn one_thousand_word_transcript_produces_stable_summary_bullets_and_top_ten() {
    let sentences: Vec<String> = (0..50)
        .map(|index| {
            format!("Sentence {index} company announced financial results and increased production across international markets this year during its latest quarterly operating period.")
        })
        .collect();
    assert_eq!(
        sentences.iter().map(|line| word_count(line)).sum::<usize>(),
        1_000
    );
    let borrowed: Vec<&str> = sentences.iter().map(String::as_str).collect();
    let input = request(
        &borrowed,
        orthogonal_embeddings(sentences.len()),
        Options {
            summary_length: SummaryLength::Short,
            bullet_count: 5,
            min_heavy: 10,
            max_heavy: 10,
            top_fraction: Some(0.1),
        },
    );
    let first = run(input.clone()).unwrap();
    assert_eq!(first.ranked.len(), 50);
    assert_eq!(first.heavy_sentences.len(), 10);
    assert_eq!(first.bullet_points.len(), 5);
    assert!(!first.summary.is_empty());
    assert!(first.summary.split_whitespace().count() <= 80);
    assert!(first
        .bullet_points
        .iter()
        .all(|bullet| sentences.iter().any(|line| line == &bullet.text)));
    assert!(first.timings.total_ms.is_finite());
    for _ in 0..9 {
        let next = run(input.clone()).unwrap();
        assert_eq!(next.summary, first.summary);
        assert_eq!(next.ranked, first.ranked);
        assert_eq!(next.heavy_sentences, first.heavy_sentences);
        assert_eq!(next.bullet_points, first.bullet_points);
        assert_eq!(next.timeline, first.timeline);
        assert!(next.timings.total_ms.is_finite());
    }
}

#[test]
fn bullet_count_modes_return_unique_source_sentences() {
    let lines = labeled_test_sentences(12);
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
    for count in [3, 5, 10] {
        let result = run(request(
            &borrowed,
            orthogonal_embeddings(lines.len()),
            Options {
                bullet_count: count,
                ..Options::default()
            },
        ))
        .unwrap();
        assert_eq!(result.bullet_points.len(), count);
        for (index, bullet) in result.bullet_points.iter().enumerate() {
            assert!(lines.iter().any(|line| line == &bullet.text));
            assert!(result.bullet_points[..index]
                .iter()
                .all(|prior| prior.text != bullet.text));
        }
    }
}

#[test]
fn summary_length_modes_stay_within_declared_word_bands_when_source_allows() {
    let lines = labeled_test_sentences(50);
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
    for (mode, minimum, maximum) in [
        (SummaryLength::Short, 50, 80),
        (SummaryLength::Medium, 100, 150),
        (SummaryLength::Long, 200, 300),
    ] {
        let result = run(request(
            &borrowed,
            orthogonal_embeddings(lines.len()),
            Options {
                summary_length: mode,
                ..Options::default()
            },
        ))
        .unwrap();
        let words = word_count(&result.summary);
        assert!(
            (minimum..=maximum).contains(&words),
            "summary has {words} words, expected {minimum}..={maximum}"
        );
        for sentence in result.summary.split_inclusive('.') {
            assert!(lines.iter().any(|line| line == sentence.trim()));
        }
    }
}

#[test]
fn synthetic_labeled_corpus_computes_precision_and_recall_transparently() {
    #[derive(Deserialize)]
    struct Corpus {
        dataset_type: String,
        label_semantics: String,
        embedding_note: String,
        cases: Vec<Case>,
    }
    #[derive(Deserialize)]
    struct Case {
        sentences: Vec<Label>,
    }
    #[derive(Deserialize)]
    struct Label {
        text: String,
        label: u8,
        #[serde(alias = "embedding_group")]
        _embedding_group: String,
    }
    let corpus: Corpus =
        serde_json::from_str(include_str!("../fixtures/phrase_impact_labels.json")).unwrap();
    assert_eq!(
        corpus.dataset_type,
        "synthetic_developer_authored_not_human_evaluation"
    );
    assert!(corpus.label_semantics.contains("not human validation"));
    assert!(corpus.embedding_note.contains("not model embeddings"));
    assert_eq!(corpus.cases.len(), 33);
    let case_count = corpus.cases.len();
    let mut p5_hits = 0;
    let mut p10_hits = 0;
    let mut relevant = 0;
    let mut very_important = 0;
    let mut r10_hits = 0;
    for case in corpus.cases {
        let vectors = case
            .sentences
            .iter()
            .map(|sentence| lexical_vector(&sentence.text, &lexicon_stopword_set("en")))
            .map(|vector| vector.into_iter().map(|value| value as f32).collect())
            .collect();
        let text = case
            .sentences
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let result = run(Request {
            transcript: text,
            language: "en".into(),
            embeddings: vectors,
            timings: Vec::new(),
            options: Options {
                min_heavy: 1,
                max_heavy: 12,
                ..Default::default()
            },
            embedding_ms: 0.0,
            lexical_only: false,
            stopwords: lexicon_stopwords("en"),
        })
        .unwrap();
        let labels: Vec<bool> = case
            .sentences
            .iter()
            .map(|sentence| {
                assert!(sentence.label <= 2);
                sentence.label > 0
            })
            .collect();
        assert_eq!(labels.len(), 12);
        assert!(labels.iter().filter(|&&label| label).count() >= 3);
        relevant += labels.iter().filter(|&&label| label).count();
        very_important += case
            .sentences
            .iter()
            .filter(|sentence| sentence.label == 2)
            .count();
        p5_hits += result
            .ranked
            .iter()
            .take(5)
            .filter(|sentence| labels[sentence.index])
            .count();
        p10_hits += result
            .ranked
            .iter()
            .take(10)
            .filter(|sentence| labels[sentence.index])
            .count();
        r10_hits += result
            .ranked
            .iter()
            .take(10)
            .filter(|sentence| labels[sentence.index])
            .count();
    }
    assert!(very_important > 0, "fixture must include label 2");
    let p5 = p5_hits as f64 / (case_count * 5) as f64;
    let p10 = p10_hits as f64 / (case_count * 10) as f64;
    let r10 = r10_hits as f64 / relevant as f64;
    eprintln!(
        "synthetic fixture only; not human evidence: P@5={p5:.3}, P@10={p10:.3}, R@10={r10:.3}"
    );
    // These loose checks protect only the synthetic harness wiring. They
    // are not evidence for the requested human-judged Precision@K targets.
    assert!(p5 > 0.45, "synthetic regression P@5={p5:.3}");
    assert!(p10 > 0.25, "synthetic regression P@10={p10:.3}");
    assert!(r10 >= 0.75, "synthetic regression R@10={r10:.3}");
}

fn lexical_scene_request(transcript: &str) -> Request {
    Request {
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

#[test]
fn statistical_sentence_is_not_a_title_but_stays_a_bullet() {
    assert!(is_statistical_sentence("Sales rose 40% compared to last year across every region we track."));
    assert!(!is_statistical_sentence("Formula 1"));
    assert!(!is_statistical_sentence("Industria 4.0"));
    assert!(!is_statistical_sentence("G7 summit"));
    assert!(!is_statistical_sentence("Windows 11"));
}

#[test]
fn topic_with_digits_validates_when_it_matches_scene_text() {
    let text = "The G7 summit gathered leaders to discuss industrial cooperation and trade.";
    let (title, status, source) = select_scene_title(text, Some("G7 summit"), &lexicon_stopword_set("en"));
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
        SceneInput { scene_id: "s1".into(), text: scene.into(), topic: None, start_byte: Some(0), end_byte: Some(first_end) },
        SceneInput { scene_id: "s2".into(), text: scene.into(), topic: None, start_byte: Some(first_end + 1), end_byte: Some(transcript.len()) },
    ];
    let doc = run_with_scene_inputs(lexical_scene_request(&transcript), &inputs, &ChapterOptions::default()).unwrap();
    assert_eq!(doc.scene_highlights.len(), 2);
    assert!(doc.scene_highlights.iter().all(|h| h.globally_indexed));
    // Repeated text without slices degrades to local-only, never first-match globals.
    let local = vec![
        SceneInput { scene_id: "s1".into(), text: scene.into(), topic: None, start_byte: None, end_byte: None },
        SceneInput { scene_id: "s2".into(), text: scene.into(), topic: None, start_byte: None, end_byte: None },
    ];
    let doc = run_with_scene_inputs(lexical_scene_request(&transcript), &local, &ChapterOptions::default()).unwrap();
    assert!(doc.scene_highlights.iter().all(|h| !h.globally_indexed));
}

#[test]
fn duplicate_scene_identity_is_a_contract_error() {
    let inputs = vec![
        SceneInput { scene_id: "s1".into(), text: "Alpha beta gamma delta. Epsilon zeta eta theta.".into(), topic: None, start_byte: None, end_byte: None },
        SceneInput { scene_id: "s1".into(), text: "Other text here with enough words to analyze.".into(), topic: None, start_byte: None, end_byte: None },
    ];
    let err = run_with_scene_inputs(lexical_scene_request("Alpha beta gamma delta. Other text here."), &inputs, &ChapterOptions::default()).unwrap_err();
    assert!(err.contains("duplicate"));
}

#[test]
fn utf8_offsets_validate_on_accented_text() {
    let scene = "La città è bellissima. L’industria cresce piano.";
    let transcript = scene.to_string();
    let inputs = vec![SceneInput {
        scene_id: "s1".into(), text: scene.into(), topic: None,
        start_byte: Some(0), end_byte: Some(transcript.len()),
    }];
    let doc = run_with_scene_inputs(lexical_scene_request(&transcript), &inputs, &ChapterOptions::default()).unwrap();
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
    let (title, status, source) = select_scene_title(text, Some("Quantum computing"), &lexicon_stopword_set("en"));
    assert_ne!(source, "topic_validated", "incoherent topic must not validate");
    assert_eq!(status, "resolved");
    let title = title.unwrap();
    assert!(!title.to_lowercase().contains("quantum"));
    // R08 coherent control: matching topic validates.
    let (title, status, source) = select_scene_title(text, Some("neighborhood bakery"), &lexicon_stopword_set("en"));
    assert_eq!((status.as_str(), source.as_str()), ("resolved", "topic_validated"));
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
    let doc = run_with_scene_inputs(lexical_scene_request(&transcript), &inputs, &ChapterOptions::default()).unwrap();
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
        SceneInput { scene_id: "s-a".into(), text: t.into(), topic: None, start_byte: Some(starts[0]), end_byte: Some(starts[0] + t.len()) },
        SceneInput { scene_id: "s-b".into(), text: t.into(), topic: None, start_byte: Some(starts[1]), end_byte: Some(starts[1] + t.len()) },
        SceneInput { scene_id: "s-c".into(), text: t.into(), topic: None, start_byte: Some(starts[2]), end_byte: Some(starts[2] + t.len()) },
    ];
    let doc = run_with_scene_inputs(lexical_scene_request(&transcript), &inputs, &ChapterOptions::default()).unwrap();
    let ids: Vec<&str> = doc.scene_highlights.iter().map(|h| h.scene_id.as_str()).collect();
    assert_eq!(ids, vec!["s-a", "s-b", "s-c"]);
}

#[test]
fn two_concepts_in_one_scene_yield_distinct_bullets() {
    // R11: one scene, two concepts -> distinct bullets, no forced chapters.
    let text = "Germany lost one hundred thousand industrial jobs in a single year, and factory orders declined sharply across every major region. Economists warn that the downturn may persist through next winter unless exports recover soon. Meanwhile the national football team won the championship after a dramatic final match watched by millions. Celebrations filled the streets of Berlin until the early morning hours.";
    let doc = run_with_scene_inputs(
        lexical_scene_request(text),
        &[SceneInput { scene_id: "s1".into(), text: text.into(), topic: None, start_byte: Some(0), end_byte: Some(text.len()) }],
        &ChapterOptions::default(),
    ).unwrap();
    assert_eq!(doc.scene_highlights.len(), 1, "one scene yields one highlight, not forced chapters");
    let h = &doc.scene_highlights[0];
    assert!(h.bullets.len() >= 2, "two concepts deserve two bullets, got {}", h.bullets.len());
    let joined = h.bullets.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join(" | ").to_lowercase();
    let industry = joined.contains("jobs") || joined.contains("factory") || joined.contains("exports");
    let sport = joined.contains("football") || joined.contains("championship") || joined.contains("berlin");
    assert!(industry && sport, "bullets must span both concepts: {joined}");
    for b in &h.bullets {
        assert!(text.contains(&b.text));
    }
}

#[test]
fn short_scene_invents_no_bullets_and_missing_title_is_unavailable() {
    let inputs = vec![SceneInput {
        scene_id: "s1".into(), text: "Ciao.".into(), topic: None,
        start_byte: None, end_byte: None,
    }];
    let doc = run_with_scene_inputs(
        lexical_scene_request("Ciao. Questa è una scena più lunga con abbastanza parole per l'analisi completa del sistema."),
        &inputs, &ChapterOptions::default(),
    ).unwrap();
    assert_eq!(doc.scene_highlights.len(), 1);
    assert!(doc.scene_highlights[0].bullets.is_empty(), "no invented bullets for a one-word scene");
}
