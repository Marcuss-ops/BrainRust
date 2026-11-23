use super::*;

/// Keeps canonical source timing while projecting an already translated full
/// transcript onto the same cue count.
///
/// The source translation remains authoritative; this only distributes the
/// translated text across the source-language cue windows so every language
/// carries identical timing, order and segmentation.
pub fn cues_with_text(timing: &[TimedCue], text: &str) -> Vec<TimedCue> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let count = timing.len();
    if count == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count);
    for (i, cue) in timing.iter().enumerate() {
        let mut start = i.saturating_mul(words.len()) / count;
        let mut end = (i + 1).saturating_mul(words.len()) / count;
        if end <= start && start < words.len() {
            end = start + 1;
        }
        if start > words.len() {
            start = words.len();
        }
        if end > words.len() {
            end = words.len();
        }
        let mut projected = cue.clone();
        projected.text = words[start..end].join(" ");
        out.push(projected);
    }
    out
}

/// Rewrites cues into readable short-form captions.
///
/// An input with no usable cue window yields no captions; an input that is
/// usable but cannot be improved is returned unchanged. Captions are never
/// invented and a transcript is never dropped because normalization could not
/// help.
pub fn normalize_short_form_cues(cues: &[TimedCue], policy: ShortFormPolicy) -> Vec<TimedCue> {
    if cues.is_empty() {
        return Vec::new();
    }
    let p = policy.with_defaults();
    let mut out: Vec<TimedCue> = Vec::with_capacity(cues.len());
    let mut usable = 0usize;
    for cue in cues {
        if !cue.text.trim().is_empty() && cue.end_ms > cue.start_ms {
            usable += 1;
        }
        out.extend(split_cue(cue, &p));
    }
    if out.is_empty() {
        if usable == 0 {
            return Vec::new();
        }
        return cues.to_vec();
    }
    let out = merge_short_pieces(out, &p);
    close_small_gaps(out, &p)
}

/// Splits one source cue into caption-sized pieces whose windows partition the
/// source window in proportion to each piece's text length.
pub(super) fn split_cue(cue: &TimedCue, p: &ShortFormPolicy) -> Vec<TimedCue> {
    let text = cue.text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || cue.end_ms <= cue.start_ms {
        return Vec::new();
    }
    let window = cue.end_ms - cue.start_ms;
    let mut pieces = chunk_text(&text, p);
    if pieces.is_empty() {
        return Vec::new();
    }
    // A piece may still hold more than its share of the utterance (dense
    // speech). Subdivide until every piece is within its reading time or no
    // word boundary is left.
    loop {
        let total: usize = pieces.iter().map(|s| s.chars().count()).sum();
        if total == 0 {
            return vec![TimedCue {
                start_ms: cue.start_ms,
                end_ms: cue.end_ms,
                text,
            }];
        }
        if !any_piece_needs_split(&pieces, total, window, p) {
            break;
        }
        match subdivide(&pieces) {
            Some(next) => pieces = next,
            None => break,
        }
    }
    let total: usize = pieces.iter().map(|s| s.chars().count()).sum();
    if total == 0 {
        return vec![TimedCue {
            start_ms: cue.start_ms,
            end_ms: cue.end_ms,
            text,
        }];
    }
    let mut out = Vec::with_capacity(pieces.len());
    let mut consumed = 0usize;
    for (i, piece) in pieces.iter().enumerate() {
        let count = piece.chars().count();
        let start = if consumed > 0 {
            cue.start_ms + window * consumed as i64 / total as i64
        } else {
            cue.start_ms
        };
        let mut end = cue.end_ms;
        if i + 1 < pieces.len() {
            end = cue.start_ms + window * (consumed + count) as i64 / total as i64;
        }
        if end <= start {
            end = start + 10;
        }
        if end > cue.end_ms {
            end = cue.end_ms;
        }
        out.push(TimedCue {
            start_ms: start,
            end_ms: end,
            text: piece.clone(),
        });
        consumed += count;
    }
    rebalance_min_durations(out, p)
}

/// Lifts captions below `min_cue_ms` by moving shared boundaries against a
/// neighbour that can spare the time, keeping the exact partition.
pub(super) fn rebalance_min_durations(
    mut pieces: Vec<TimedCue>,
    p: &ShortFormPolicy,
) -> Vec<TimedCue> {
    if pieces.len() < 2 {
        return pieces;
    }
    for i in 0..pieces.len() {
        let mut deficit = p.min_cue_ms - (pieces[i].end_ms - pieces[i].start_ms);
        if deficit <= 0 {
            continue;
        }
        let neighbours = [Some(i + 1), i.checked_sub(1)];
        for n in neighbours.into_iter().flatten() {
            if deficit <= 0 || n >= pieces.len() {
                continue;
            }
            let spare = (pieces[n].end_ms - pieces[n].start_ms) - p.min_cue_ms;
            if spare <= 0 {
                continue;
            }
            let take = deficit.min(spare);
            if n > i {
                pieces[i].end_ms += take;
                pieces[n].start_ms += take;
            } else {
                pieces[i].start_ms -= take;
                pieces[n].end_ms -= take;
            }
            deficit -= take;
        }
    }
    pieces
}

