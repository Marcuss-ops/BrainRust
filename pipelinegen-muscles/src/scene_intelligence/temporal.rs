//! Internal split of scene_intelligence (pure move, no behavior change).

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
    let mut out = Vec::new();
    for s in sentences {
        for (start, end, expr) in scan_intervals(&s.text) {
            out.push(TemporalEvent {
                event_id: format!("event-{:04}", s.id),
                text_span: s.text.clone(),
                temporal_expression: expr,
                normalized_year: Some(start),
                normalized_end_year: Some(end),
                certainty: 0.9,
                narration_index: s.id,
            });
        }
        let covered = interval_years(&s.text);
        for year in scan_years(&s.text) {
            if covered.contains(&year) {
                continue;
            }
            out.push(TemporalEvent {
                event_id: format!("event-{:04}", s.id),
                text_span: s.text.clone(),
                temporal_expression: year.to_string(),
                normalized_year: Some(year),
                normalized_end_year: None,
                certainty: 0.95,
                narration_index: s.id,
            });
        }
        if let Some(off) = relative_offset(&s.text) {
            let resolved = anchor_year.map(|a| a + off);
            out.push(TemporalEvent {
                event_id: format!("event-{:04}-rel", s.id),
                text_span: s.text.clone(),
                temporal_expression: relative_expression(&s.text),
                normalized_year: resolved,
                normalized_end_year: None,
                certainty: if resolved.is_some() { 0.75 } else { 0.5 },
                narration_index: s.id,
            });
            continue;
        }
    }
    out.sort_by(|a, b| {
        a.normalized_year
            .cmp(&b.normalized_year)
            .then(a.narration_index.cmp(&b.narration_index))
    });
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

/// Offset in years for relative expressions ("tre anni dopo" → +3,
/// "l'anno successivo" → +1). None when no relative expression.
pub(super) fn relative_offset(text: &str) -> Option<i32> {
    let toks = tokens(text);
    const REL_PLURAL: &[&[&str]] = &[
        &["anni", "dopo"],
        &["years", "later"],
        &["jahre", "später"],
        &["jahre", "spaeter"],
        &["ans", "plus", "tard"],
    ];
    const REL_SINGLE: &[&str] = &[
        "successivo",
        "successiva",
        "following",
        "suivante",
        "siguiente",
    ];
    for pat in REL_PLURAL {
        for i in 0..toks.len().saturating_sub(pat.len() - 1) {
            if toks[i..i + pat.len()] == **pat {
                // Number word or digit just before the pattern.
                if i > 0 {
                    if let Some(n) = number_word(&toks[i - 1]) {
                        return Some(n);
                    }
                }
                return Some(1);
            }
        }
    }
    if toks.windows(2).any(|w| {
        (w[0] == "anno" || w[0] == "year" || w[0] == "jahr" || w[0] == "an")
            && REL_SINGLE.contains(&w[1].as_str())
    }) {
        return Some(1);
    }
    // "N anni dopo" with N as prefix of the pattern already handled;
    // explicit "tre anni dopo" / "three years later".
    if contains_phrase(&toks, &["tre", "anni", "dopo"])
        || contains_phrase(&toks, &["three", "years", "later"])
    {
        return Some(3);
    }
    None
}

pub(super) fn number_word(t: &str) -> Option<i32> {
    match t {
        "un" | "uno" | "una" | "one" | "ein" | "eine" => Some(1),
        "due" | "two" | "zwei" | "deux" | "dos" => Some(2),
        "tre" | "three" | "drei" | "trois" | "tres" => Some(3),
        "quattro" | "four" | "vier" | "quatre" | "cuatro" => Some(4),
        "cinque" | "five" | "fünf" | "funf" | "cinq" | "cinco" => Some(5),
        _ => t.parse::<i32>().ok().filter(|n| (1..=50).contains(n)),
    }
}

/// Token-window equality against a marker phrase.
pub(super) fn window_eq(toks: &[String], pos: usize, marker: &[&str]) -> bool {
    pos + marker.len() <= toks.len()
        && toks[pos..pos + marker.len()]
            .iter()
            .zip(marker.iter())
            .all(|(a, b)| a == b)
}

/// Intervals ("2020–2023", "dal 2020 al 2023"): (start, end, expression).
pub(super) fn scan_intervals(text: &str) -> Vec<(i32, i32, String)> {
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
                || ["al", "to", "and", "e", "e il", "bis", "au", "a"].contains(&between.as_str());
            if is_sep && y1 >= y0 {
                out.push((y0, y1, text[s0..e1].trim().to_string()));
                used[a] = true;
                used[a + 1] = true;
            }
        }
    }
    let _ = used;
    out
}

/// Years covered by intervals (suppressed as bare singles).
pub(super) fn interval_years(text: &str) -> std::collections::BTreeSet<i32> {
    scan_intervals(text)
        .into_iter()
        .flat_map(|(a, b, _)| [a, b])
        .collect()
}

pub(super) fn relative_expression(text: &str) -> String {
    let lower = text.to_lowercase();
    for cand in [
        "tre anni dopo",
        "three years later",
        "anni dopo",
        "years later",
    ] {
        if lower.contains(cand) {
            return cand.to_string();
        }
    }
    "relative".to_string()
}

