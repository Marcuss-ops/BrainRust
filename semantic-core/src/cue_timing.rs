//! Deterministic subtitle/cue kernel.
//!
//! This is the Rust authority for the pure text-and-time work that used to
//! live in `refactored/internal/capabilities/assets/texttracks`:
//!
//! - short-form caption normalization (split, merge, gap closing);
//! - full-text → cue distribution that preserves source timing;
//! - ASS line wrapping, content compilation and structural validation;
//! - VTT/SRT cue extraction from file content.
//!
//! The Go layer keeps file system, Drive publication, database and job
//! orchestration. Nothing here touches I/O, wall-clock time or randomness:
//! identical cues + policy always produce identical bytes.

use serde::{Deserialize, Serialize};

/// One timed caption. Mirrors `detail.TimedCue` on the Go side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedCue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Readability contract applied before cues are compiled into an ASS track.
///
/// A zero field means "use the canonical default", matching the Go
/// `withDefaults` behaviour: callers can send a partial policy.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortFormPolicy {
    pub max_cps: f64,
    pub min_cue_ms: i64,
    pub max_chars_per_line: usize,
    pub max_lines: usize,
    pub max_gap_merge_ms: i64,
}

impl Default for ShortFormPolicy {
    fn default() -> Self {
        Self {
            max_cps: 22.0,
            min_cue_ms: 700,
            max_chars_per_line: ASS_WRAP_CHARS,
            max_lines: 2,
            max_gap_merge_ms: 400,
        }
    }
}

/// The single owner of the ASS line width. The short-form normalizer budgets
/// captions against the same width so a normalized cue can never re-wrap into
/// more lines than the policy allows.
pub const ASS_WRAP_CHARS: usize = 40;

impl ShortFormPolicy {
    /// Fills every non-positive field with the canonical default.
    pub fn with_defaults(self) -> Self {
        let d = Self::default();
        Self {
            max_cps: if self.max_cps <= 0.0 {
                d.max_cps
            } else {
                self.max_cps
            },
            min_cue_ms: if self.min_cue_ms <= 0 {
                d.min_cue_ms
            } else {
                self.min_cue_ms
            },
            max_chars_per_line: if self.max_chars_per_line == 0 {
                d.max_chars_per_line
            } else {
                self.max_chars_per_line
            },
            max_lines: if self.max_lines == 0 {
                d.max_lines
            } else {
                self.max_lines
            },
            max_gap_merge_ms: if self.max_gap_merge_ms <= 0 {
                d.max_gap_merge_ms
            } else {
                self.max_gap_merge_ms
            },
        }
    }

    pub fn max_chars_per_cue(&self) -> usize {
        self.max_chars_per_line * self.max_lines
    }

    /// Time this text needs to be read, floored at `min_cue_ms`.
    fn reading_ms(&self, chars: usize) -> i64 {
        let need = (chars as f64 * 1000.0 / self.max_cps + 0.5) as i64;
        need.max(self.min_cue_ms)
    }

