//! Internal split of scene_intelligence (pure move, no behavior change).

use std::collections::HashMap;

use super::*;

// ── 4. Timeline events ────────────────────────────────────────────

/// One historical event mention. `normalized_year` is set ONLY for explicit
/// absolute years; relative expressions ("tre anni dopo") stay relative
/// unless the caller supplies an anchor. Video-playback time is NEVER stored
/// here — join it via the scene's `video_start_ms`/`video_end_ms` instead.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TemporalEvent {
    pub event_id: String,
    pub text_span: String,
    pub temporal_expression: String,
    pub normalized_year: Option<i32>,
    /// End year for intervals ("2020–2023"): an interval is represented
    /// as a span, never collapsed to an instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_end_year: Option<i32>,
    pub certainty: f64,
    pub narration_index: usize,
    /// Position of this event in the order it was NARRATED (0-based, across
    /// the analyzed sentences). The list is sorted chronologically for
    /// certified behaviour, but the animator must follow this order: "Nel
    /// 2005, ripensando al 1998…" must activate 2005 first, not 1998.
    #[serde(default)]
    pub narration_order: usize,
    /// Byte span of the individual temporal surface ("1998", "2020–2023",
    /// "tre anni dopo") inside the SCENE text, computed as the sentence's
    /// start offset plus the in-sentence match. This replaces the
    /// sentence-index-only anchoring, so the planner can bind each event to
    /// certified voiceover word timing without re-parsing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_byte: Option<usize>,
}

/// Extract year mentions and order them chronologically (narration order is
/// preserved in `narration_index`). With an anchor year, relative
/// expressions ("tre anni dopo") resolve against it; without one they stay
/// unresolved (None) — ambiguity is never guessed away.
pub fn extract_temporal(sentences: &[&Sentence]) -> Vec<TemporalEvent> {
    extract_temporal_with_anchor(sentences, None)
}

pub fn extract_temporal_with_anchor(
    sentences: &[&Sentence],
    anchor_year: Option<i32>,
) -> Vec<TemporalEvent> {
    if sentences.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut occurrence: HashMap<usize, usize> = HashMap::new();
    let mut narration_order = 0usize;
    for s in sentences.iter() {
        // Byte offset of the sentence inside its SCENE text: the surface
        // spans below are scene-relative, so the planner can join them to
        // voiceover word timing on the same coordinate system.
        let sentence_text_base = s.start_byte;
        let next_occurrence = |occurrence: &mut HashMap<usize, usize>| {
            let n = occurrence.entry(s.id).or_insert(0);
            let id = *n;
            *n += 1;
            id
        };
        for (start, end, expr, rel_start, rel_end) in scan_interval_spans(&s.text) {
            let occurrence_id = next_occurrence(&mut occurrence);
            let (sb, eb) = absolute_surface_span(sentence_text_base, rel_start, rel_end);
            out.push(TemporalEvent {
                event_id: format!("event-{:04}:{:02}", s.id, occurrence_id),
                text_span: s.text.clone(),
                temporal_expression: expr,
                normalized_year: Some(start),
                normalized_end_year: Some(end),
                certainty: 0.9,
                narration_index: s.id,
                narration_order,
                start_byte: Some(sb),
                end_byte: Some(eb),
            });
            narration_order += 1;
        }
        let covered = interval_years(&s.text);
        for (year, rel_start, rel_end) in scan_year_spans(&s.text) {
            if covered.contains(&year) {
                continue;
            }
            let occurrence_id = next_occurrence(&mut occurrence);
            let (sb, eb) = absolute_surface_span(sentence_text_base, rel_start, rel_end);
            out.push(TemporalEvent {
                event_id: format!("event-{:04}:{:02}", s.id, occurrence_id),
                text_span: s.text.clone(),
                temporal_expression: year.to_string(),
                normalized_year: Some(year),
                normalized_end_year: None,
                certainty: 0.95,
                narration_index: s.id,
                narration_order,
                start_byte: Some(sb),
                end_byte: Some(eb),
            });
            narration_order += 1;
        }
        if let Some((off, rel_start, rel_end)) = relative_offset_span(s.token_index(), &s.text) {
            let resolved = anchor_year.map(|a| a + off);
            let occurrence_id = next_occurrence(&mut occurrence);
            let (sb, eb) = absolute_surface_span(sentence_text_base, rel_start, rel_end);
            out.push(TemporalEvent {
                event_id: format!("event-{:04}:{:02}-rel", s.id, occurrence_id),
                text_span: s.text.clone(),
                temporal_expression: relative_expression(&s.text),
                normalized_year: resolved,
                normalized_end_year: None,
                certainty: if resolved.is_some() { 0.75 } else { 0.5 },
                narration_index: s.id,
                narration_order,
                start_byte: Some(sb),
                end_byte: Some(eb),
            });
            narration_order += 1;
            continue;
        }
    }
    out.sort_by(|a, b| {
        a.normalized_year
            .cmp(&b.normalized_year)
            .then(a.narration_index.cmp(&b.narration_index))
            .then(a.narration_order.cmp(&b.narration_order))
    });
    out
}

