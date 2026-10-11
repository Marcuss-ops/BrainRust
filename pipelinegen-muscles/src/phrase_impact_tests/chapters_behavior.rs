use super::super::*;
use super::common::*;

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

fn analyze_chapter_text(text: &str, language: &str) -> Result<ResultDocument, MuscleError> {
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
        run_with_chapter_options(request.clone(), &[], &[], &behavioral_chapter_options()).unwrap();
    let scenes = sentences.to_vec();
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
    assert!(
        (1..=3).contains(&result.chapter_manifest.chapters.len()),
        "a gradual topical transition should not cause sentence-by-sentence fragmentation: {:?}",
        result.chapter_manifest.chapters
    );
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
            run_with_chapter_options(request(), &[], &[], &behavioral_chapter_options()).unwrap();
        let second =
            run_with_chapter_options(request(), &[], &[], &behavioral_chapter_options()).unwrap();
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
        run_with_chapter_options(request.clone(), &[], &[], &behavioral_chapter_options()).unwrap();
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
        run_with_chapter_options(request.clone(), &[], &[], &behavioral_chapter_options()).unwrap();
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
    assert!(run_with_chapter_options(malformed, &[], &[], &behavioral_chapter_options()).is_err());
    let short = analyze_chapter_text("Rain falls.", "en").unwrap();
    assert_eq!(short.chapter_manifest.chapters.len(), 1);
    assert_eq!(short.chapter_manifest.chapters[0].start_sentence, 0);
    assert_eq!(short.chapter_manifest.chapters[0].end_sentence, 1);
}
