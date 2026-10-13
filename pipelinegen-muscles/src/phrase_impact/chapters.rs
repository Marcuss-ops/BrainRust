//! Internal split of phrase_impact (pure move, no behavior change).

use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct BoundaryEvidence {
    semantic: f64,
    lexical: f64,
    scene: f64,
}

impl BoundaryEvidence {
    fn score(self, profile: &ChapterOptions) -> f64 {
        let weight = profile.semantic_weight + profile.lexical_weight + profile.scene_weight;
        if weight <= f64::EPSILON {
            return 0.0;
        }
        (self.semantic * profile.semantic_weight
            + self.lexical * profile.lexical_weight
            + self.scene * profile.scene_weight)
            / weight
    }
}

pub(super) fn token_distribution(sentences: &[RankedSentence]) -> std::collections::HashMap<String, f64> {
    let mut counts = std::collections::HashMap::<String, f64>::new();
    for sentence in sentences {
        for token in sentence.text.split(|ch: char| !ch.is_alphanumeric()) {
            let token = token.to_lowercase();
            if token.chars().count() > 1 {
                *counts.entry(token).or_default() += 1.0;
            }
        }
    }
    let norm = counts
        .values()
        .map(|count| count * count)
        .sum::<f64>()
        .sqrt();
    if norm > f64::EPSILON {
        for count in counts.values_mut() {
            *count /= norm;
        }
    }
    counts
}

pub(super) fn distribution_similarity(
    left: &std::collections::HashMap<String, f64>,
    right: &std::collections::HashMap<String, f64>,
) -> f64 {
    left.iter()
        .map(|(token, value)| value * right.get(token).copied().unwrap_or_default())
        .sum::<f64>()
        .clamp(0.0, 1.0)
}

pub(super) fn boundary_evidence(
    ranked: &[RankedSentence],
    vectors: &[Vec<f64>],
    boundary: usize,
    scene_starts: &[(usize, usize)],
    profile: &ChapterOptions,
) -> BoundaryEvidence {
    if boundary == 0 || boundary >= ranked.len() {
        return BoundaryEvidence::default();
    }
    let window = profile.context_sentences.max(1);
    let left_start = boundary.saturating_sub(window);
    let right_end = (boundary + window).min(ranked.len());
    let left = left_start..boundary;
    let right = boundary..right_end;
    let mut cross = 0.0;
    let mut cross_count = 0usize;
    for a in left.clone() {
        for b in right.clone() {
            cross += cosine(&vectors[a], &vectors[b]);
            cross_count += 1;
        }
    }
    let left_tokens = token_distribution(&ranked[left]);
    let right_tokens = token_distribution(&ranked[right]);
    BoundaryEvidence {
        semantic: 1.0 - cross / cross_count.max(1) as f64,
        lexical: 1.0 - distribution_similarity(&left_tokens, &right_tokens),
        scene: f64::from(scene_starts.iter().any(|(start, _)| *start == boundary)),
    }
}