/// Surface span: scene-relative absolute byte offsets.
fn absolute_surface_span(sentence_base: usize, rel_start: usize, rel_end: usize) -> (usize, usize) {
    (sentence_base + rel_start, sentence_base + rel_end)
}

fn scan_year_spans(text: &str) -> Vec<(i32, usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let j = {
                let mut k = i;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                k
            };
            if j - i == 4 {
                let left_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
                let right_ok = j >= bytes.len() || !bytes[j].is_ascii_digit();
                if left_ok && right_ok {
                    if let Ok(year) = text[i..j].parse::<i32>() {
                        if (1000..=2100).contains(&year) {
                            out.push((year, i, j));
                        }
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

pub(super) fn scan_years(text: &str) -> Vec<i32> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<i32>| {
        if cur.len() == 4 {
            if let Ok(y) = cur.parse::<i32>() {
                if (1000..=2100).contains(&y) {
                    out.push(y);
                }
            }
        }
        cur.clear();
    };
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() {
            cur.push(ch);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub(super) fn number_word(t: &str) -> Option<i32> {
    // Number words ("tre" → 3) live in the shared lexicon; digits and any
    // other numeric token parse directly inside the documented 1..=50 range.
    if let Some(entry) = semantic_core::lexicon::lexicon()
        .discourse
        .number_words
        .iter()
        .find(|entry| entry.words.iter().any(|word| word == t))
    {
        return Some(entry.value);
    }
    t.parse::<i32>().ok().filter(|n| (1..=50).contains(n))
}

/// Offset in years for relative expressions WITH the in-sentence byte span
/// of the matched marker ("tre anni dopo" → (+3, span)). None when no
/// relative expression.
pub(super) fn relative_offset_span(index: &TokenIndex, text: &str) -> Option<(i32, usize, usize)> {
    let toks = index.tokens();
    let cues = &semantic_core::lexicon::lexicon().discourse;
    let rel_plural = &cues.relative_plural;
    // One membership index per call: these cue tables are set questions.
    let rel_single: std::collections::HashSet<&str> =
        cues.relative_single.iter().map(String::as_str).collect();
    let year_units: std::collections::HashSet<&str> =
        cues.year_units.iter().map(String::as_str).collect();
    // Token→char boundary approximation: tokens are lowercase words built
    // over the same text, so reconstruct the span by searching the surface.
    let lower = text.to_lowercase();
    let span_of = |toks: &[String], from: usize, to: usize| -> (usize, usize) {
        let joined = toks[from..to.min(toks.len())].join(" ");
        lower
            .find(&joined)
            .map(|pos| (pos, pos + joined.len()))
            .unwrap_or((0, text.len()))
    };
    for pat in rel_plural {
        for i in 0..toks.len().saturating_sub(pat.len() - 1) {
            if toks[i..i + pat.len()] == pat[..] {
                let mut offset = None;
                // Number word just before the pattern, else +1 year.
                if i > 0 {
                    offset = number_word(&toks[i - 1]);
                }
                let span = span_of(toks, i, i + pat.len());
                return Some((offset.unwrap_or(1), span.0, span.1));
            }
        }
    }
    if let Some(win) = toks
        .windows(2)
        .position(|w| year_units.contains(w[0].as_str()) && rel_single.contains(w[1].as_str()))
    {
        let span = span_of(toks, win, win + 2);
        return Some((1, span.0, span.1));
    }
    for pattern in &cues.relative_fixed {
        if !index.contains_phrase(pattern) {
            continue;
        }
        if let Some(pos) = toks
            .windows(pattern.len())
            .position(|w| w.iter().zip(pattern.iter()).all(|(a, b)| a == b))
        {
            let span = span_of(toks, pos, pos + pattern.len());
            return Some((3, span.0, span.1));
        }
        return Some((3, 0, text.len()));
    }
    None
}

/// Token-window equality against a marker phrase. Generic over the marker
/// storage so the same matcher serves lexicon tables and inline literals.
pub(super) fn window_eq<S: AsRef<str>>(toks: &[String], pos: usize, marker: &[S]) -> bool {
    pos + marker.len() <= toks.len()
        && toks[pos..pos + marker.len()]
            .iter()
            .zip(marker.iter())
            .all(|(a, b)| a.as_str() == b.as_ref())
}

/// Intervals with byte spans: (start, end, expression, in-sentence start,
/// in-sentence end). The span covers exactly the digits+separator surface.
pub(super) fn scan_interval_spans(text: &str) -> Vec<(i32, i32, String, usize, usize)> {
    scan_intervals(text).into_iter().collect()
}

/// Intervals ("2020–2023", "dal 2020 al 2023"): (start, end, expression,
/// in-sentence start byte, in-sentence end byte).
pub(super) fn scan_intervals(text: &str) -> Vec<(i32, i32, String, usize, usize)> {
    // Work on char indices; years here are ASCII so byte/char coincide.
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut spans: Vec<(usize, usize, i32)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j - i == 4 {
                if let Ok(y) = text[i..j].parse::<i32>() {
                    if (1000..=2100).contains(&y) {
                        // Boundary check: not embedded in a longer digit run.
                        let left_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
                        let right_ok = j >= bytes.len() || !bytes[j].is_ascii_digit();
                        if left_ok && right_ok {
                            spans.push((i, j, y));
                        }
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let mut used = vec![false; spans.len()];
    for a in 0..spans.len() {
        if used[a] {
            continue;
        }
        if a + 1 < spans.len() {
            let (s0, e0, y0) = spans[a];
            let (s1, e1, y1) = spans[a + 1];
            let between = text[e0..s1].trim().to_lowercase();
            let is_sep = between == "-"
                || between == "–"
                || between == "—"
                || semantic_core::lexicon::lexicon()
                    .discourse
                    .interval_connectives
                    .contains(&between);
            if is_sep && y1 >= y0 {
                let trimmed_start = s0 + text[s0..e1].len() - text[s0..e1].trim_start().len();
                let trimmed_end = e1 - (text[s0..e1].len() - text[s0..e1].trim_end().len());
                out.push((
                    y0,
                    y1,
                    text[s0..e1].trim().to_string(),
                    trimmed_start,
                    trimmed_end,
                ));
                used[a] = true;
                used[a + 1] = true;
            }
        }
    }
    out
}

/// Years covered by intervals (suppressed as bare singles).
pub(super) fn interval_years(text: &str) -> std::collections::BTreeSet<i32> {
    scan_intervals(text)
        .into_iter()
        .flat_map(|(a, b, _, _, _)| [a, b])
        .collect()
}

pub(super) fn relative_expression(text: &str) -> String {
    let lower = text.to_lowercase();
    for cand in &semantic_core::lexicon::lexicon()
        .discourse
        .relative_expressions
    {
        if lower.contains(cand.as_str()) {
            return cand.clone();
        }
    }
    "relative".to_string()
}