pub(super) fn any_piece_needs_split(
    pieces: &[String],
    total: usize,
    window: i64,
    p: &ShortFormPolicy,
) -> bool {
    if total == 0 {
        return false;
    }
    pieces.iter().any(|s| {
        let share = window * s.chars().count() as i64 / total as i64;
        p.needs_split(s, share)
    })
}

/// Groups words into captions that wrap to at most `max_lines` lines. Line
/// counting reuses `wrap_asst_text` so the ASS output and this budget can never
/// disagree.
pub(super) fn chunk_text(text: &str, p: &ShortFormPolicy) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let max_chars = p.max_chars_per_cue();
    let mut pieces: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in words {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty()
            && (candidate.chars().count() > max_chars
                || wrapped_lines(&candidate, p.max_chars_per_line) > p.max_lines)
        {
            pieces.push(line);
            line = word.to_string();
            continue;
        }
        line = candidate;
    }
    if !line.is_empty() {
        pieces.push(line);
    }
    pieces
}

pub(super) fn wrapped_lines(text: &str, max_chars: usize) -> usize {
    let wrapped = wrap_asst_text(text, max_chars);
    if wrapped.is_empty() {
        return 0;
    }
    wrapped.matches("\\N").count() + 1
}

/// Splits every piece that carries more than one word, producing a finer
/// partition at word boundaries. Returns `None` when no piece can be split
/// further.
pub(super) fn subdivide(pieces: &[String]) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(pieces.len() * 2);
    let mut changed = false;
    for piece in pieces {
        let words: Vec<&str> = piece.split_whitespace().collect();
        if words.len() < 2 {
            out.push(piece.clone());
            continue;
        }
        let half = words.len() / 2;
        out.push(words[..half].join(" "));
        out.push(words[half..].join(" "));
        changed = true;
    }
    changed.then_some(out)
}

/// Folds pieces shorter than `min_cue_ms` into a neighbouring piece when the
/// merged text still fits both the character budget and the duration budget.
pub(super) fn merge_short_pieces(pieces: Vec<TimedCue>, p: &ShortFormPolicy) -> Vec<TimedCue> {
    let mut out: Vec<TimedCue> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        if piece.end_ms - piece.start_ms < p.min_cue_ms {
            if let Some(last) = out.last() {
                if let Some(merged) = merge_caption(last, &piece, p) {
                    if let Some(slot) = out.last_mut() {
                        *slot = merged;
                    }
                    continue;
                }
            }
        }
        out.push(piece);
    }
    // A trailing sub-second caption can only merge forward.
    let mut i = 0usize;
    while i + 1 < out.len() {
        if out[i].end_ms - out[i].start_ms >= p.min_cue_ms {
            i += 1;
            continue;
        }
        match merge_caption(&out[i], &out[i + 1], p) {
            Some(merged) => {
                out[i] = merged;
                out.remove(i + 1);
            }
            None => i += 1,
        }
    }
    out
}

pub(super) fn merge_caption(a: &TimedCue, b: &TimedCue, p: &ShortFormPolicy) -> Option<TimedCue> {
    if b.start_ms - a.end_ms > p.max_gap_merge_ms {
        // A caption is never stretched across a real silence.
        return None;
    }
    let mut text = a.text.clone();
    if text.trim().is_empty() {
        text = b.text.clone();
    } else if !b.text.trim().is_empty() {
        text = format!("{text} {}", b.text);
    }
    if text.chars().count() > p.max_chars_per_cue() {
        return None;
    }
    if wrapped_lines(&text, p.max_chars_per_line) > p.max_lines {
        return None;
    }
    Some(TimedCue {
        start_ms: a.start_ms,
        end_ms: b.end_ms,
        text,
    })
}

/// Removes sub-`max_gap_merge_ms` silences between adjacent captions so a
/// caption cannot blink off and back on.
pub(super) fn close_small_gaps(mut cues: Vec<TimedCue>, p: &ShortFormPolicy) -> Vec<TimedCue> {
    if p.max_gap_merge_ms <= 0 {
        return cues;
    }
    for i in 0..cues.len().saturating_sub(1) {
        let gap = cues[i + 1].start_ms - cues[i].end_ms;
        if gap <= 0 || gap > p.max_gap_merge_ms {
            continue;
        }
        cues[i].end_ms = cues[i + 1].start_ms;
    }
    cues
}