/// Extracts a chapter/scene title from `text`. The only language-specific input
/// is the injected `stopwords` set: this function knows nothing about any
/// particular language, so it behaves identically for every language whose
/// stop-word set the caller supplies.
pub(super) fn chapter_title(
    text: &str,
    topics: &[String],
    scene_ids: &[usize],
    stopwords: &HashSet<String>,
) -> (String, String) {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .collect();
    // Every language-specific token this function filters on arrives through
    // `stopwords`. The authoritative multi-language resource is the repository
    // lexicon (config/lexicons/<lang>/, injected through the Go adapter), so the
    // crate needs no hand-maintained list per language and a language the
    // lexicon does not enumerate degrades to the cross-linguistic fallback
    // profile instead of to an unrelated language's words.
    let is_concept_token = |word: &str| {
        word.chars().count() > 1 && !word.chars().all(char::is_numeric) && !stopwords.contains(word)
    };
    let mut token_frequency = std::collections::HashMap::<String, usize>::new();
    for word in words.iter().filter(|word| is_concept_token(word)) {
        *token_frequency.entry(word.clone()).or_default() += 1;
    }
    // Candidate spans are enumerated over MAXIMAL RUNS of content tokens, which
    // lets each one be checked for completeness: a span is complete when it
    // covers a whole run, i.e. the tokens on both sides of it are stop words (or
    // the text boundary). Completeness is a purely structural property that
    // needs only the injected stop set, so this one rule works in every
    // language and is what stops a title from being cut mid-phrase.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0;
    while cursor < words.len() {
        if !is_concept_token(&words[cursor]) {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < words.len() && is_concept_token(&words[cursor]) {
            cursor += 1;
        }
        runs.push((start, cursor));
    }
    let mut counts = std::collections::HashMap::<String, usize>::new();
    let mut span_quality = std::collections::HashMap::<String, f64>::new();
    let mut record = |phrase: String, quality: f64| {
        *counts.entry(phrase.clone()).or_default() += 1;
        let slot = span_quality.entry(phrase).or_insert(0.0);
        if quality > *slot {
            *slot = quality;
        }
    };
    for (start, end) in runs {
        let span = end - start;
        if span > MAX_TITLE_TOKENS {
            // The run cannot be quoted whole, so it must be trimmed. It
            // contributes exactly ONE candidate — its TAIL — and never its head
            // or an interior window: that rule is what stops a title from being
            // a cut that starts or ends mid-phrase, and it removes the arbitrary
            // choice the old code made by term rarity and then by the sort
            // tie-break. A tail that shares a token with a longer run elsewhere
            // still has to win on its own score.
            record(
                words[end - MAX_TITLE_TOKENS..end].join(" "),
                TRIMMED_RUN_TAIL_SPAN,
            );
            continue;
        }
        for length in 2..=span {
            for offset in 0..=(span - length) {
                let quality = if length == span {
                    WHOLE_RUN_SPAN
                } else {
                    PARTIAL_SPAN
                };
                record(
                    words[start + offset..start + offset + length].join(" "),
                    quality,
                );
            }
        }
    }
    let document_frequency = token_frequency.clone();
    let mut candidates: Vec<(String, f64, &'static str)> = counts
        .into_iter()
        .map(|(phrase, frequency)| {
            let tokens: Vec<&str> = phrase.split_whitespace().collect();
            let specificity = tokens
                .iter()
                .map(|token| {
                    1.0 + (words.len() as f64
                        / token_frequency.get(*token).copied().unwrap_or(1) as f64)
                        .ln()
                })
                .sum::<f64>()
                / tokens.len().max(1) as f64;
            let term_weight = tokens
                .iter()
                .map(|token| 1.0 / document_frequency.get(*token).copied().unwrap_or(1) as f64)
                .sum::<f64>();
            let coverage = tokens.len() as f64 / words.len().max(1) as f64;
            let quality = span_quality.get(&phrase).copied().unwrap_or(PARTIAL_SPAN);
            (
                phrase,
                specificity * coverage.sqrt() * frequency as f64 * term_weight * quality,
                "extractive_keyphrase",
            )
        })
        .collect();
    let mut topic_candidates = std::collections::HashMap::<String, (f64, f64, usize)>::new();
    for &scene in scene_ids {
        if let Some(topic) = topics.get(scene) {
            let normalized = topic.replace(['-', '_'], " ").trim().to_string();
            let topic_tokens: Vec<String> = normalized
                .split(|character: char| !character.is_alphanumeric())
                .map(str::to_lowercase)
                .filter(|token| token.chars().count() > 1)
                .collect();
            if topic_tokens.is_empty() {
                continue;
            }
            let overlap = topic_tokens
                .iter()
                .filter(|token| token_frequency.contains_key(*token))
                .count();
            let coverage = overlap as f64 / topic_tokens.len() as f64;
            if coverage > 0.0 {
                let specificity = topic_tokens
                    .iter()
                    .filter_map(|token| token_frequency.get(token))
                    .map(|frequency| 1.0 / *frequency as f64)
                    .sum::<f64>()
                    / topic_tokens.len() as f64;
                let entry = topic_candidates.entry(normalized).or_default();
                entry.0 += coverage;
                entry.1 += specificity;
                entry.2 += 1;
            }
        }
    }
    for (topic, (coverage, specificity, count)) in topic_candidates {
        let samples = count.max(1) as f64;
        let coverage = coverage / samples;
        let specificity = specificity / samples;
        let words_in_topic = topic.split_whitespace().count().max(1) as f64;
        let score = coverage * specificity * words_in_topic;
        candidates.push((topic, score, "scene_topic"));
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if let Some((phrase, _, source)) = candidates.into_iter().next() {
        let title = phrase
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ");
        (title, source.into())
    } else {
        let fallback = words
            .iter()
            .find(|word| is_concept_token(word))
            .cloned()
            .unwrap_or_else(|| {
                text.split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_lowercase()
            });
        (fallback, "extractive_token".into())
    }
}


pub(super) fn build_chapters(
    ranked: &[RankedSentence],
    vectors: &[Vec<f64>],
    timings: &[Timing],
    scene_starts: &[(usize, usize)],
    topics: &[String],
    options: &ChapterOptions,
    stopwords: &HashSet<String>,
) -> Vec<Chapter> {
    let min_sentences = options.min_sentences.max(1);
    let max_sentences = options.max_sentences.max(min_sentences);
    if ranked.is_empty() {
        return Vec::new();
    }

    // Dynamic programming maximizes within-chapter pair cohesion plus the
    // evidence at each selected boundary, while charging a profile-level
    // complexity penalty for every additional chapter.
    let n = ranked.len();
    let mut words_prefix = vec![0usize; n + 1];
    for index in 0..n {
        words_prefix[index + 1] = words_prefix[index] + word_count(&ranked[index].text);
    }
    let evidence: Vec<f64> = (0..=n)
        .map(|boundary| {
            boundary_evidence(ranked, vectors, boundary, scene_starts, options).score(options)
        })
        .collect();
    let mut best_score = vec![f64::NEG_INFINITY; n + 1];
    let mut previous = vec![None; n + 1];
    let mut chapter_counts = vec![usize::MAX; n + 1];
    best_score[0] = 0.0;
    chapter_counts[0] = 0;
    for end in 1..=n {
        let mut vector_sum = vec![0.0; vectors[0].len()];
        let mut pair_cohesion = 0.0;
        for begin in (0..end).rev().take(max_sentences) {
            let added = &vectors[begin];
            pair_cohesion += added
                .iter()
                .zip(&vector_sum)
                .map(|(a, b)| a * b)
                .sum::<f64>();
            for (sum, value) in vector_sum.iter_mut().zip(added) {
                *sum += value;
            }
            let length = end - begin;
            let short_only_document = begin == 0 && end == n && length < min_sentences;
            let short_tail = end == n && length < min_sentences;
            if (!short_only_document && !short_tail && length < min_sentences)
                || (words_prefix[end] - words_prefix[begin] < options.min_words && end != n)
                || !best_score[begin].is_finite()
            {
                continue;
            }
            let cohesion_pairs = length.saturating_mul(length.saturating_sub(1)) / 2;
            let cohesion = if cohesion_pairs == 0 {
                0.0
            } else {
                pair_cohesion / cohesion_pairs as f64
            };
            let length_prior = (length as f64 / max_sentences as f64).ln_1p();
            let score = best_score[begin]
                + cohesion
                + length_prior
                + options.evidence_weight * evidence[begin]
                - options.complexity_penalty;
            let count = chapter_counts[begin] + 1;
            let replace = score > best_score[end] + f64::EPSILON
                || ((score - best_score[end]).abs() <= f64::EPSILON
                    && (count < chapter_counts[end]
                        || (count == chapter_counts[end]
                            && previous[end].is_some_and(|old| begin < old))));
            if replace {
                best_score[end] = score;
                chapter_counts[end] = count;
                previous[end] = Some(begin);
            }
        }
    }
    if previous[n].is_none() {
        // Product limits are soft at document edges; a coherent bounded
        // partition remains preferable to dropping transcript coverage.
        let mut cursor = n;
        while cursor > 0 {
            let begin = cursor.saturating_sub(max_sentences);
            previous[cursor] = Some(begin);
            cursor = begin;
        }
    }
    let mut boundaries = vec![n];
    let mut cursor = n;
    while cursor > 0 {
        let begin = previous[cursor].unwrap_or(0);
        if begin >= cursor {
            break;
        }
        boundaries.push(begin);
        cursor = begin;
    }
    boundaries.sort_unstable();

    let mut chapters = Vec::new();
    for pair in boundaries.windows(2) {
        let (begin, end) = (pair[0], pair[1]);
        if begin >= end {
            continue;
        }
        let mut scene_ids = Vec::new();
        for (scene_pos, &(scene_begin, scene_id)) in scene_starts.iter().enumerate() {
            let scene_end = scene_starts
                .get(scene_pos + 1)
                .map(|(next, _)| *next)
                .unwrap_or(ranked.len());
            let overlap = end.min(scene_end).saturating_sub(begin.max(scene_begin));
            scene_ids.extend(std::iter::repeat_n(scene_id, overlap));
        }
        let text = ranked[begin..end]
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let (title, title_source) = chapter_title(&text, topics, &scene_ids, stopwords);
        let mut candidates: Vec<usize> = (begin..end).collect();
        candidates.sort_by(|&a, &b| {
            ranked[b]
                .importance
                .total_cmp(&ranked[a].importance)
                .then_with(|| {
                    let coherence = |index: usize| {
                        let left = if index > begin {
                            cosine(&vectors[index], &vectors[index - 1])
                        } else {
                            0.0
                        };
                        let right = if index + 1 < end {
                            cosine(&vectors[index], &vectors[index + 1])
                        } else {
                            0.0
                        };
                        left.max(right)
                    };
                    coherence(b).total_cmp(&coherence(a))
                })
                .then(a.cmp(&b))
        });
        let ranked_texts: Vec<&str> = ranked
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect();
        let mut selected: Vec<usize> = Vec::new();
        for candidate in candidates {
            // Same extraction contract as every other bullet surface: a
            // chapter bullet anchors on a complete, substantive sentence,
            // never on a fragment or the continuation chunk of a long one.
            if !bullet_starts_sentence(&ranked_texts, candidate)
                || !bullet_completes_sentence(&ranked_texts, candidate)
                || !bullet_has_substance(ranked_texts[candidate])
            {
                continue;
            }
            if selected
                .iter()
                .all(|&prior| !equivalent(&ranked[candidate], &ranked[prior], vectors))
            {
                selected.push(candidate);
                if selected.len() >= options.bullet_count.clamp(1, 10) {
                    break;
                }
            }
        }
        selected.sort_unstable();
        let mut spans: Vec<(usize, usize)> = selected
            .into_iter()
            .map(|index| contextual_bullet_span(index, begin, end, vectors))
            // Contextual expansion must not end on a sentence boundary either:
            // widen to the whole sentence when it fits inside the chapter, and
            // drop the span when the sentence crosses the chapter edge.
            .filter_map(|(span_start, span_end)| {
                sentence_complete_span(&ranked_texts, span_start, span_end, begin, end)
            })
            .collect();
        spans.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (span_start, span_end) in spans {
            if let Some(last) = merged.last_mut() {
                if span_start <= last.1 {
                    last.1 = last.1.max(span_end);
                    continue;
                }
            }
            merged.push((span_start, span_end));
        }
        let bullets = merged
            .into_iter()
            .filter_map(|(span_start, span_end)| {
                let text = ranked[span_start..span_end]
                    .iter()
                    .map(|sentence| sentence.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                bullet_has_substance(&text).then_some(ChapterBullet {
                    start_sentence: span_start,
                    end_sentence: span_end,
                    text,
                })
            })
            .collect();
        let start_ms = timings.get(begin).map(|timing| timing.start_us / 1000);
        let end_ms = timings
            .get(end.saturating_sub(1))
            .map(|timing| timing.end_us / 1000);
        chapters.push(Chapter {
            title,
            title_source,
            start_sentence: begin,
            end_sentence: end,
            start_ms,
            end_ms,
            bullets,
        });
    }
    chapters
}

