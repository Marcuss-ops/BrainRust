//! Internal split of phrase_impact (pure move, no behavior change).

use super::*;

pub(super) fn content_signal(text: &str, language: &str) -> f64 {
    let text = text.to_lowercase();
    let language = language.to_lowercase();
    // News-event salience cues live in the shared lexicon table
    // (`discourse_cues.json`), one entry per language with a `default`
    // fallback — knowledge out of code, so a new language is a data change.
    let cues = crate::lexicon::lexicon()
        .discourse
        .content_cues_for(&language);
    if cues.iter().any(|cue| text.contains(cue.as_str())) {
        1.0
    } else if text
        .chars()
        .any(|ch| ch.is_ascii_digit() || matches!(ch, '$' | '€' | '£' | '¥' | '%'))
    {
        0.25
    } else {
        0.0
    }
}

pub(super) fn apply_noise_penalty(text: &str, importance: f64) -> f64 {
    let normalized = text.to_lowercase();
    let words: Vec<&str> = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let lexicon = crate::lexicon::lexicon();
    let mut filler_count = words
        .iter()
        .filter(|word| {
            lexicon
                .discourse
                .filler_words
                .iter()
                .any(|f| f.as_str() == **word)
        })
        .count();
    for phrase in &lexicon.discourse.filler_phrases {
        filler_count += words
            .windows(phrase.len())
            .filter(|window| {
                window
                    .iter()
                    .zip(phrase)
                    .all(|(word, filler)| *word == filler.as_str())
            })
            .count();
    }
    if words.first() == Some(&"well") {
        filler_count += 1;
    }
    let multiplier = match filler_count {
        0 => 1.0,
        1 => 0.75,
        _ => 0.5,
    };
    importance * multiplier
}

pub(super) fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

pub(super) fn target_summary_words(mode: &SummaryLength) -> (usize, usize, usize) {
    match mode {
        SummaryLength::Short => (65, 50, 80),
        SummaryLength::Medium => (125, 100, 150),
        SummaryLength::Long => (250, 200, 300),
    }
}

pub(super) fn normalized_text(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn equivalent(a: &RankedSentence, b: &RankedSentence, vectors: &[Vec<f64>]) -> bool {
    normalized_text(&a.text) == normalized_text(&b.text)
        || cosine(&vectors[a.index], &vectors[b.index]) >= DUPLICATE_COSINE
}

pub(super) fn select_diverse(
    indices: &[usize],
    ranked: &[RankedSentence],
    vectors: &[Vec<f64>],
    limit: usize,
) -> Vec<usize> {
    let mut chosen = Vec::new();
    for &index in indices {
        if chosen
            .iter()
            .any(|&prior| equivalent(&ranked[index], &ranked[prior], vectors))
        {
            continue;
        }
        chosen.push(index);
        if chosen.len() >= limit {
            break;
        }
    }
    chosen
}

pub(super) fn extract_summary(
    ranked: &[RankedSentence],
    vectors: &[Vec<f64>],
    mode: &SummaryLength,
) -> String {
    let (target, minimum, maximum) = target_summary_words(mode);
    let mut center = vec![0.0; vectors[0].len()];
    for vector in vectors {
        for (out, value) in center.iter_mut().zip(vector) {
            *out += value;
        }
    }
    let norm = center.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm > f64::EPSILON {
        for value in &mut center {
            *value /= norm;
        }
    }
    let mut order: Vec<usize> = (0..ranked.len()).collect();
    order.sort_by(|&a, &b| {
        let score = |i: usize| 0.8 * cosine(&vectors[i], &center) + 0.2 * ranked[i].centrality;
        score(b).total_cmp(&score(a)).then(a.cmp(&b))
    });
    let order = select_diverse(&order, ranked, vectors, ranked.len());
    let mut chosen = Vec::new();
    let mut count = 0;
    for index in order {
        let size = word_count(&ranked[index].text);
        if size == 0 || size > maximum || (count > 0 && count + size > maximum) {
            continue;
        }
        chosen.push(index);
        count += size;
        if count >= target {
            break;
        }
    }
    if count < minimum {
        for (index, sentence) in ranked.iter().enumerate() {
            if chosen.contains(&index) {
                continue;
            }
            let size = word_count(&sentence.text);
            if size > 0 && count + size <= maximum {
                chosen.push(index);
                count += size;
            }
            if count >= minimum {
                break;
            }
        }
    }
    chosen.sort_unstable();
    chosen
        .into_iter()
        .map(|index| ranked[index].text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
