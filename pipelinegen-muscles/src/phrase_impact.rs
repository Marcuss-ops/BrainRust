use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;

const GRAPH_K: usize = 6;
const LOCAL_CONTEXT: usize = 3;
const DAMPING: f64 = 0.85;
const MAX_SEGMENT_WORDS: usize = 48;
const DUPLICATE_COSINE: f64 = 0.96;
/// Longest keyphrase a title may quote. Five tokens is the overlay budget the
/// renderer treats as one visual line, so a longer span would not fit it.
const MAX_TITLE_TOKENS: usize = 5;
/// Score multiplier for a candidate span that covers a whole content run: the
/// span IS the phrase, so nothing was cut off it.
const WHOLE_RUN_SPAN: f64 = 1.0;
/// Score multiplier for the TAIL of a content run longer than the title budget.
/// Such a run cannot be quoted whole, so it must be trimmed, and the tail is
/// the only window of it that is ever recorded — see the run enumeration below.
/// It stays close to an intact phrase in score, so a complete short run still
/// outranks a trimmed one while neither can be outranked by a mid-phrase cut.
const TRIMMED_RUN_TAIL_SPAN: f64 = 0.9;
/// Score multiplier for a window cut out of a content run that DOES fit the
/// title budget: a sub-window of a short run is allowed (it may be the better
/// phrase) but ranks below the whole run.
const PARTIAL_SPAN: f64 = 0.5;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLength {
    Short,
    Medium,
    Long,
}

