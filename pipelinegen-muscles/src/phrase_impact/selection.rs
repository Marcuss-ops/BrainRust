//! Internal split of phrase_impact (pure move, no behavior change).

use super::*;

pub(super) fn contextual_bullet_span(
    index: usize,
    chapter_start: usize,
    chapter_end: usize,
    vectors: &[Vec<f64>],
) -> (usize, usize) {
    if chapter_end.saturating_sub(chapter_start) < 2 {
        return (index, index + 1);
    }
    let adjacent: Vec<f64> = (chapter_start..chapter_end.saturating_sub(1))
        .map(|at| cosine(&vectors[at], &vectors[at + 1]))
        .collect();
    let baseline = adjacent.iter().sum::<f64>() / adjacent.len() as f64;
    let mut start = index;
    let mut end = index + 1;
    if index > chapter_start && cosine(&vectors[index], &vectors[index - 1]) > baseline {
        start -= 1;
    }
    if index + 1 < chapter_end && cosine(&vectors[index], &vectors[index + 1]) > baseline {
        end += 1;
    }
    (start, end)
}

/// True for full statistical propositions ("sales rose 40%", "down 6% since
/// 2020"): penalized as titles, still valid as bullets. Short nominals with
/// digits ("Formula 1", "Industria 4.0", "G7", "Windows 11") return false.
pub(super) fn is_statistical_sentence(text: &str) -> bool {
    let lower = text.to_lowercase();
    let has_digit = text.chars().any(|c| c.is_ascii_digit());
    if !has_digit {
        return false;
    }
    let words = text.split_whitespace().count();
    if words <= 4 {
        return false;
    }
    let stat_markers = ['%', '$', '€', '£'];
    if text.chars().any(|c| stat_markers.contains(&c)) {
        return true;
    }
    let verbs = [
        " rose ",
        " fell ",
        " grew ",
        " dropped ",
        " increased ",
        " decreased ",
        " declined ",
        " surged ",
        " shrank ",
        " down ",
        " up ",
        " by ",
        " aumentate ",
        " aumentato ",
        " diminuito ",
        " calo ",
        " crescita ",
        " riduzione ",
        " rispetto ",
        " since ",
        " from ",
        " dal ",
    ];
    let padded = format!(" {lower} ");
    verbs.iter().any(|v| padded.contains(v))
}