    /// Whether a caption asks for more dwell time than its text needs. A
    /// single line is never split: it reads in one glance and must stay for as
    /// long as it is spoken.
    fn needs_split(&self, text: &str, duration_ms: i64) -> bool {
        let chars = text.chars().count();
        if chars <= self.max_chars_per_line {
            return false;
        }
        duration_ms > self.reading_ms(chars)
    }
}

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
fn split_cue(cue: &TimedCue, p: &ShortFormPolicy) -> Vec<TimedCue> {
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
fn rebalance_min_durations(mut pieces: Vec<TimedCue>, p: &ShortFormPolicy) -> Vec<TimedCue> {
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

fn any_piece_needs_split(
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
fn chunk_text(text: &str, p: &ShortFormPolicy) -> Vec<String> {
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

fn wrapped_lines(text: &str, max_chars: usize) -> usize {
    let wrapped = wrap_asst_text(text, max_chars);
    if wrapped.is_empty() {
        return 0;
    }
    wrapped.matches("\\N").count() + 1
}

/// Splits every piece that carries more than one word, producing a finer
/// partition at word boundaries. Returns `None` when no piece can be split
/// further.
fn subdivide(pieces: &[String]) -> Option<Vec<String>> {
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
fn merge_short_pieces(pieces: Vec<TimedCue>, p: &ShortFormPolicy) -> Vec<TimedCue> {
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

fn merge_caption(a: &TimedCue, b: &TimedCue, p: &ShortFormPolicy) -> Option<TimedCue> {
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
fn close_small_gaps(mut cues: Vec<TimedCue>, p: &ShortFormPolicy) -> Vec<TimedCue> {
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

/// Wraps subtitle text to `max_runes` per line using ASS `\N` breaks.
pub fn wrap_asst_text(text: &str, max_runes: usize) -> String {
    if max_runes == 0 || text.chars().count() <= max_runes {
        return text.to_string();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return text.to_string();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in words {
        if word.chars().count() > max_runes {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let runes: Vec<char> = word.chars().collect();
            let mut offset = 0usize;
            while runes.len() - offset > max_runes {
                lines.push(runes[offset..offset + max_runes].iter().collect());
                offset += max_runes;
            }
            line = runes[offset..].iter().collect();
            continue;
        }
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if candidate.chars().count() > max_runes {
            lines.push(line);
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.join("\\N")
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontPreset {
    pub font_name: &'static str,
    pub font_size: i32,
    pub bold: i32,
    pub outline: f64,
    pub shadow: i32,
    pub margin_v: i32,
}

const PRESET_MONTSERRAT: FontPreset = FontPreset {
    font_name: "Montserrat",
    font_size: 58,
    bold: 1,
    outline: 4.0,
    shadow: 3,
    margin_v: 36,
};
const PRESET_POPPINS: FontPreset = FontPreset {
    font_name: "Poppins",
    font_size: 58,
    bold: 1,
    outline: 4.0,
    shadow: 6,
    margin_v: 36,
};
const PRESET_IMPACT: FontPreset = FontPreset {
    font_name: "Impact",
    font_size: 62,
    bold: 0,
    outline: 4.0,
    shadow: 3,
    margin_v: 36,
};
const PRESET_ANTON: FontPreset = FontPreset {
    font_name: "Anton",
    font_size: 60,
    bold: 0,
    outline: 4.0,
    shadow: 3,
    margin_v: 36,
};
const PRESET_BEBAS: FontPreset = FontPreset {
    font_name: "Bebas Neue",
    font_size: 64,
    bold: 0,
    outline: 4.0,
    shadow: 3,
    margin_v: 36,
};
const PRESET_ROBOTO: FontPreset = FontPreset {
    font_name: "Roboto",
    font_size: 56,
    bold: 1,
    outline: 3.5,
    shadow: 3,
    margin_v: 36,
};
const PRESET_YOUNG: FontPreset = FontPreset {
    font_name: "Poppins",
    font_size: 60,
    bold: 1,
    outline: 4.0,
    shadow: 5,
    margin_v: 40,
};
const PRESET_YOUNG_POP: FontPreset = FontPreset {
    font_name: "Poppins",
    font_size: 64,
    bold: 1,
    outline: 5.0,
    shadow: 7,
    margin_v: 44,
};
const PRESET_YOUNG_CLEAN: FontPreset = FontPreset {
    font_name: "Montserrat",
    font_size: 56,
    bold: 1,
    outline: 3.0,
    shadow: 2,
    margin_v: 40,
};
const PRESET_YOUNG_CENTER: FontPreset = FontPreset {
    font_name: "Poppins",
    font_size: 60,
    bold: 1,
    outline: 4.0,
    shadow: 5,
    margin_v: 40,
};

/// Maps a style ID or preset name to typography parameters.
///
/// The "Young" family is checked before the plain presets and
/// most-specific-first so `subs-young-pop` can never silently fall back to the
/// base `subs-young` typography.
pub fn resolve_font_preset(style_id: &str) -> FontPreset {
    let norm = style_id.trim().to_lowercase();
    if norm.contains("young-pop") {
        return PRESET_YOUNG_POP;
    }
    if norm.contains("young-clean") {
        return PRESET_YOUNG_CLEAN;
    }
    if norm.contains("young-center") {
        return PRESET_YOUNG_CENTER;
    }
    if norm.contains("young") {
        return PRESET_YOUNG;
    }
    if norm.contains("impact") {
        return PRESET_IMPACT;
    }
    if norm.contains("anton") {
        return PRESET_ANTON;
    }
    if norm.contains("bebas") {
        return PRESET_BEBAS;
    }
    if norm.contains("roboto") {
        return PRESET_ROBOTO;
    }
    if norm.contains("poppins") {
        return PRESET_POPPINS;
    }
    PRESET_MONTSERRAT
}

/// Accounts for ASS centisecond precision and guarantees a strictly positive
/// window.
fn normalize_ass_window(start_ms: i64, end_ms: i64) -> (i64, i64) {
    let mut start = start_ms.max(0);
    let mut end = end_ms.max(start);
    start = (start / 10) * 10;
    end = (end / 10) * 10;
    if end <= start {
        end = start + 10;
    }
    (start, end)
}

fn format_ass_time(ms: i64) -> String {
    let ms = ms.max(0);
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let c = (ms % 1000) / 10;
    format!("{h}:{m:02}:{s:02}.{c:02}")
}

/// Canonical, deterministic ASS content generator.
///
/// Identical cues + style always produce identical bytes; it generates no
/// absolute paths. Fail-closed: empty cues are a typed error, never an empty
/// placeholder track.
pub fn compile_ass_content(cues: &[TimedCue], style_id: &str) -> Result<String, String> {
    if cues.is_empty() {
        return Err("ass_materializer: timed_cues is empty".to_string());
    }
    let style_id = if style_id.is_empty() {
        "vidrush-default"
    } else {
        style_id
    };
    let preset = resolve_font_preset(style_id);
    let mut out = String::new();
    out.push_str("[Script Info]\n");
    out.push_str("Title: PipelineGen Auto Subtitles\n");
    out.push_str("ScriptType: v4.00+\n");
    out.push_str("WrapStyle: 0\n");
    out.push_str("PlayResX: 1920\n");
    out.push_str("PlayResY: 1080\n\n");
    out.push_str("[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    let alignment = if style_id.trim().to_lowercase().contains("center") {
        5
    } else {
        2
    };
    out.push_str(&format!(
        "Style: {style_id},{font},{size},&H00FFFFFF,&H000000FF,&H00000000,&H00000000,{bold},0,0,0,100,100,0,0,1,{outline:.1},{shadow},{alignment},10,10,{margin},1\n\n",
        font = preset.font_name,
        size = preset.font_size,
        bold = preset.bold,
        outline = preset.outline,
        shadow = preset.shadow,
        margin = preset.margin_v,
    ));
    out.push_str("[Events]\n");
    out.push_str(
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );

    let mut last_end_ms = 0i64;
    for cue in cues {
        let (mut start_ms, mut end_ms) = normalize_ass_window(cue.start_ms, cue.end_ms);
        if start_ms < last_end_ms {
            start_ms = last_end_ms;
        }
        if end_ms <= start_ms {
            end_ms = start_ms + 10;
        }
        let mut text = cue.text.replace('\n', " ");
        text = text.replace('\r', "");
        let text = wrap_asst_text(&text, ASS_WRAP_CHARS);
        out.push_str(&format!(
            "Dialogue: 0,{},{},{},,0,0,0,,{}\n",
            format_ass_time(start_ms),
            format_ass_time(end_ms),
            style_id,
            text
        ));
        last_end_ms = end_ms;
    }
    Ok(out)
}

/// Structural validator used before an ASS artifact is sealed into a plan.
pub fn validate_ass_content(content: &str, clip_duration_ms: i64) -> Result<(), String> {
    let mut has_script_info = false;
    let mut has_styles = false;
    let mut has_events = false;
    let mut has_dialogue = false;
    let mut last_end_ms = -1i64;

    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let line_num = idx + 1;
        match line {
            "[Script Info]" => has_script_info = true,
            "[V4+ Styles]" => has_styles = true,
            "[Events]" => has_events = true,
            _ => {
                if line.starts_with("Dialogue:") {
                    has_dialogue = true;
                    // Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text.
                    let full: Vec<&str> = line.splitn(10, ',').collect();
                    if full.len() < 10 {
                        return Err(format!("malformed Dialogue line at {line_num}"));
                    }
                    let start_str = full[1];
                    let end_str = full[2];
                    let text = full[9];
                    if text.trim().is_empty() {
                        return Err(format!("empty Dialogue text at line {line_num}"));
                    }
                    let start_ms = parse_ass_time(start_str).map_err(|e| {
                        format!("invalid start time {start_str:?} at line {line_num}: {e}")
                    })?;
                    let end_ms = parse_ass_time(end_str).map_err(|e| {
                        format!("invalid end time {end_str:?} at line {line_num}: {e}")
                    })?;
                    if start_ms < 0 || end_ms < 0 {
                        return Err(format!("negative timestamp at line {line_num}"));
                    }
                    if start_ms >= end_ms {
                        return Err(format!("start time >= end time at line {line_num}"));
                    }
                    if start_ms < last_end_ms {
                        return Err(format!("cues not sorted or overlap at line {line_num}"));
                    }
                    last_end_ms = end_ms;
                }
            }
        }
    }

    if !has_script_info {
        return Err("missing [Script Info] section".to_string());
    }
    if !has_styles {
        return Err("missing [V4+ Styles] section".to_string());
    }
    if !has_events {
        return Err("missing [Events] section".to_string());
    }
    if !has_dialogue {
        return Err("missing Dialogue lines".to_string());
    }
    if clip_duration_ms > 0 && last_end_ms > clip_duration_ms + 250 {
        return Err(format!(
            "last cue end ({last_end_ms} ms) exceeds clip duration ({clip_duration_ms} ms + 250ms tolerance)"
        ));
    }
    Ok(())
}

fn parse_ass_time(value: &str) -> Result<i64, String> {
    let mut parts = value.split([':', '.']);
    let h: i64 = parts
        .next()
        .ok_or("missing hours")?
        .trim()
        .parse()
        .map_err(|_| "invalid hours".to_string())?;
    let m: i64 = parts
        .next()
        .ok_or("missing minutes")?
        .trim()
        .parse()
        .map_err(|_| "invalid minutes".to_string())?;
    let s: i64 = parts
        .next()
        .ok_or("missing seconds")?
        .trim()
        .parse()
        .map_err(|_| "invalid seconds".to_string())?;
    let c: i64 = parts
        .next()
        .ok_or("missing centiseconds")?
        .trim()
        .parse()
        .map_err(|_| "invalid centiseconds".to_string())?;
    if parts.next().is_some() {
        return Err("trailing timestamp components".to_string());
    }
    Ok(h * 3_600_000 + m * 60_000 + s * 1000 + c * 10)
}

/// Splits a cue timing line only when the arrow is whitespace-delimited, like
/// the Go parser's `\s+-->\s+` contract.
fn split_cue_arrow(line: &str) -> Option<(&str, &str)> {
    let idx = line.find("-->")?;
    let before = &line[..idx];
    let after = &line[idx + 3..];
    let whitespace_before = before.chars().last().is_some_and(char::is_whitespace);
    let whitespace_after = after.chars().next().is_some_and(char::is_whitespace);
    (whitespace_before && whitespace_after).then_some((before, after))
}

fn parse_timestamp(value: &str) -> Option<i64> {
    let normalized = value.trim().replace(',', ".");
    let mut parts = normalized.split(':');
    let h = parts.next()?;
    let m = parts.next()?;
    let rest = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    // The Go regex is `\d{1,2}:\d{2}:\d{2}[.,]\d{3}`: hours 1-2 digits,
    // minutes/seconds exactly two, milliseconds exactly three.
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (s, ms) = rest.split_once('.')?;
    if s.len() != 2 || ms.len() != 3 {
        return None;
    }
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    let s: i64 = s.parse().ok()?;
    let ms: i64 = ms.parse().ok()?;
    Some(h * 3_600_000 + m * 60_000 + s * 1000 + ms)
}

/// Extracts `(plain_text, cues)` from VTT or SRT content.
///
/// The format is chosen from the filename extension (`.srt` → SRT, anything
/// else → VTT). A malformed file returns a typed error; individual cues with
/// invalid timing are dropped, matching the Go parser.
pub fn parse_subtitle_text(
    filename: &str,
    content: &str,
) -> Result<(String, Vec<TimedCue>), String> {
    let is_srt = filename.to_lowercase().ends_with(".srt");
    let mut cues: Vec<TimedCue> = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();
    let mut current: Option<TimedCue> = None;

    for raw in content.lines() {
        let line = raw.trim();
        if !is_srt && line.starts_with("WEBVTT") {
            continue;
        }
        if is_srt && line.parse::<i64>().is_ok() {
            if let Some(cue) = current.take() {
                text_parts.push(cue.text.clone());
                cues.push(cue);
            }
            continue;
        }
        if let Some((start, end)) = split_cue_arrow(line) {
            let (Some(start_ms), Some(end_ms)) = (parse_timestamp(start), parse_timestamp(end))
            else {
                continue;
            };
            if let Some(cue) = current.take() {
                text_parts.push(cue.text.clone());
                cues.push(cue);
            }
            current = Some(TimedCue {
                start_ms,
                end_ms,
                text: String::new(),
            });
            continue;
        }
        if line.is_empty() {
            if let Some(cue) = current.take() {
                text_parts.push(cue.text.clone());
                cues.push(cue);
            }
            continue;
        }
        if let Some(cue) = current.as_mut() {
            if !cue.text.is_empty() {
                cue.text.push('\n');
            }
            cue.text.push_str(line);
        }
    }
    if let Some(cue) = current.take() {
        text_parts.push(cue.text.clone());
        cues.push(cue);
    }
    if cues.is_empty() {
        return Err(format!(
            "texttracks.ParseSubtitleFile: no parseable cues in {filename:?}"
        ));
    }
    Ok((text_parts.join("\n"), cues))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(start_ms: i64, end_ms: i64, text: &str) -> TimedCue {
        TimedCue {
            start_ms,
            end_ms,
            text: text.to_string(),
        }
    }

    fn words_of(cues: &[TimedCue]) -> Vec<String> {
        cues.iter()
            .flat_map(|c| {
                c.text
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn assert_readable(policy: ShortFormPolicy, out: &[TimedCue]) {
        for (i, c) in out.iter().enumerate() {
            let dur = c.end_ms - c.start_ms;
            assert!(dur > 0, "caption {i} has an empty window: {c:?}");
            let lines = wrapped_lines(&c.text, policy.max_chars_per_line);
            assert!(
                lines <= policy.max_lines,
                "caption {i} wraps to {lines} lines: {c:?}"
            );
            let chars = c.text.chars().count();
            if chars > policy.max_chars_per_line {
                assert!(
                    dur <= policy.reading_ms(chars),
                    "caption {i} dwells {dur}ms for {chars} chars: {c:?}"
                );
            }
            if i > 0 {
                assert!(
                    c.start_ms >= out[i - 1].end_ms,
                    "captions {}/{} overlap",
                    i - 1,
                    i
                );
            }
        }
    }

    #[test]
    fn splits_long_whisper_segment() {
        let source = vec![cue(
            13040,
            20480,
            "this is fine and I did it and but I was in Dubai in 2004 and went to the top of this building",
        )];
        let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
        assert!(
            out.len() >= 2,
            "long segment produced {} captions",
            out.len()
        );
        assert_readable(ShortFormPolicy::default(), &out);
        for c in &out {
            assert!(
                c.start_ms >= 13040 && c.end_ms <= 20480,
                "escaped window: {c:?}"
            );
        }
        assert_eq!(
            words_of(&out).join(" "),
            source[0]
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    #[test]
    fn preserves_readable_transcript() {
        let source = vec![
            cue(0, 2000, "Okay, no."),
            cue(2000, 4000, "I'm on the go."),
            cue(4000, 5000, "How does that even work?"),
        ];
        let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
        assert_eq!(out, source);
    }

    #[test]
    fn closes_sub_second_gap() {
        let source = vec![
            cue(0, 3000, "first caption here"),
            cue(3200, 6000, "second caption here"),
        ];
        let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].end_ms, out[1].start_ms);
    }

    #[test]
    fn keeps_real_silence() {
        let source = vec![
            cue(0, 3000, "first caption here"),
            cue(6200, 9000, "second caption here"),
        ];
        let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
        assert_eq!(out.len(), 2);
        assert!(
            out[0].end_ms < out[1].start_ms,
            "a 3.2s silence must stay silent"
        );
    }

    #[test]
    fn real_whisper_segments_stay_readable() {
        let source = vec![
            cue(0, 4800, "Where he can tell what colored underwear a man is wearing just from just from looking at"),
            cue(5440, 12920, "You do not disappoint. Oh, man. Listen, and this is any scraping the surface, buddy. This is okay"),
            cue(16120, 18800, "Should I do me to do me to do it to you?"),
            cue(18800, 23720, "I have so many I have so many just you facing the wrong way on a toilet"),
            cue(23720, 30799, "You just naked in an empty bathtub. It's great. I am just just the water way to start the day the water slowly running around"),
        ];
        let policy = ShortFormPolicy::default();
        let out = normalize_short_form_cues(&source, policy);
        assert!(
            out.len() >= source.len() * 2,
            "got {} captions from {} segments",
            out.len(),
            source.len()
        );
        assert_readable(policy, &out);
        assert_eq!(words_of(&out).join(" "), words_of(&source).join(" "));
    }

    #[test]
    fn normalization_is_deterministic() {
        let source = vec![
            cue(0, 5200, "where he can tell what colored underwear a man is wearing just from looking at him"),
            cue(5440, 12920, "you do not disappoint oh man listen and this is any scraping the surface"),
        ];
        let first = normalize_short_form_cues(&source, ShortFormPolicy::default());
        let second = normalize_short_form_cues(&source, ShortFormPolicy::default());
        assert_eq!(first, second);
    }

    #[test]
    fn merges_sub_second_caption() {
        let source = vec![cue(0, 1000, "first"), cue(1100, 1300, "flash")];
        let out = normalize_short_form_cues(&source, ShortFormPolicy::default());
        assert_eq!(out.len(), 1, "expected merge: {out:?}");
        assert_eq!(out[0].text, "first flash");
        assert_eq!((out[0].start_ms, out[0].end_ms), (0, 1300));
    }

    #[test]
    fn merged_captions_stay_bounded() {
        let source = vec![
            cue(0, 1000, "first"),
            cue(1100, 1300, "flash"),
            cue(1300, 9000, "a much longer utterance that definitely splits into several captions because it keeps going"),
            cue(9100, 9200, "tiny"),
            cue(10000, 15000, "and then a closing line that also needs a couple of captions"),
        ];
        let policy = ShortFormPolicy::default();
        let out = normalize_short_form_cues(&source, policy);
        assert!(!out.is_empty());
        assert_eq!(out[0].start_ms, source[0].start_ms);
        assert!(out.last().unwrap().end_ms <= source.last().unwrap().end_ms);
        for i in 1..out.len() {
            assert!(out[i].start_ms >= out[i - 1].end_ms);
        }
        assert_eq!(words_of(&out).join(" "), words_of(&source).join(" "));
        assert_readable(policy, &out);
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(normalize_short_form_cues(&[], ShortFormPolicy::default()).is_empty());
        let degenerate = vec![cue(0, 0, "x")];
        assert!(normalize_short_form_cues(&degenerate, ShortFormPolicy::default()).is_empty());
    }

    #[test]
    fn distributes_text_across_timing() {
        let timing = vec![cue(0, 1000, ""), cue(1000, 2000, "")];
        let out = cues_with_text(&timing, "one two three four");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "one two");
        assert_eq!(out[1].text, "three four");
        assert_eq!((out[1].start_ms, out[1].end_ms), (1000, 2000));
    }

    #[test]
    fn distribution_never_loses_words() {
        let timing = vec![cue(0, 10, ""), cue(10, 20, ""), cue(20, 30, "")];
        let out = cues_with_text(&timing, "alpha beta gamma delta epsilon");
        assert_eq!(
            out.iter()
                .map(|c| c.text.split_whitespace().count())
                .sum::<usize>(),
            5
        );
        assert_eq!(
            out.iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            "alpha beta gamma delta epsilon"
        );
    }

    #[test]
    fn ass_compilation_is_deterministic() {
        let cues = vec![cue(0, 3280, "hello"), cue(3280, 6000, "world")];
        let first = compile_ass_content(&cues, "shorts-v1").unwrap();
        let second = compile_ass_content(&cues, "shorts-v1").unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with("[Script Info]"));
        assert!(first.contains("Style: shorts-v1,"));
        assert_eq!(first.matches("Dialogue:").count(), 2);
        assert!(first.contains("Dialogue: 0,0:00:00.00,0:00:03.28,shorts-v1,,0,0,0,,hello"));
    }

    #[test]
    fn ass_style_changes_bytes() {
        let cues = vec![cue(0, 3280, "hello"), cue(3280, 6000, "world")];
        assert_ne!(
            compile_ass_content(&cues, "shorts-v1").unwrap(),
            compile_ass_content(&cues, "shorts-v2").unwrap()
        );
    }

    #[test]
    fn ass_empty_cues_fail_closed() {
        assert!(compile_ass_content(&[], "shorts-v1").is_err());
    }

    #[test]
    fn poppins_preset_has_stronger_shadow() {
        let content =
            compile_ass_content(&[cue(0, 3280, "hello")], "matt-damon-benchmark-v1-poppins")
                .unwrap();
        assert!(content.contains("Style: matt-damon-benchmark-v1-poppins,Poppins,58,"));
        assert!(content.contains(",1,0,0,0,100,100,0,0,1,4.0,6,"));
        let poppins = resolve_font_preset("matt-damon-benchmark-v1-poppins");
        let montserrat = resolve_font_preset("shorts-v1");
        assert_eq!(poppins.font_name, "Poppins");
        assert!(poppins.shadow > montserrat.shadow);
    }

    #[test]
    fn young_family_is_distinct_and_specific_first() {
        assert_eq!(resolve_font_preset("subs-young").font_size, 60);
        let pop = resolve_font_preset("subs-young-pop");
        assert_eq!((pop.font_size, pop.shadow, pop.margin_v), (64, 7, 44));
        let clean = resolve_font_preset("subs-young-clean");
        assert_eq!((clean.font_name, clean.font_size), ("Montserrat", 56));
        let center = resolve_font_preset("subs-young-center");
        assert_eq!(center.font_size, 60);
    }

    #[test]
    fn center_style_uses_screen_center_alignment() {
        let content = compile_ass_content(&[cue(0, 1000, "hi")], "shorts-young-center").unwrap();
        assert!(content.contains(",1,4.0,5,5,10,10,40,1"), "{content}");
    }

    #[test]
    fn ass_validator_accepts_generated_content() {
        let content = compile_ass_content(
            &[cue(0, 3280, "hello"), cue(3280, 6000, "world")],
            "shorts-v1",
        )
        .unwrap();
        assert!(validate_ass_content(&content, 6000).is_ok());
        assert!(validate_ass_content(&content, 1000).is_err());
        assert!(validate_ass_content("", 0).is_err());
    }

    #[test]
    fn ass_wrap_respects_width_and_never_drops_text() {
        let text =
            "a very long subtitle line that must wrap into several pieces without losing words";
        let wrapped = wrap_asst_text(text, 20);
        for line in wrapped.split("\\N") {
            assert!(line.chars().count() <= 20, "line too long: {line:?}");
        }
        assert_eq!(wrapped.replace("\\N", " "), text);
    }

    #[test]
    fn parses_vtt_cues() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nhello\nworld\n\n00:00:03.000 --> 00:00:04.000\nbye\n";
        let (plain, cues) = parse_subtitle_text("subs.vtt", vtt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[0].start_ms, cues[0].end_ms), (1000, 2500));
        assert_eq!(cues[0].text, "hello\nworld");
        assert_eq!(cues[1].text, "bye");
        assert_eq!(plain, "hello\nworld\nbye");
    }

    #[test]
    fn parses_srt_cues() {
        let srt =
            "1\n00:00:01,000 --> 00:00:02,000\nfirst\n\n2\n00:00:03,000 --> 00:00:04,000\nsecond\n";
        let (plain, cues) = parse_subtitle_text("subs.srt", srt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[1].text, "second");
        assert_eq!(plain, "first\nsecond");
    }

    #[test]
    fn parser_rejects_timestamps_go_would_not_match() {
        let malformed = "WEBVTT\n\n1:2:3.4-->5:6:7.8\nhello\n";
        assert!(parse_subtitle_text("bad.vtt", malformed).is_err());
        let no_millis = "WEBVTT\n\n00:00:01 --> 00:00:02\nhello\n";
        assert!(parse_subtitle_text("bad.vtt", no_millis).is_err());
    }

    #[test]
    fn malformed_subtitle_file_fails_closed() {
        assert!(parse_subtitle_text("empty.vtt", "WEBVTT\n\n").is_err());
    }
}