impl Default for SummaryLength {
    fn default() -> Self {
        Self::Medium
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Options {
    #[serde(default)]
    pub summary_length: SummaryLength,
    #[serde(default = "default_bullet_count")]
    pub bullet_count: usize,
    #[serde(default = "default_min_heavy")]
    pub min_heavy: usize,
    #[serde(default = "default_max_heavy")]
    pub max_heavy: usize,
    #[serde(default)]
    pub top_fraction: Option<f64>,
}

fn default_bullet_count() -> usize {
    5
}
fn default_min_heavy() -> usize {
    3
}
fn default_max_heavy() -> usize {
    15
}

impl Default for Options {
    fn default() -> Self {
        Self {
            summary_length: SummaryLength::default(),
            bullet_count: default_bullet_count(),
            min_heavy: default_min_heavy(),
            max_heavy: default_max_heavy(),
            top_fraction: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Timing {
    pub start_us: i64,
    pub end_us: i64,
}

/// Versioned editorial profile. Weights are explicit configuration, not
/// per-transcript decisions; tune them only against a separate annotated set.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ChapterOptions {
    pub profile_version: String,
    pub min_sentences: usize,
    pub max_sentences: usize,
    pub min_words: usize,
    pub context_sentences: usize,
    pub semantic_weight: f64,
    pub lexical_weight: f64,
    pub scene_weight: f64,
    pub evidence_weight: f64,
    pub complexity_penalty: f64,
    pub bullet_count: usize,
}

impl Default for ChapterOptions {
    fn default() -> Self {
        Self {
            profile_version: "segmentation.v1".into(),
            min_sentences: 5,
            max_sentences: 36,
            min_words: 90,
            context_sentences: 3,
            semantic_weight: 1.0,
            lexical_weight: 1.0,
            scene_weight: 0.2,
            evidence_weight: 1.0,
            complexity_penalty: 1.0,
            bullet_count: 3,
        }
    }
}

/// Verified scene identity for per-scene highlights (editorial.v1).
/// start_byte/end_byte locate the scene verbatim inside transcript when known;
/// None means local-only analysis (no global offsets fabricated).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SceneInput {
    pub scene_id: String,
    pub text: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub start_byte: Option<usize>,
    #[serde(default)]
    pub end_byte: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SceneBulletSpan {
    pub sentence_start: usize,
    pub sentence_end: usize,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScenePhraseSpan {
    pub sentence_index: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
    pub score: f64,
    /// True when the span fits the visual overlay budget (<=5 tokens).
    /// Longer editorial candidates are kept with false so the renderer,
    /// not the engine, decides admissibility.
    pub visual_eligible: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SceneHighlight {
    pub scene_id: String,
    /// None when no reliable candidate exists; diagnostics may use scene_id.
    pub title: Option<String>,
    pub title_status: String,
    pub title_source: String,
    pub bullets: Vec<SceneBulletSpan>,
    pub highlights: Vec<ScenePhraseSpan>,
    /// True when scene slice verified against transcript (global offsets valid).
    pub globally_indexed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub transcript: String,
    #[serde(default = "default_language")]
    pub language: String,
    /// Caller-provided, one-vector-per-segment embeddings. This crate performs no inference.
    pub embeddings: Vec<Vec<f32>>,
    #[serde(default)]
    pub timings: Vec<Timing>,
    #[serde(default)]
    pub options: Options,
    #[serde(default)]
    pub embedding_ms: f64,
    /// Bypass semantic vectors with a deterministic lexical-content ranking.
    #[serde(default)]
    pub lexical_only: bool,
    /// The caller-supplied phrase stop-word set for `language`, resolved from
    /// the repository lexicon (config/lexicons/<lang>/) at request time. This
    /// crate carries no language word lists of its own: keyphrase extraction is
    /// ONE language-independent algorithm whose only language-specific input is
    /// this set, so the same code path serves every language the lexicon
    /// profiles and the cross-linguistic `fallback` profile for the rest.
    /// Empty means "no filtering"; the Go adapter always injects the set.
    #[serde(default)]
    pub stopwords: Vec<String>,
}

fn default_language() -> String {
    "en".to_string()
}

/// Normalizes the injected phrase stop-word set once per request so every
/// downstream stage tests exactly the same tokens.
fn phrase_stopwords(words: &[String]) -> HashSet<String> {
    words
        .iter()
        .map(|word| word.trim().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RankedSentence {
    pub index: usize,
    #[serde(rename = "start")]
    pub start_sec: Option<f64>,
    #[serde(rename = "end")]
    pub end_sec: Option<f64>,
    pub text: String,
    pub centrality: f64,
    pub novelty: f64,
    pub importance: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BulletPoint {
    pub sentence_index: usize,
    pub text: String,
    #[serde(rename = "start")]
    pub start_sec: Option<f64>,
    #[serde(rename = "end")]
    pub end_sec: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StageTimings {
    pub split_ms: f64,
    pub embedding_ms: f64,
    pub similarity_ms: f64,
    pub ranking_ms: f64,
    pub summary_ms: f64,
    pub bullet_ms: f64,
    pub total_ms: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ResultDocument {
    pub summary: String,
    pub bullet_points: Vec<BulletPoint>,
    pub heavy_sentences: Vec<RankedSentence>,
    /// All sentences in descending importance order.
    pub ranked: Vec<RankedSentence>,
    /// Selected heavy sentences in chronological order.
    pub timeline: Vec<RankedSentence>,
    pub chapter_manifest: ChapterManifest,
    #[serde(default)]
    pub scene_highlights: Vec<SceneHighlight>,
    pub timings: StageTimings,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Chapter {
    pub title: String,
    pub title_source: String,
    /// Inclusive start index into the sentence sequence.
    pub start_sentence: usize,
    /// Exclusive end index into the sentence sequence.
    pub end_sentence: usize,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub bullets: Vec<ChapterBullet>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChapterBullet {
    /// Inclusive start index into the sentence sequence.
    pub start_sentence: usize,
    /// Exclusive end index into the sentence sequence.
    pub end_sentence: usize,
    /// Exact concatenation of the original contiguous source sentences.
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChapterManifest {
    pub schema_version: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
}

#[derive(Clone, Debug)]
struct Neighbor {
    index: usize,
    weight: f64,
}

pub fn split_sentences(text: &str, language: &str) -> Vec<Segment> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut cuts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i].1;
        if !matches!(ch, '.' | '!' | '?' | '。' | '！' | '？') {
            i += 1;
            continue;
        }
        if ch == '.' && is_decimal_point(&chars, i) {
            i += 1;
            continue;
        }
        let punctuation_end = punctuation_run_end(&chars, i);
        let next_index = punctuation_end + 1;
        let next = chars.get(next_index).map(|(_, c)| *c);
        let following_index = (next_index..chars.len())
            .find(|&index| !chars[index].1.is_whitespace())
            .unwrap_or(chars.len());

        // Split before a plausible capitalized sentence even when punctuation
        // is missing its following space, but not within initialisms.
        if ch == '.'
            && next.is_some_and(|value| !value.is_whitespace() && value.is_ascii_uppercase())
            && following_index == next_index
            && !(i > 0 && chars[i - 1].1.is_ascii_uppercase())
        {
            cuts.push(chars[punctuation_end].0 + chars[punctuation_end].1.len_utf8());
            i = punctuation_end + 1;
            continue;
        }
        if matches!(ch, '。' | '！' | '？')
            && next.is_some_and(|value| !value.is_whitespace())
            && chars
                .get(following_index)
                .is_some_and(|(_, value)| value.is_uppercase())
        {
            cuts.push(chars[punctuation_end].0 + chars[punctuation_end].1.len_utf8());
            i = punctuation_end + 1;
            continue;
        }
        if next.is_some_and(|value| {
            !value.is_whitespace()
                && !matches!(value, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»')
        }) && !matches!(ch, '。' | '！' | '？')
        {
            i = punctuation_end + 1;
            continue;
        }
        if matches!(ch, '.' | '。') && is_dotted_initialism(&chars, i, following_index) {
            i += 1;
            continue;
        }
        if ch == '.' && is_inner_dotted_abbreviation(&chars, i) {
            i += 1;
            continue;
        }
        if ch == '.' && suppress_abbreviation(&chars, i, following_index, language) {
            i += 1;
            continue;
        }
        cuts.push(chars[punctuation_end].0 + chars[punctuation_end].1.len_utf8());
        i = punctuation_end + 1;
        while i < chars.len()
            && matches!(chars[i].1, '"' | '\'' | '’' | '”' | ')' | ']' | '}' | '»')
        {
            if let Some(last) = cuts.last_mut() {
                *last = chars[i].0 + chars[i].1.len_utf8();
            }
            i += 1;
        }
    }

    let mut segments = Vec::new();
    let mut start = 0;
    for end in cuts {
        push_segment(text, start, end, &mut segments);
        start = end;
    }
    push_segment(text, start, text.len(), &mut segments);

    let mut bounded = Vec::new();
    for segment in segments {
        if word_count(&segment.text) <= MAX_SEGMENT_WORDS {
            bounded.push(segment);
        } else {
            split_long_segment(text, segment.start_byte, segment.end_byte, &mut bounded);
        }
    }
    bounded
}

fn push_segment(text: &str, start: usize, end: usize, output: &mut Vec<Segment>) {
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return;
    }
    let slice = &text[start..end];
    let leading = slice.len() - slice.trim_start().len();
    let value = slice.trim();
    if value.is_empty() {
        return;
    }
    let start_byte = start + leading;
    output.push(Segment {
        start_byte,
        end_byte: start_byte + value.len(),
        text: value.to_string(),
    });
}

fn split_long_segment(text: &str, start: usize, end: usize, output: &mut Vec<Segment>) {
    let source = &text[start..end];
    let mut ranges = Vec::new();
    let mut word_start = None;
    for (offset, ch) in source.char_indices() {
        if ch.is_whitespace() {
            if let Some(begin) = word_start.take() {
                ranges.push((begin, offset));
            }
        } else if word_start.is_none() {
            word_start = Some(offset);
        }
    }
    if let Some(begin) = word_start {
        ranges.push((begin, source.len()));
    }
    let mut first = 0;
    while first < ranges.len() {
        let end_word = (first + MAX_SEGMENT_WORDS).min(ranges.len());
        push_segment(
            text,
            start + ranges[first].0,
            start + ranges[end_word - 1].1,
            output,
        );
        first = end_word;
    }
}

fn is_decimal_point(chars: &[(usize, char)], i: usize) -> bool {
    i > 0
        && i + 1 < chars.len()
        && chars[i - 1].1.is_ascii_digit()
        && chars[i + 1].1.is_ascii_digit()
}

fn punctuation_run_end(chars: &[(usize, char)], i: usize) -> usize {
    let mut end = i;
    while end + 1 < chars.len() && matches!(chars[end + 1].1, '.' | '!' | '?' | '。' | '！' | '？')
    {
        end += 1;
    }
    end
}
fn is_dotted_initialism(chars: &[(usize, char)], period: usize, following: usize) -> bool {
    if chars[period].1 != '.' {
        return false;
    }
    if is_clock_suffix(chars, period) {
        return true;
    }
    if period + 1 < chars.len()
        && chars[period + 1].1.is_ascii_uppercase()
        && (period == 0 || !chars[period - 1].1.is_ascii_uppercase())
    {
        return true;
    }
    if period >= 2
        && chars[period - 1].1.is_ascii_uppercase()
        && chars[period - 2].1.is_ascii_uppercase()
        && chars
            .get(following)
            .is_some_and(|(_, value)| value.is_ascii_lowercase())
    {
        let mut cursor = period - 1;
        let mut capitals = 1;
        while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase()
        {
            capitals += 1;
            cursor -= 2;
        }
        if capitals >= 2 {
            return true;
        }
    }
    if period == 0 || !chars[period - 1].1.is_ascii_uppercase() {
        return false;
    }
    if chars
        .get(following)
        .is_some_and(|(_, value)| value.is_ascii_lowercase())
    {
        let mut cursor = period - 1;
        let mut capitals = 1;
        while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase()
        {
            capitals += 1;
            cursor -= 2;
        }
        return capitals >= 2;
    }

    // A spaced single-letter initial followed by another initial or a
    // capitalized token is part of a name, not a sentence boundary.
    match chars.get(following) {
        Some((_, value)) if value.is_ascii_uppercase() => {}
        _ => return false,
    }
    let next_token_end = chars[following..]
        .iter()
        .position(|(_, value)| !value.is_ascii_alphabetic())
        .map_or(chars.len(), |relative| following + relative);
    let next_is_initial = next_token_end == following + 1
        && chars
            .get(next_token_end)
            .is_some_and(|(_, value)| *value == '.');
    if !next_is_initial && next_token_end <= following + 1 {
        return false;
    }
    let starts_initial = period == 1 || !chars[period - 2].1.is_ascii_uppercase();
    if starts_initial {
        return true;
    }

    let mut cursor = period - 1;
    let mut capitals = 1;
    while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase() {
        capitals += 1;
        cursor -= 2;
    }
    capitals >= 2
}

fn is_clock_suffix(chars: &[(usize, char)], period: usize) -> bool {
    if period < 4
        || chars[period - 2].1 != '.'
        || !chars[period - 3].1.is_ascii_alphabetic()
        || !chars[period - 1].1.is_ascii_alphabetic()
    {
        return false;
    }
    let mut cursor = period - 4;
    while cursor > 0 && chars[cursor].1.is_whitespace() {
        cursor -= 1;
    }
    let mut saw_colon = false;
    let mut saw_digit = false;
    loop {
        match chars[cursor].1 {
            ':' => saw_colon = true,
            value if value.is_ascii_digit() => saw_digit = true,
            value if value.is_whitespace() => break,
            _ => return false,
        }
        if cursor == 0 {
            break;
        }
        cursor -= 1;
    }
    saw_colon && saw_digit
}

fn is_inner_dotted_abbreviation(chars: &[(usize, char)], period: usize) -> bool {
    period >= 1
        && period + 2 < chars.len()
        && chars[period - 1].1.is_ascii_alphabetic()
        && chars[period + 1].1.is_ascii_alphabetic()
        && chars[period + 2].1 == '.'
}

fn suppress_abbreviation(
    chars: &[(usize, char)],
    period: usize,
    following: usize,
    language: &str,
) -> bool {
    let mut start = period;
    while start > 0 && (chars[start - 1].1.is_ascii_alphabetic() || chars[start - 1].1 == '.') {
        start -= 1;
    }
    let token: String = chars[start..=period].iter().map(|(_, c)| *c).collect();
    let token = token.to_lowercase();
    let lang = language
        .split(|ch| ch == '-' || ch == '_')
        .next()
        .unwrap_or("en")
        .to_lowercase();
    let common = [
        "dr.", "mr.", "mrs.", "ms.", "prof.", "sr.", "sra.", "srta.", "st.", "vs.", "e.g.", "i.e.",
        "no.", "ph.d.",
    ];
    let localized: &[&str] = match lang.as_str() {
        "it" => &["dott.", "sig.", "ecc."],
        "es" => &["dra.", "dpto.", "núm."],
        "fr" => &["m.", "mme.", "mlle.", "etc."],
        "de" => &["z.b.", "bzw.", "usw.", "dr."],
        "pt" => &["dr.", "dra.", "sr.", "sra.", "etc."],
        _ => &[],
    };
    let corporate = ["inc.", "corp.", "ltd.", "co.", "llc.", "gmbh."];
    let next = chars.get(following).map(|(_, c)| *c);
    if corporate.contains(&token.as_str()) {
        return next.is_some_and(|c| c.is_lowercase());
    }
    let abbreviation = common.contains(&token.as_str())
        || localized.contains(&token.as_str())
        || ["etc.", "ecc."].contains(&token.as_str());
    abbreviation && next.is_some_and(|c| c.is_alphanumeric())
}

fn cosine(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x * y)
        .sum::<f64>()
        .clamp(0.0, 1.0)
}

fn cosine_matrix(vectors: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = vectors.len();
    let mut matrix = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let value = cosine(&vectors[i], &vectors[j]);
            matrix[i][j] = value;
            matrix[j][i] = value;
        }
    }
    matrix
}

fn top_k_graph(similarities: &[Vec<f64>]) -> Vec<Vec<Neighbor>> {
    let n = similarities.len();
    let mut graph = vec![Vec::new(); n];
    for i in 0..n {
        let mut candidates: Vec<_> = similarities[i]
            .iter()
            .copied()
            .enumerate()
            .filter(|(j, w)| *j != i && *w > 0.0)
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        for (j, weight) in candidates.into_iter().take(GRAPH_K) {
            if !graph[i].iter().any(|edge: &Neighbor| edge.index == j) {
                graph[i].push(Neighbor { index: j, weight });
            }
            if !graph[j].iter().any(|edge: &Neighbor| edge.index == i) {
                graph[j].push(Neighbor { index: i, weight });
            }
        }
    }
    for row in &mut graph {
        row.sort_by_key(|edge| edge.index);
    }
    graph
}

fn page_rank(graph: &[Vec<Neighbor>]) -> Vec<f64> {
    let n = graph.len();
    if n == 0 {
        return Vec::new();
    }
    let mut rank = vec![1.0 / n as f64; n];
    let degree: Vec<f64> = graph
        .iter()
        .map(|row| row.iter().map(|edge| edge.weight).sum())
        .collect();
    for _ in 0..50 {
        let mut next = vec![(1.0 - DAMPING) / n as f64; n];
        let mut dangling = 0.0;
        for i in 0..n {
            if degree[i] == 0.0 {
                dangling += rank[i];
                continue;
            }
            for edge in &graph[i] {
                next[edge.index] += DAMPING * rank[i] * edge.weight / degree[i];
            }
        }
        let share = DAMPING * dangling / n as f64;
        for value in &mut next {
            *value += share;
        }
        let delta: f64 = next.iter().zip(&rank).map(|(a, b)| (a - b).abs()).sum();
        rank = next;
        if delta < 1e-8 {
            break;
        }
    }
    rank
}

fn percentile_scale(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let low = sorted[((sorted.len() - 1) as f64 * 0.10) as usize];
    let high = sorted[((sorted.len() - 1) as f64 * 0.90).ceil() as usize];
    if high - low < 1e-9 {
        return vec![0.0; values.len()];
    }
    values
        .iter()
        .map(|value| ((value - low) / (high - low)).clamp(0.0, 1.0))
        .collect()
}

fn content_signal(text: &str, language: &str) -> f64 {
    let text = text.to_lowercase();
    let language = language.to_lowercase();
    let cues: &[&str] = match language
        .split(|ch| ch == '-' || ch == '_')
        .next()
        .unwrap_or("en")
    {
        "it" => &[
            "annunc",
            "licenzi",
            "bancarott",
            "acquis",
            "lanci",
            "dimess",
            "arrest",
            "record",
            "prima",
            "uccis",
            "morto",
            "approv",
            "accord",
        ],
        "es" => &[
            "anunci",
            "despid",
            "bancarrota",
            "adquis",
            "lanz",
            "renunci",
            "arrest",
            "récord",
            "primer",
            "muert",
            "aprob",
            "acuerdo",
        ],
        "pt" => &[
            "anunci", "demiss", "falên", "aquis", "lanç", "renunci", "preso", "record", "primeir",
            "mort", "aprov", "acordo",
        ],
        "fr" => &[
            "annonc",
            "licenci",
            "faillite",
            "acquisition",
            "lancé",
            "démission",
            "arrêt",
            "record",
            "premièr",
            "mort",
            "approuv",
            "accord",
        ],
        "de" => &[
            "ankündig",
            "entlass",
            "insolven",
            "übernahm",
            "start",
            "rücktritt",
            "verhaft",
            "rekord",
            "erstmal",
            "gestorb",
            "genehmig",
            "vereinbar",
        ],
        _ => &[
            "announc",
            "layoff",
            "laid off",
            "bankrupt",
            "acquisition",
            "acquired",
            "launch",
            "resign",
            "arrest",
            "charged",
            "denied",
            "recall",
            "record",
            "first",
            "disaster",
            "killed",
            "died",
            "approved",
            "settlement",
            "fired",
            "warn",
        ],
    };
    if cues.iter().any(|cue| text.contains(cue)) {
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

fn apply_noise_penalty(text: &str, importance: f64) -> f64 {
    let normalized = text.to_lowercase();
    let words: Vec<&str> = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut filler_count = words
        .iter()
        .filter(|word| matches!(**word, "um" | "uh" | "erm" | "er" | "hmm" | "ah"))
        .count();
    for phrase in [(&["you", "know"][..]), (&["i", "mean"][..])] {
        filler_count += words
            .windows(phrase.len())
            .filter(|window| *window == phrase)
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

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn target_summary_words(mode: &SummaryLength) -> (usize, usize, usize) {
    match mode {
        SummaryLength::Short => (65, 50, 80),
        SummaryLength::Medium => (125, 100, 150),
        SummaryLength::Long => (250, 200, 300),
    }
}

fn normalized_text(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn equivalent(a: &RankedSentence, b: &RankedSentence, vectors: &[Vec<f64>]) -> bool {
    normalized_text(&a.text) == normalized_text(&b.text)
        || cosine(&vectors[a.index], &vectors[b.index]) >= DUPLICATE_COSINE
}

fn select_diverse(
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

fn extract_summary(
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
        for index in 0..ranked.len() {
            if chosen.contains(&index) {
                continue;
            }
            let size = word_count(&ranked[index].text);
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

/// Creates a deterministic sparse lexical vector for offline ranking.
/// Hashing bounds memory use regardless of transcript vocabulary size.
/// The stop-word set is the caller's per-language input: the crate owns none.
fn lexical_vector(text: &str, stopwords: &HashSet<String>) -> Vec<f64> {
    const DIMENSIONS: usize = 512;
    let mut vector = vec![0.0; DIMENSIONS];
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.chars().count() > 1)
    {
        let lowered = token.to_lowercase();
        if stopwords.contains(&lowered) {
            continue;
        }
        let mut hash = 2_166_136_261_u32;
        for byte in lowered.bytes() {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(16_777_619);
        }
        vector[hash as usize % DIMENSIONS] += 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm > f64::EPSILON {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

#[derive(Clone, Copy, Debug, Default)]
struct BoundaryEvidence {
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

fn token_distribution(sentences: &[RankedSentence]) -> std::collections::HashMap<String, f64> {
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

fn distribution_similarity(
    left: &std::collections::HashMap<String, f64>,
    right: &std::collections::HashMap<String, f64>,
) -> f64 {
    left.iter()
        .map(|(token, value)| value * right.get(token).copied().unwrap_or_default())
        .sum::<f64>()
        .clamp(0.0, 1.0)
}

fn boundary_evidence(
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
    let left_tokens = token_distribution(&ranked[left].to_vec());
    let right_tokens = token_distribution(&ranked[right].to_vec());
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
fn chapter_title(
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
        word.chars().count() > 1
            && !word.chars().all(char::is_numeric)
            && !stopwords.contains(word)
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
            record(words[end - MAX_TITLE_TOKENS..end].join(" "), TRIMMED_RUN_TAIL_SPAN);
            continue;
        }
        for length in 2..=span {
            for offset in 0..=(span - length) {
                let quality = if length == span {
                    WHOLE_RUN_SPAN
                } else {
                    PARTIAL_SPAN
                };
                record(words[start + offset..start + offset + length].join(" "), quality);
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
            let normalized = topic.replace('-', " ").replace('_', " ").trim().to_string();
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

fn contextual_bullet_span(
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
fn is_statistical_sentence(text: &str) -> bool {
    let lower = text.to_lowercase();
    let has_digit = text.chars().any(|c| c.is_ascii_digit());
    if !has_digit {
        return false;
    }
    let words = text.split_whitespace().count();
    if words <= 4 {
        return false;
    }
    let stat_markers = [
        '%', '$', '€', '£',
    ];
    if text.chars().any(|c| stat_markers.contains(&c)) {
        return true;
    }
    let verbs = [
        " rose ", " fell ", " grew ", " dropped ", " increased ", " decreased ",
        " declined ", " surged ", " shrank ", " down ", " up ", " by ",
        " aumentate ", " aumentato ", " diminuito ", " calo ", " crescita ",
        " riduzione ", " rispetto ", " since ", " from ", " dal ",
    ];
    let padded = format!(" {lower} ");
    verbs.iter().any(|v| padded.contains(v))
}

fn title_case_phrase(phrase: &str) -> String {
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
fn select_scene_title(
    scene_text: &str,
    topic: Option<&str>,
    stopwords: &HashSet<String>,
) -> (Option<String>, String, String) {
    if let Some(raw) = topic.map(str::trim).filter(|t| !t.is_empty()) {
        let normalized = raw.replace('-', " ").replace('_', " ");
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
    if title.trim().is_empty() || is_statistical_sentence(&title) && title.split_whitespace().count() > 5 {
        return (None, "unavailable".into(), "none".into());
    }
    if is_statistical_sentence(scene_text) && title == scene_text.trim() {
        return (None, "unavailable".into(), "none".into());
    }
    (Some(title), "resolved".into(), source)
}

/// Extract short overlay spans inside top sentences. The engine may score
/// longer candidates; visual_eligible marks the <=5-token budget subset.
fn select_phrase_spans(
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
                let first = win.first().unwrap().2.to_lowercase();
                let last = win.last().unwrap().2.to_lowercase();
                if stopwords.contains(&first) || stopwords.contains(&last) {
                    continue;
                }
                let text = win.iter().map(|(_, _, t)| *t).collect::<Vec<_>>().join(" ");
                if text.chars().count() < 4 {
                    continue;
                }
                let rarity = win
                    .iter()
                    .map(|(_, _, t)| if t.chars().all(|c| c.is_alphabetic()) && t.len() > 5 { 1.5 } else { 1.0 })
                    .sum::<f64>()
                    / len as f64;
                let brevity = if len <= 5 { 1.2 } else { 0.85 };
                let score = importance[si] * rarity * brevity;
                if best.map(|(_, _, s)| score > s).unwrap_or(true) {
                    best = Some((win.first().unwrap().0, win.last().unwrap().1, score));
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
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.sentence_index.cmp(&b.sentence_index)));
    out.truncate(limit);
    out
}

fn build_scene_highlights(
    transcript: &str,
    global_segments: &[Segment],
    global_importance: &[f64],
    scenes: &[SceneInput],
    language: &str,
    stopwords: &HashSet<String>,
) -> (Vec<SceneHighlight>, bool) {
    let mut out = Vec::new();
    let mut certified = true;
    let mut seen_ids = std::collections::HashSet::new();
    for scene in scenes {
        if scene.scene_id.trim().is_empty() || !seen_ids.insert(scene.scene_id.clone()) {
            certified = false;
            continue;
        }
        let globally_indexed;
        let scene_segments: Vec<Segment>;
        let local_importance: Vec<f64>;
        if let (Some(s), Some(e)) = (scene.start_byte, scene.end_byte) {
            let valid = e <= transcript.len()
                && s < e
                && transcript.is_char_boundary(s)
                && transcript.is_char_boundary(e)
                && transcript[s..e] == *scene.text;
            if !valid {
                // Slice mismatch: local-only analysis, no fabricated globals.
                globally_indexed = false;
                let parts = split_sentences(&scene.text, language);
                if parts.is_empty() {
                    out.push(SceneHighlight {
                        scene_id: scene.scene_id.clone(),
                        title: None,
                        title_status: "unavailable".into(),
                        title_source: "none".into(),
                        bullets: Vec::new(),
                        highlights: Vec::new(),
                        globally_indexed,
                    });
                    continue;
                }
                // Map local sentences onto overlapping global range when the
                // raw bytes match somewhere (repeated text => first match is
                // NOT assumed; require the declared slice, else local-only).
                certified = false;
                scene_segments = parts;
                local_importance = vec![0.5; scene_segments.len()];
            } else {
                globally_indexed = true;
                let mut v = Vec::new();
                let mut imp = Vec::new();
                for (gi, seg) in global_segments.iter().enumerate() {
                    if seg.start_byte >= s && seg.end_byte <= e {
                        v.push(seg.clone());
                        imp.push(*global_importance.get(gi).unwrap_or(&0.5));
                    }
                }
                if v.is_empty() {
                    let parts = split_sentences(&scene.text, language);
                    scene_segments = parts;
                    local_importance = vec![0.5; scene_segments.len()];
                } else {
                    scene_segments = v;
                    local_importance = imp;
                }
            }
        } else {
            globally_indexed = false;
            let parts = split_sentences(&scene.text, language);
            if parts.is_empty() {
                out.push(SceneHighlight {
                    scene_id: scene.scene_id.clone(),
                    title: None,
                    title_status: "unavailable".into(),
                    title_source: "none".into(),
                    bullets: Vec::new(),
                    highlights: Vec::new(),
                    globally_indexed,
                });
                continue;
            }
            scene_segments = parts;
            local_importance = vec![0.5; scene_segments.len()];
        }
        if scene_segments.is_empty() {
            out.push(SceneHighlight {
                scene_id: scene.scene_id.clone(),
                title: None,
                title_status: "unavailable".into(),
                title_source: "none".into(),
                bullets: Vec::new(),
                highlights: Vec::new(),
                globally_indexed,
            });
            continue;
        }
        let (title, title_status, title_source) =
            select_scene_title(&scene.text, scene.topic.as_deref(), stopwords);
        // Bullet budget scales with scene length; short scenes get none invented.
        let words: usize = scene.text.split_whitespace().count();
        let bullet_n = if words < 15 || scene_segments.len() < 1 {
            0
        } else if words < 60 {
            1.min(scene_segments.len())
        } else {
            3.min(scene_segments.len())
        };
        let mut order: Vec<usize> = (0..scene_segments.len()).collect();
        order.sort_by(|&a, &b| local_importance[b].total_cmp(&local_importance[a]).then(a.cmp(&b)));
        let mut bullets = Vec::new();
        for &li in order.iter().take(bullet_n) {
            // Global sentence index when indexed, else local index.
            let (gs, ge) = if globally_indexed {
                let seg = &scene_segments[li];
                let gi = global_segments.iter().position(|g| g.start_byte == seg.start_byte).unwrap_or(li);
                (gi, gi + 1)
            } else {
                (li, li + 1)
            };
            bullets.push(SceneBulletSpan {
                sentence_start: gs,
                sentence_end: ge,
                text: scene_segments[li].text.clone(),
            });
        }
        bullets.sort_by(|a, b| a.sentence_start.cmp(&b.sentence_start));
        let highlights = select_phrase_spans(&scene_segments, &local_importance, stopwords, 2);
        out.push(SceneHighlight {
            scene_id: scene.scene_id.clone(),
            title,
            title_status,
            title_source,
            bullets,
            highlights,
            globally_indexed,
        });
    }
    (out, certified)
}

fn build_chapters(
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
        let mut selected: Vec<usize> = Vec::new();
        for candidate in candidates {
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
            .map(|(span_start, span_end)| ChapterBullet {
                start_sentence: span_start,
                end_sentence: span_end,
                text: ranked[span_start..span_end]
                    .iter()
                    .map(|sentence| sentence.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
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

/// the built-in bounded lexical vectors when lexical_only is explicitly true.
/// Output summaries and bullets are extractive to prevent unsupported claims.
pub fn run(request: Request) -> Result<ResultDocument, String> {
    run_with_scene_context(request, &[], &[])
}

/// Per-scene highlights entry point (editorial.v1). Legacy extractive
/// products are computed by the canonical path; highlights reuse the ranked
/// importance so there is exactly one analysis, not two engines.
/// Duplicate scene ids are a contract error (manifest not certified);
/// slice mismatches degrade to local-only analysis for that scene.
pub fn run_with_scene_inputs(
    request: Request,
    inputs: &[SceneInput],
    chapter_options: &ChapterOptions,
) -> Result<ResultDocument, String> {
    let mut seen = std::collections::HashSet::new();
    for input in inputs {
        if input.scene_id.trim().is_empty() || !seen.insert(input.scene_id.clone()) {
            return Err("duplicate scene identity".into());
        }
    }
    let scenes: Vec<String> = inputs.iter().map(|s| s.text.clone()).collect();
    let topics: Vec<String> = inputs
        .iter()
        .map(|s| s.topic.clone().unwrap_or_default())
        .collect();
    let mut doc = run_with_chapter_options(request.clone(), &scenes, &topics, chapter_options)?;
    let segments = split_sentences(&request.transcript, &request.language);
    let mut importance_by_index = vec![0.5; segments.len()];
    for ranked in &doc.ranked {
        if ranked.index < importance_by_index.len() {
            importance_by_index[ranked.index] = ranked.importance;
        }
    }
    let stopwords = phrase_stopwords(&request.stopwords);
    let (highlights, certified) = build_scene_highlights(
        &request.transcript,
        &segments,
        &importance_by_index,
        inputs,
        &request.language,
        &stopwords,
    );
    if !certified {
        return Err("incoherent scene identity".into());
    }
    doc.scene_highlights = highlights;
    Ok(doc)
}

pub fn run_with_scene_context(
    request: Request,
    scenes: &[String],
    scene_topics: &[String],
) -> Result<ResultDocument, String> {
    run_with_chapter_options(request, scenes, scene_topics, &ChapterOptions::default())
}

pub fn run_with_chapter_options(
    request: Request,
    scenes: &[String],
    scene_topics: &[String],
    chapter_options: &ChapterOptions,
) -> Result<ResultDocument, String> {
    if chapter_options.min_sentences == 0
        || chapter_options.max_sentences < chapter_options.min_sentences
        || chapter_options.max_sentences > 200
        || chapter_options.context_sentences == 0
        || chapter_options.context_sentences > chapter_options.max_sentences
        || chapter_options.profile_version.trim().is_empty()
        || [
            chapter_options.semantic_weight,
            chapter_options.lexical_weight,
            chapter_options.scene_weight,
            chapter_options.evidence_weight,
            chapter_options.complexity_penalty,
        ]
        .iter()
        .any(|weight| !weight.is_finite() || *weight < 0.0)
    {
        return Err("invalid chapter options".into());
    }
    let total = Instant::now();
    if request.transcript.trim().is_empty() {
        return Err("transcript is empty".into());
    }
    if !request.embedding_ms.is_finite() || request.embedding_ms < 0.0 {
        return Err("embedding_ms is invalid".into());
    }
    if request
        .options
        .top_fraction
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err("top_fraction must be in [0,1]".into());
    }
    let split_start = Instant::now();
    let segments = split_sentences(&request.transcript, &request.language);
    let split_ms = split_start.elapsed().as_secs_f64() * 1000.0;
    if segments.is_empty() {
        return Err("sentence splitter returned no sentences".into());
    }
    let mut scene_starts = Vec::new();
    let mut resolved_scenes = scenes;
    let mut resolved_topics = scene_topics;
    if !resolved_scenes.is_empty() {
        if !resolved_topics.is_empty() && resolved_topics.len() != resolved_scenes.len() {
            // Scene metadata is optional context for chapter labels, never a
            // reason to alter or suppress the legacy extractive products.
            resolved_scenes = &[];
            resolved_topics = &[];
        }
    }
    if !resolved_scenes.is_empty() {
        let mut sentence_offset = 0usize;
        let mut reconstructed = Vec::new();
        for (scene_index, scene) in resolved_scenes.iter().enumerate() {
            let parts = split_sentences(scene, &request.language);
            if parts.is_empty() {
                continue;
            }
            scene_starts.push((sentence_offset, scene_index));
            sentence_offset += parts.len();
            reconstructed.extend(parts.into_iter().map(|part| part.text));
        }
        if reconstructed.len() != segments.len()
            || reconstructed
                .iter()
                .zip(&segments)
                .any(|(a, b)| a != &b.text)
        {
            scene_starts.clear();
            resolved_topics = &[];
        }
    }
    if request.lexical_only {
        if !request.embeddings.is_empty() {
            return Err("lexical_only cannot be combined with embeddings".into());
        }
    } else if request.embeddings.len() != segments.len() {
        return Err(format!(
            "embedding count {} does not match sentence count {}",
            request.embeddings.len(),
            segments.len()
        ));
    }
    if !request.timings.is_empty() && request.timings.len() != segments.len() {
        return Err("timing count does not match sentence count".into());
    }
    let mut previous_start = -1_i64;
    for timing in &request.timings {
        if timing.start_us < 0
            || timing.end_us <= timing.start_us
            || timing.start_us < previous_start
        {
            return Err("invalid/nonchronological sentence timing".into());
        }
        previous_start = timing.start_us;
    }
    let stopwords = phrase_stopwords(&request.stopwords);
    let similarity_start = Instant::now();
    let vectors = if request.lexical_only {
        if !request.embeddings.is_empty() {
            return Err("lexical_only cannot be combined with embeddings".into());
        }
        segments
            .iter()
            .map(|segment| lexical_vector(&segment.text, &stopwords))
            .collect()
    } else {
        let dimension = request.embeddings.first().map(Vec::len).unwrap_or(0);
        if dimension == 0 {
            return Err("embedding dimension is zero".into());
        }
        let mut vectors = Vec::with_capacity(request.embeddings.len());
        for (index, row) in request.embeddings.iter().enumerate() {
            if row.len() != dimension {
                return Err(format!("embedding {index} dimension mismatch"));
            }
            if row.iter().any(|value| !value.is_finite()) {
                return Err(format!("embedding {index} is non-finite"));
            }
            let mut vector: Vec<f64> = row.iter().map(|value| *value as f64).collect();
            let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
            if norm <= f64::EPSILON {
                return Err(format!("embedding {index} has zero norm"));
            }
            for value in &mut vector {
                *value /= norm;
            }
            vectors.push(vector);
        }
        vectors
    };
    let n = segments.len();
    let similarities = cosine_matrix(&vectors);
    let mut raw_novelty = vec![0.0; n];
    for i in 1..n {
        let begin = i.saturating_sub(LOCAL_CONTEXT);
        let nearest = (begin..i)
            .map(|j| similarities[i][j])
            .fold(0.0_f64, f64::max);
        raw_novelty[i] = 1.0 - nearest;
    }
    let similarity_ms = similarity_start.elapsed().as_secs_f64() * 1000.0;
    let ranking_start = Instant::now();
    let graph = top_k_graph(&similarities);
    let centrality = percentile_scale(&page_rank(&graph));
    let novelty = percentile_scale(&raw_novelty);
    let mut source_order = Vec::with_capacity(n);
    for i in 0..n {
        let timing = request.timings.get(i);
        let score = apply_noise_penalty(
            &segments[i].text,
            0.65 * centrality[i]
                + 0.10 * novelty[i]
                + 0.25 * content_signal(&segments[i].text, &request.language),
        );
        source_order.push(RankedSentence {
            index: i,
            start_sec: timing.map(|value| value.start_us as f64 / 1_000_000.0),
            end_sec: timing.map(|value| value.end_us as f64 / 1_000_000.0),
            text: segments[i].text.clone(),
            centrality: centrality[i],
            novelty: novelty[i],
            importance: score.clamp(0.0, 1.0),
        });
    }
    let mut rank_indices: Vec<usize> = (0..n).collect();
    rank_indices.sort_by(|&a, &b| {
        source_order[b]
            .importance
            .total_cmp(&source_order[a].importance)
            .then(a.cmp(&b))
    });
    let fraction = request.options.top_fraction.unwrap_or(0.125);
    let min_heavy = request.options.min_heavy.clamp(1, 15);
    let max_heavy = request.options.max_heavy.max(min_heavy).min(15);
    let desired = ((n as f64 * fraction).ceil() as usize)
        .max(min_heavy)
        .min(max_heavy)
        .min(n);
    let selected = select_diverse(&rank_indices, &source_order, &vectors, desired);
    let heavy_sentences: Vec<_> = selected
        .iter()
        .map(|&index| source_order[index].clone())
        .collect();
    let ranking_ms = ranking_start.elapsed().as_secs_f64() * 1000.0;

    let summary_start = Instant::now();
    let summary = extract_summary(&source_order, &vectors, &request.options.summary_length);
    let summary_ms = summary_start.elapsed().as_secs_f64() * 1000.0;

    let bullet_start = Instant::now();
    let bullet_limit = request.options.bullet_count.clamp(1, 10);
    let bullets = select_diverse(&rank_indices, &source_order, &vectors, bullet_limit)
        .into_iter()
        .map(|index| BulletPoint {
            sentence_index: source_order[index].index,
            text: source_order[index].text.clone(),
            start_sec: source_order[index].start_sec,
            end_sec: source_order[index].end_sec,
        })
        .collect();
    let chapters = build_chapters(
        &source_order,
        &vectors,
        &request.timings,
        &scene_starts,
        resolved_topics,
        chapter_options,
        &stopwords,
    );
    let mut timeline = heavy_sentences.clone();
    timeline.sort_by(|a, b| match (a.start_sec, b.start_sec) {
        (Some(start_a), Some(start_b)) => start_a.total_cmp(&start_b).then(a.index.cmp(&b.index)),
        _ => a.index.cmp(&b.index),
    });
    let bullet_ms = bullet_start.elapsed().as_secs_f64() * 1000.0;
    let mut ranked = source_order;
    ranked.sort_by(|a, b| {
        b.importance
            .total_cmp(&a.importance)
            .then(a.index.cmp(&b.index))
    });
    let elapsed_ms = total.elapsed().as_secs_f64() * 1000.0;
    Ok(ResultDocument {
        summary,
        bullet_points: bullets,
        heavy_sentences,
        ranked,
        timeline,
        chapter_manifest: ChapterManifest {
            schema_version: "chapter_manifest.v1".into(),
            chapters,
        },
        scene_highlights: Vec::new(),
        timings: StageTimings {
            split_ms,
            embedding_ms: request.embedding_ms,
            similarity_ms,
            ranking_ms,
            summary_ms,
            bullet_ms,
            total_ms: elapsed_ms + request.embedding_ms,
        },
    })
}


#[cfg(test)]
#[path = "phrase_impact_tests.rs"]
mod tests;
