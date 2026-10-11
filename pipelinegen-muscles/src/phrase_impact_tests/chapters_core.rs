use super::super::*;
use super::common::*;

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
        high_penalty.chapter_manifest.chapters.len() <= low_penalty.chapter_manifest.chapters.len()
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