pub(super) fn title_case_phrase(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Topic-first title selection: topic is a prioritized candidate validated
/// against the real scene text, never blind truth. Falls back to an extracted
/// nominal keyphrase; returns None (unavailable) instead of inventing one.
pub(super) fn select_scene_title(
    scene_text: &str,
    topic: Option<&str>,
    stopwords: &HashSet<String>,
) -> (Option<String>, String, String) {
    if let Some(raw) = topic.map(str::trim).filter(|t| !t.is_empty()) {
        let normalized = raw.replace(['-', '_'], " ");
        let topic_tokens: Vec<String> = normalized
            .split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .filter(|t| t.chars().count() > 1)
            .collect();
        if !topic_tokens.is_empty() {
            let lower_scene = scene_text.to_lowercase();
            let overlap = topic_tokens
                .iter()
                .filter(|t| lower_scene.contains(t.as_str()))
                .count();
            let coverage = overlap as f64 / topic_tokens.len() as f64;
            if coverage >= 0.34 && !is_statistical_sentence(&normalized) {
                return (
                    Some(title_case_phrase(&normalized)),
                    "resolved".into(),
                    "topic_validated".into(),
                );
            }
        }
    }
    let (title, source) = chapter_title(scene_text, &[], &[], stopwords);
    if title.trim().is_empty()
        || is_statistical_sentence(&title) && title.split_whitespace().count() > 5
    {
        return (None, "unavailable".into(), "none".into());
    }
    if is_statistical_sentence(scene_text) && title == scene_text.trim() {
        return (None, "unavailable".into(), "none".into());
    }
    (Some(title), "resolved".into(), source)
}

/// Extract short overlay spans inside top sentences. The engine may score
/// longer candidates; visual_eligible marks the <=5-token budget subset.
pub(super) fn select_phrase_spans(
    sentences: &[Segment],
    importance: &[f64],
    stopwords: &HashSet<String>,
    limit: usize,
) -> Vec<ScenePhraseSpan> {
    let mut order: Vec<usize> = (0..sentences.len()).collect();
    order.sort_by(|&a, &b| importance[b].total_cmp(&importance[a]).then(a.cmp(&b)));
    let mut out = Vec::new();
    for &si in order.iter().take(limit * 2 + 2) {
        let seg = &sentences[si];
        let tokens: Vec<(usize, usize, &str)> = {
            let mut v = Vec::new();
            let mut idx = 0usize;
            for tok in seg.text.split_whitespace() {
                if let Some(pos) = seg.text[idx..].find(tok) {
                    let s = idx + pos;
                    v.push((seg.start_byte + s, seg.start_byte + s + tok.len(), tok));
                    idx = s + tok.len();
                }
            }
            v
        };
        if tokens.len() < 2 {
            continue;
        }
        let mut best: Option<(usize, usize, f64)> = None;
        for len in 2..=tokens.len().min(8) {
            for win in tokens.windows(len) {
                // `len >= 2`, so the window always has a first and last token.
                let [first_tok, .., last_tok] = win else {
                    continue;
                };
                let first = first_tok.2.to_lowercase();
                let last = last_tok.2.to_lowercase();
                if stopwords.contains(&first) || stopwords.contains(&last) {
                    continue;
                }
                let text = win.iter().map(|(_, _, t)| *t).collect::<Vec<_>>().join(" ");
                if text.chars().count() < 4 {
                    continue;
                }
                let rarity = win
                    .iter()
                    .map(|(_, _, t)| {
                        if t.chars().all(|c| c.is_alphabetic()) && t.len() > 5 {
                            1.5
                        } else {
                            1.0
                        }
                    })
                    .sum::<f64>()
                    / len as f64;
                let brevity = if len <= 5 { 1.2 } else { 0.85 };
                let score = importance[si] * rarity * brevity;
                if best.map(|(_, _, s)| score > s).unwrap_or(true) {
                    best = Some((first_tok.0, last_tok.1, score));
                }
            }
        }
        if let Some((s, e, score)) = best {
            let bytes = &sentences[si].text.as_bytes()[s - seg.start_byte..e - seg.start_byte];
            let text = String::from_utf8_lossy(bytes).into_owned();
            let visual = text.split_whitespace().count() <= 5;
            out.push(ScenePhraseSpan {
                sentence_index: si,
                start_byte: s,
                end_byte: e,
                text,
                score,
                visual_eligible: visual,
            });
            if out.len() >= limit {
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.sentence_index.cmp(&b.sentence_index))
    });
    out.truncate(limit);
    out
}

/// Terminal punctuation (optionally followed by a closing quote or bracket)
/// means the segment closes a sentence. A head chunk cut at
/// MAX_SEGMENT_WORDS ends mid-sentence and is never published as a bullet.
pub(super) fn bullet_closes_sentence(text: &str) -> bool {
    let trimmed = text.trim_end();
    let mut chars = trimmed.chars().rev();
    match chars.next() {
        Some('.' | '!' | '?' | '…' | '。' | '！' | '？') => true,
        Some('"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»') => matches!(
            chars.next(),
            Some('.' | '!' | '?' | '…' | '。' | '！' | '？')
        ),
        _ => false,
    }
}

/// A bullet needs a minimum of substance: two-word tails like "impronta 33."
/// are fragments, not points. CJK sentences carry no spaces, so the
/// alphanumeric-character floor keeps whole non-spaced sentences eligible.
pub(super) fn bullet_has_substance(text: &str) -> bool {
    word_count(text) >= MIN_BULLET_WORDS
        || text.chars().filter(|ch| ch.is_alphanumeric()).count() >= MIN_BULLET_CHARS
}

/// Whitespace-collapsed, case-folded identity used for cross-scene bullet
/// dedup, so a repeated sentence is recognized even when spacing varies.
pub(super) fn bullet_identity(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A bullet must START a sentence: index 0, or the previous segment closes a
/// sentence. A continuation chunk of a long sentence is not a sentence start.
pub(super) fn bullet_starts_sentence(texts: &[&str], index: usize) -> bool {
    index == 0 || bullet_closes_sentence(texts[index - 1])
}

/// A bullet must COMPLETE a sentence: the segment closes one, or it is the
/// transcript tail (the final segment may legitimately end without
/// punctuation).
pub(super) fn bullet_completes_sentence(texts: &[&str], index: usize) -> bool {
    bullet_closes_sentence(texts[index]) || index + 1 == texts.len()
}

/// Expand (start, end) to the boundaries of the sentence it touches, inside
/// [lower, upper). Returns None when the sentence crosses the window: it
/// cannot be quoted whole, so the extractor drops it instead of publishing a
/// truncated slice.
pub(super) fn sentence_complete_span(
    texts: &[&str],
    start: usize,
    end: usize,
    lower: usize,
    upper: usize,
) -> Option<(usize, usize)> {
    let mut start = start;
    let mut end = end;
    while start > lower && !bullet_closes_sentence(texts[start - 1]) {
        start -= 1;
    }
    if start > 0 && !bullet_closes_sentence(texts[start - 1]) {
        return None;
    }
    while end < upper && !bullet_closes_sentence(texts[end - 1]) {
        end += 1;
    }
    if end == upper && end < texts.len() && !bullet_closes_sentence(texts[end - 1]) {
        return None;
    }
    Some((start, end))
}


