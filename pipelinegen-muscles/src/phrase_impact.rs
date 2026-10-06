use serde::{Deserialize, Serialize};
use std::time::Instant;

const GRAPH_K: usize = 6;
const LOCAL_CONTEXT: usize = 3;
const DAMPING: f64 = 0.85;
const MAX_SEGMENT_WORDS: usize = 48;
const DUPLICATE_COSINE: f64 = 0.96;

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
}

fn default_language() -> String {
    "en".to_string()
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
    pub timings: StageTimings,
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
        && chars.get(following).is_some_and(|(_, value)| value.is_ascii_lowercase())
    {
        let mut cursor = period - 1;
        let mut capitals = 1;
        while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase() {
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
    if chars.get(following).is_some_and(|(_, value)| value.is_ascii_lowercase()) {
        let mut cursor = period - 1;
        let mut capitals = 1;
        while cursor >= 2 && chars[cursor - 1].1 == '.' && chars[cursor - 2].1.is_ascii_uppercase() {
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
        && chars.get(next_token_end).is_some_and(|(_, value)| *value == '.');
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
fn lexical_vector(text: &str, language: &str) -> Vec<f64> {
    const DIMENSIONS: usize = 512;
    let stopwords = match language
        .split('-')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "it" => [
            "il", "lo", "la", "i", "gli", "le", "di", "a", "da", "in", "con", "su", "per", "tra",
            "e", "o", "un", "una", "che", "è",
        ]
        .as_slice(),
        "es" => [
            "el", "la", "los", "las", "de", "a", "en", "con", "y", "o", "un", "una", "que", "es",
        ]
        .as_slice(),
        "fr" => [
            "le", "la", "les", "de", "des", "du", "à", "en", "et", "ou", "un", "une", "que", "est",
        ]
        .as_slice(),
        _ => [
            "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "with", "is", "are",
            "was", "were", "that", "this", "it",
        ]
        .as_slice(),
    };
    let mut vector = vec![0.0; DIMENSIONS];
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.chars().count() > 1)
    {
        if stopwords
            .iter()
            .any(|word| token.eq_ignore_ascii_case(word))
        {
            continue;
        }
        let mut hash = 2_166_136_261_u32;
        for byte in token.to_lowercase().bytes() {
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

/// Pure deterministic transcript processing over caller-provided vectors or
/// the built-in bounded lexical vectors when lexical_only is explicitly true.
/// Output summaries and bullets are extractive to prevent unsupported claims.
pub fn run(request: Request) -> Result<ResultDocument, String> {
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
    let similarity_start = Instant::now();
    let vectors = if request.lexical_only {
        if !request.embeddings.is_empty() {
            return Err("lexical_only cannot be combined with embeddings".into());
        }
        segments
            .iter()
            .map(|segment| lexical_vector(&segment.text, &request.language))
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
mod tests {
    use super::*;

    fn vec3(x: f32, y: f32, z: f32) -> Vec<f32> {
        vec![x, y, z]
    }
    fn request(lines: &[&str], embeddings: Vec<Vec<f32>>, options: Options) -> Request {
        Request {
            transcript: lines.join(" "),
            language: "en".into(),
            embeddings,
            timings: Vec::new(),
            options,
            embedding_ms: 0.0,
            lexical_only: false,
        }
    }

    fn orthogonal_embeddings(count: usize) -> Vec<Vec<f32>> {
        (0..count)
            .map(|index| {
                let mut vector = vec![0.0; count];
                vector[index] = 1.0;
                vector
            })
            .collect()
    }

    fn labeled_test_sentences(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| {
                format!(
                    "Sentence {index} explains that the company reported operational details across its regional businesses during the current financial quarter ending in June."
                )
            })
            .collect()
    }

    #[test]
    fn sentence_split_plain_and_difficult_cases_preserve_surface() {
        let source =
            "Apple launched a new phone. Sales increased by 20%. Investors reacted positively.";
        let parts = split_sentences(source, "en");
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            [
                "Apple launched a new phone.",
                "Sales increased by 20%.",
                "Investors reacted positively."
            ]
        );
        assert!(parts
            .iter()
            .all(|part| &source[part.start_byte..part.end_byte] == part.text));
        let difficult = "Dr. Smith joined Apple Inc. in 2025. Revenue reached $2.5 billion. U.S. sales increased.";
        assert_eq!(
            split_sentences(difficult, "en")
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            [
                "Dr. Smith joined Apple Inc. in 2025.",
                "Revenue reached $2.5 billion.",
                "U.S. sales increased."
            ]
        );
    }

    #[test]
    fn localized_splitting_and_unpunctuated_fallback() {
        for (language, text, expected) in [
            ("it", "La dott. Rossi è arrivata. Poi ha parlato.", 2),
            ("es", "La Dra. García llegó. Después habló.", 2),
            ("pt", "O Dr. Silva chegou. Depois falou.", 2),
            ("fr", "M. Dupont est arrivé. Ensuite il a parlé.", 2),
            ("de", "Dr. Müller kam an. Danach sprach er.", 2),
            ("ja", "これは文です。次の文です！", 2),
        ] {
            assert_eq!(
                split_sentences(text, language).len(),
                expected,
                "{language}: {text}"
            );
        }
        assert_eq!(
            split_sentences("Revenue grew.Next the board met.", "en").len(),
            2
        );
        let source = (0..95)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = split_sentences(&source, "en");
        assert!(
            chunks.len() >= 2
                && chunks
                    .iter()
                    .all(|part| word_count(&part.text) <= MAX_SEGMENT_WORDS)
        );
        assert_eq!(
            chunks
                .iter()
                .flat_map(|part| part.text.split_whitespace())
                .collect::<Vec<_>>(),
            source.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn synthetic_transcript_preserves_long_sentences_abbreviations_and_utf8_offsets() {
        let transcript = include_str!("../fixtures/elon_musk_synthetic_transcript_it.txt");
        let segments = split_sentences(transcript, "it");
        let segment_texts: Vec<&str> = segments.iter().map(|segment| segment.text.as_str()).collect();
        for expected in [
            "Il secondo punto chiave fu l'annuncio sintetico secondo cui Tesla Energy stava valutando, sempre in questo scenario inventato, un investimento da €750 milioni in un impianto di accumulo vicino a Berlino con una decisione finale prevista per il 2 aprile 2026.",
            "Il settimo punto ad alta importanza dichiarò che Tesla non avrebbe annunciato licenziamenti durante l'evento sintetico e che, al contrario, il piano operativo ipotizzava 1,500 nuove assunzioni tecniche tra Texas e Germania entro diciotto mesi.",
            "La quarta frase pesante stabilì che SpaceX avrebbe programmato, nello scenario sintetico, una finestra di prova di Starship il 21 giugno 2026 alle 9:15 a.m., con un massimo di tre tentativi tecnici distribuiti nella stessa settimana.",
            "J. R. Collins, personaggio fittizio introdotto apposta per il test, chiese se l'uso delle iniziali puntate potesse confondere un segmentatore di frasi o un modello che non gestisce correttamente i nomi abbreviati.",
            "Alle 10:45 p.m. la sessione terminò e i partecipanti lasciarono la sala, mentre il sistema di registrazione salvò la trascrizione completa per i test successivi di segmentazione, entity extraction, ranking e summarization.",
            "Un ulteriore blocco di controllo descrive una riunione successiva del 18 marzo 2026 a Milan, Italy, nella quale Luca Bianchi confronta i nove passaggi principali del benchmark e segnala che alcune frasi molto specifiche possono risultare semanticamente lontane dal centroide pur essendo essenziali per il video.",
        ] {
            assert!(segment_texts.contains(&expected), "sentence was split or altered: {expected}");
        }
        for segment in &segments {
            assert!(segment.end_byte <= transcript.len());
            assert!(transcript.is_char_boundary(segment.start_byte));
            assert!(transcript.is_char_boundary(segment.end_byte));
            assert_eq!(&transcript[segment.start_byte..segment.end_byte], segment.text);
        }
        assert!(segments.iter().all(|segment| word_count(&segment.text) <= MAX_SEGMENT_WORDS));
    }

    fn is_extractive(summary: &str, segments: &[Segment]) -> bool {
        let mut remaining = summary;
        let mut matched = 0;
        for segment in segments {
            let Some(tail) = remaining.strip_prefix(&segment.text) else {
                continue;
            };
            remaining = tail.strip_prefix(' ').unwrap_or(tail);
            matched += 1;
            if remaining.is_empty() {
                return matched > 0;
            }
        }
        false
    }

    #[test]
    fn synthetic_transcript_summarization_keeps_negation_and_source_spans() {
        #[derive(Deserialize)]
        struct GroundTruth {
            sentences: Vec<LabeledSentence>,
        }
        #[derive(Deserialize)]
        struct LabeledSentence {
            id: String,
            text: String,
            label: String,
            #[serde(default)]
            must_preserve_negation: Option<String>,
        }

        let transcript = include_str!("../fixtures/elon_musk_synthetic_transcript_it.txt");
        let truth: GroundTruth = serde_json::from_str(include_str!("../fixtures/elon_musk_synthetic_ground_truth.json")).unwrap();
        let result = run(Request {
            transcript: transcript.to_string(),
            language: "it".into(),
            embeddings: Vec::new(),
            timings: Vec::new(),
            options: Options {
                summary_length: SummaryLength::Short,
                bullet_count: 10,
                min_heavy: 15,
                max_heavy: 15,
                top_fraction: Some(0.125),
            },
            embedding_ms: 0.0,
            lexical_only: true,
        })
        .unwrap();
        assert!(truth.sentences.iter().all(|sentence| matches!(sentence.label.as_str(), "heavy" | "trap")));
        let segments = split_sentences(transcript, "it");
        let segment_texts: std::collections::HashSet<&str> = segments.iter().map(|segment| segment.text.as_str()).collect();
        for sentence in &truth.sentences {
            assert!(segment_texts.contains(sentence.text.as_str()), "fixture sentence {} not found in source split", sentence.id);
        }
        let heavy_count = truth.sentences.iter().filter(|sentence| sentence.label == "heavy").count();
        assert_eq!(heavy_count, 9);
        let id_by_text: std::collections::HashMap<&str, &str> = truth.sentences.iter().map(|sentence| (sentence.text.as_str(), sentence.id.as_str())).collect();
        let heavy_ids: std::collections::HashSet<&str> = truth.sentences.iter().filter(|sentence| sentence.label == "heavy").map(|sentence| sentence.id.as_str()).collect();
        let mut ranking_metrics = Vec::new();
        for cutoff in [5, 10] {
            let top = result.ranked.iter().take(cutoff).collect::<Vec<_>>();
            let hits = top.iter().filter(|sentence| id_by_text.get(sentence.text.as_str()).is_some_and(|id| heavy_ids.contains(id))).count();
            let precision = hits as f64 / top.len() as f64;
            assert!(precision.is_finite() && (0.0..=1.0).contains(&precision));
            ranking_metrics.push((cutoff, precision));
        }
        assert_eq!(ranking_metrics.len(), 2);
        let negation = truth.sentences.iter().find_map(|sentence| sentence.must_preserve_negation.as_deref()).unwrap();
        let summary_contains_claim = result.summary.to_lowercase().contains("licenziamenti");
        let summary_preserves_negation = result.summary.to_lowercase().contains(&negation.to_lowercase());
        if summary_contains_claim {
            assert!(summary_preserves_negation, "summary mentioned layoffs but dropped negation: {}", result.summary);
        }
        assert!(is_extractive(&result.summary, &segments), "summary must only contain complete source sentences: {}", result.summary);
        for bullet in &result.bullet_points {
            assert!(segments.iter().any(|segment| segment.text == bullet.text), "bullet must be a source sentence: {}", bullet.text);
        }
    }

    #[test]
    fn whisper_like_unpunctuated_transcript_keeps_numbers_and_all_surface_text() {
        let transcript = "well you know the company announced 20,000 layoffs uh investors reacted quickly the board will review the plan next year";
        let segments = split_sentences(transcript, "en");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, transcript);
        let result = run(Request {
            transcript: transcript.into(),
            language: "en".into(),
            embeddings: vec![vec3(1., 0., 0.)],
            timings: Vec::new(),
            options: Options {
                min_heavy: 1,
                max_heavy: 1,
                summary_length: SummaryLength::Short,
                bullet_count: 1,
                ..Options::default()
            },
            embedding_ms: 0.0,
            lexical_only: false,
        })
        .unwrap();
        assert_eq!(result.heavy_sentences[0].text, transcript);
        assert!(result.summary.contains("20,000 layoffs"));
        assert!(result.bullet_points[0].text.contains("20,000 layoffs"));
    }

    #[test]
    fn end_to_end_outputs_preserve_sentences_in_supported_languages() {
        let cases = [
            (
                "en",
                [
                    "The company announced layoffs.",
                    "Revenue increased this year.",
                ],
            ),
            (
                "it",
                [
                    "L'azienda ha annunciato licenziamenti.",
                    "I ricavi sono aumentati quest'anno.",
                ],
            ),
            (
                "es",
                [
                    "La empresa anunció despidos.",
                    "Los ingresos aumentaron este año.",
                ],
            ),
            (
                "pt",
                [
                    "A empresa anunciou demissões.",
                    "A receita aumentou este ano.",
                ],
            ),
            (
                "fr",
                [
                    "L'entreprise a annoncé des licenciements.",
                    "Les revenus ont augmenté cette année.",
                ],
            ),
            (
                "de",
                [
                    "Das Unternehmen kündigte Entlassungen an.",
                    "Der Umsatz stieg in diesem Jahr.",
                ],
            ),
        ];
        for (language, lines) in cases {
            let result = run(Request {
                transcript: lines.join(" "),
                language: language.into(),
                embeddings: orthogonal_embeddings(lines.len()),
                timings: Vec::new(),
                options: Options {
                    summary_length: SummaryLength::Short,
                    min_heavy: 1,
                    max_heavy: 2,
                    top_fraction: Some(1.0),
                    bullet_count: 2,
                    ..Options::default()
                },
                embedding_ms: 0.0,
                lexical_only: false,
            })
            .unwrap();
            assert_eq!(result.ranked.len(), 2, "language: {language}");
            assert_eq!(result.heavy_sentences.len(), 2, "language: {language}");
            assert_eq!(result.bullet_points.len(), 2, "language: {language}");
            for line in lines {
                assert!(result.summary.contains(line), "language: {language}");
                assert!(
                    result
                        .bullet_points
                        .iter()
                        .any(|bullet| bullet.text == line),
                    "language: {language}"
                );
            }
        }
    }

    #[test]
    fn ranked_timing_and_content_fields_match_the_public_json_contract() {
        let lines = [
            "Apple reported record revenue.",
            "20,000 workers were laid off.",
        ];
        let result = run(request(
            &lines,
            vec![vec3(1., 0., 0.), vec3(0., 1., 0.)],
            Options {
                min_heavy: 2,
                max_heavy: 2,
                ..Default::default()
            },
        ))
        .unwrap();
        let json = serde_json::to_value(result).unwrap();
        assert!(json["ranked"][0]["index"].is_number());
        assert!(json["ranked"][0]["start"].is_null());
        assert!(json["ranked"][0]["importance"].is_number());
        assert!(json["ranked"][0]["centrality"].is_number());
        assert!(json["ranked"][0]["novelty"].is_number());
        assert!(json["ranked"][0]["text"].is_string());
        assert!(json["heavy_sentences"][0]["index"].is_number());
        assert!(json["heavy_sentences"][0]["importance"].is_number());
        assert!(json["heavy_sentences"][0]["centrality"].is_number());
        assert!(json["heavy_sentences"][0]["novelty"].is_number());
        assert!(json["heavy_sentences"][0]["text"].is_string());
        assert!(json["timeline"][0]["index"].is_number());
        assert!(json["bullet_points"][0]["sentence_index"].is_number());
        assert!(json["bullet_points"][0]["text"].is_string());
        assert!(json["summary"].is_string());
        assert!(json["bullet_points"].is_array());
        assert!(json["heavy_sentences"].is_array());
        assert!(json["timeline"].is_array());
        for stage in [
            "split_ms",
            "embedding_ms",
            "similarity_ms",
            "ranking_ms",
            "summary_ms",
            "bullet_ms",
            "total_ms",
        ] {
            assert!(json["timings"][stage].is_number(), "missing timing {stage}");
        }
    }

    #[test]
    fn event_cues_outrank_numeric_only_and_filler_heavy_sentences() {
        assert_eq!(content_signal("The company announced revenue.", "en"), 1.0);
        assert_eq!(content_signal("The meeting had 20 attendees.", "en"), 0.25);
        for numeric_fact in [
            "Profits reached $14.8 billion.",
            "Revenue grew by 35%.",
            "20,000 workers were affected.",
        ] {
            assert_eq!(content_signal(numeric_fact, "en"), 0.25, "{numeric_fact}");
        }
        assert_eq!(
            content_signal("The first result in 30 years was announced in 2026.", "en"),
            1.0
        );
        let clean_score = apply_noise_penalty("The company announced layoffs.", 0.8);
        let filler_score =
            apply_noise_penalty("Well, you know, um, the company announced layoffs.", 0.8);
        assert!(filler_score < clean_score);

        let lines = [
            "Well, you know, the company basically, uh, reported revenue growth.",
            "But the really important thing is they announced 5,000 layoffs.",
            "Executives discussed the plan.",
        ];
        let result = run(request(
            &lines,
            vec![vec3(1., 0., 0.), vec3(1., 0., 0.), vec3(1., 0., 0.)],
            Options {
                min_heavy: 3,
                max_heavy: 3,
                ..Default::default()
            },
        ))
        .unwrap();
        assert_eq!(result.ranked[0].text, lines[1]);

        let event_lines = [
            "The company held a meeting.",
            "Employees discussed the new strategy.",
            "Then the CEO announced that 20,000 employees would be laid off.",
            "The announcement shocked investors.",
            "The meeting ended later that afternoon.",
        ];
        let event_result = run(request(
            &event_lines,
            vec![
                vec3(0., 0., 1.),
                vec3(0., 1., 0.),
                vec3(1., 0., 0.),
                vec3(0.99, 0.01, 0.),
                vec3(0., 0., 1.),
            ],
            Options {
                min_heavy: 2,
                max_heavy: 2,
                top_fraction: Some(0.4),
                ..Options::default()
            },
        ))
        .unwrap();
        assert!(event_result.heavy_sentences.iter().any(|sentence| {
            sentence.text == "Then the CEO announced that 20,000 employees would be laid off."
        }));
        assert!(event_result.ranked.iter().any(|sentence| {
            sentence.text == "The announcement shocked investors." && sentence.importance >= 0.9
        }));
    }

    #[test]
    fn relevant_event_beats_novelty_outlier_and_duplicates_are_removed() {
        let lines = [
            "Apple reported record revenue.",
            "iPhone sales increased.",
            "The company expanded in Europe.",
            "My neighbor owns three cats.",
            "Apple expects more growth next year.",
        ];
        let result = run(request(
            &lines,
            vec![
                vec3(1.0, 0.0, 0.0),
                vec3(0.98, 0.1, 0.0),
                vec3(0.96, 0.2, 0.0),
                vec3(0.0, 1.0, 0.0),
                vec3(0.97, 0.15, 0.0),
            ],
            Options {
                min_heavy: 1,
                max_heavy: 5,
                top_fraction: Some(0.2),
                ..Default::default()
            },
        ))
        .unwrap();
        assert!(!result
            .heavy_sentences
            .iter()
            .any(|sentence| sentence.text.contains("neighbor")));
        let duplicate_lines = [
            "Apple reported record revenue.",
            "Apple reported record revenue.",
            "Apple reported record revenue.",
            "Sales increased strongly.",
            "The company announced a major acquisition.",
        ];
        let duplicates = run(request(
            &duplicate_lines,
            vec![
                vec3(1., 0., 0.),
                vec3(1., 0., 0.),
                vec3(1., 0., 0.),
                vec3(0.9, 0.1, 0.),
                vec3(0., 1., 0.),
            ],
            Options {
                min_heavy: 1,
                max_heavy: 5,
                ..Default::default()
            },
        ))
        .unwrap();
        assert_eq!(
            duplicates
                .heavy_sentences
                .iter()
                .filter(|sentence| sentence.text == duplicate_lines[0])
                .count(),
            1
        );

        let paraphrases = [
            "Apple reported record revenue.",
            "Apple posted record revenue.",
            "Apple announced record sales.",
            "Sales increased strongly.",
        ];
        let semantic_duplicates = run(request(
            &paraphrases,
            vec![
                vec3(1., 0., 0.),
                vec3(1., 0., 0.),
                vec3(1., 0., 0.),
                vec3(0., 1., 0.),
            ],
            Options {
                min_heavy: 1,
                max_heavy: 4,
                top_fraction: Some(1.0),
                ..Default::default()
            },
        ))
        .unwrap();
        assert!(
            semantic_duplicates
                .heavy_sentences
                .iter()
                .filter(|sentence| paraphrases[..3].contains(&sentence.text.as_str()))
                .count()
                <= 1
        );
    }

    #[test]
    fn summary_and_bullets_are_extractively_faithful_and_distinct_from_rank() {
        let lines = [
            "Tesla opened a new factory in Mexico.",
            "The factory will produce electric vehicles.",
            "Production is expected to begin next year.",
            "The project will employ 5,000 workers.",
        ];
        let result = run(request(
            &lines,
            vec![
                vec3(1., 0., 0.),
                vec3(0., 1., 0.),
                vec3(0., 0., 1.),
                vec3(-1., 0., 0.),
            ],
            Options {
                summary_length: SummaryLength::Short,
                min_heavy: 2,
                max_heavy: 2,
                bullet_count: 3,
                ..Default::default()
            },
        ))
        .unwrap();
        assert!(result.summary.contains("factory") && result.summary.contains("Mexico"));
        assert!(!result.summary.to_lowercase().contains("largest"));
        assert_ne!(
            result.summary,
            result
                .heavy_sentences
                .iter()
                .map(|sentence| sentence.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(result.bullet_points.len() <= 3);
        assert!(result
            .bullet_points
            .iter()
            .all(|bullet| lines.contains(&bullet.text.as_str())));
        let importance_by_index: Vec<f64> = (0..lines.len())
            .map(|index| {
                result
                    .ranked
                    .iter()
                    .find(|sentence| sentence.index == index)
                    .unwrap()
                    .importance
            })
            .collect();
        assert!(result.bullet_points.windows(2).all(|pair| {
            importance_by_index[pair[0].sentence_index]
                >= importance_by_index[pair[1].sentence_index]
        }));
        let polarity = ["The company did not declare bankruptcy."];
        let result = run(request(
            &polarity,
            vec![vec3(1., 0., 0.)],
            Options {
                min_heavy: 1,
                max_heavy: 1,
                summary_length: SummaryLength::Short,
                ..Default::default()
            },
        ))
        .unwrap();
        assert!(result.summary.contains("did not declare bankruptcy"));
    }

    #[test]
    fn extractive_summary_keeps_entity_associations_and_sentence_polarity() {
        let lines = [
            "Elon Musk discussed Tesla during the interview.",
            "Tim Cook discussed Apple during the interview.",
            "The company did not declare bankruptcy.",
        ];
        let result = run(request(
            &lines,
            orthogonal_embeddings(lines.len()),
            Options {
                summary_length: SummaryLength::Short,
                min_heavy: 1,
                max_heavy: 3,
                bullet_count: 3,
                ..Options::default()
            },
        ))
        .unwrap();
        assert!(result
            .summary
            .split_inclusive('.')
            .all(|sentence| lines.contains(&sentence.trim())));
        assert!(result.summary.contains("Elon Musk discussed Tesla"));
        assert!(result.summary.contains("Tim Cook discussed Apple"));
        assert!(result.summary.contains("did not declare bankruptcy"));
        assert!(result
            .bullet_points
            .iter()
            .all(|bullet| lines.contains(&bullet.text.as_str())));
    }

    #[test]
    fn timeline_is_chronological_and_ranked_is_descending() {
        let lines = [
            "Sentence at time five.",
            "Sentence at time twelve.",
            "Sentence at time twenty.",
        ];
        let result = run(Request {
            transcript: lines.join(" "),
            language: "en".into(),
            embeddings: vec![vec![0., 0., 1.], vec![1., 0., 0.], vec![0., 1., 0.]],
            timings: vec![
                Timing {
                    start_us: 5_000_000,
                    end_us: 6_000_000,
                },
                Timing {
                    start_us: 12_000_000,
                    end_us: 13_000_000,
                },
                Timing {
                    start_us: 20_000_000,
                    end_us: 21_000_000,
                },
            ],
            options: Options {
                min_heavy: 3,
                max_heavy: 3,
                ..Default::default()
            },
            embedding_ms: 5.0,
            lexical_only: false,
        })
        .unwrap();
        assert_eq!(result.ranked[0].index, 1);
        assert!(result
            .ranked
            .windows(2)
            .all(|pair| pair[0].importance >= pair[1].importance));
        assert_eq!(
            result
                .timeline
                .iter()
                .map(|sentence| sentence.start_sec.unwrap())
                .collect::<Vec<_>>(),
            [5., 12., 20.]
        );
        assert!((result.timings.total_ms - 5.0) >= result.timings.split_ms);
        assert!(
            result.timings.total_ms
                >= 5.0
                    + result.timings.split_ms
                    + result.timings.similarity_ms
                    + result.timings.ranking_ms
                    + result.timings.summary_ms
                    + result.timings.bullet_ms
        );
    }

    #[test]
    fn deterministic_across_one_hundred_runs_and_invalid_vectors_fail_closed() {
        let lines = [
            "The board approved a major acquisition.",
            "Revenue grew by 35% this year.",
            "The company expects further growth.",
            "The board met again later.",
        ];
        let input = request(
            &lines,
            vec![
                vec3(1., 0., 0.),
                vec3(0.9, 0.1, 0.),
                vec3(0.95, 0.05, 0.),
                vec3(0.8, 0.2, 0.),
            ],
            Options::default(),
        );
        let first = run(input.clone()).unwrap();
        for _ in 0..99 {
            let next = run(input.clone()).unwrap();
            assert_eq!(next.summary, first.summary);
            assert_eq!(next.ranked, first.ranked);
            assert_eq!(next.heavy_sentences, first.heavy_sentences);
            assert_eq!(next.bullet_points, first.bullet_points);
            assert_eq!(next.timeline, first.timeline);
        }

        let invalid = request(&lines, vec![vec3(0., 0., 0.); 4], Options::default());
        assert!(run(invalid).unwrap_err().contains("zero norm"));

        let invalid_cases = [
            (
                Request {
                    transcript: "Sentence one. Sentence two.".into(),
                    language: "en".into(),
                    embeddings: vec![vec3(1., 0., 0.)],
                    timings: Vec::new(),
                    options: Options::default(),
                    embedding_ms: 0.0,
                    lexical_only: false,
                },
                "embedding count",
            ),
            (
                Request {
                    transcript: "Sentence one.".into(),
                    language: "en".into(),
                    embeddings: vec![vec3(f32::NAN, 0., 0.)],
                    timings: Vec::new(),
                    options: Options::default(),
                    embedding_ms: 0.0,
                    lexical_only: false,
                },
                "non-finite",
            ),
            (
                Request {
                    transcript: "Sentence one.".into(),
                    language: "en".into(),
                    embeddings: vec![vec3(1., 0., 0.)],
                    timings: vec![Timing {
                        start_us: 2_000_000,
                        end_us: 1_000_000,
                    }],
                    options: Options::default(),
                    embedding_ms: 0.0,
                    lexical_only: false,
                },
                "invalid/nonchronological",
            ),
            (
                Request {
                    transcript: "Sentence one.".into(),
                    language: "en".into(),
                    embeddings: vec![vec3(1., 0., 0.)],
                    timings: Vec::new(),
                    options: Options {
                        top_fraction: Some(1.1),
                        ..Options::default()
                    },
                    embedding_ms: 0.0,
                    lexical_only: false,
                },
                "top_fraction",
            ),
        ];
        for (invalid, expected_error) in invalid_cases {
            assert!(
                run(invalid).unwrap_err().contains(expected_error),
                "expected validation error containing {expected_error}"
            );
        }
    }

    #[test]
    fn heavy_selection_obeys_fraction_and_minimum_maximum_for_transcript_sizes() {
        for (count, expected) in [(10, 3), (25, 4), (50, 7), (100, 13), (200, 15)] {
            let lines = labeled_test_sentences(count);
            let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
            let result = run(request(
                &borrowed,
                orthogonal_embeddings(count),
                Options {
                    min_heavy: 3,
                    max_heavy: 15,
                    top_fraction: Some(0.125),
                    ..Options::default()
                },
            ))
            .unwrap();
            assert_eq!(
                result.heavy_sentences.len(),
                expected,
                "unexpected selection count for {count} sentences"
            );
            assert!(result.heavy_sentences.len() <= 15);
            assert!(result
                .heavy_sentences
                .windows(2)
                .all(|pair| pair[0].importance >= pair[1].importance));
        }
    }

    #[test]
    fn one_thousand_word_transcript_produces_stable_summary_bullets_and_top_ten() {
        let sentences: Vec<String> = (0..50)
            .map(|index| {
                format!("Sentence {index} company announced financial results and increased production across international markets this year during its latest quarterly operating period.")
            })
            .collect();
        assert_eq!(
            sentences.iter().map(|line| word_count(line)).sum::<usize>(),
            1_000
        );
        let borrowed: Vec<&str> = sentences.iter().map(String::as_str).collect();
        let input = request(
            &borrowed,
            orthogonal_embeddings(sentences.len()),
            Options {
                summary_length: SummaryLength::Short,
                bullet_count: 5,
                min_heavy: 10,
                max_heavy: 10,
                top_fraction: Some(0.1),
            },
        );
        let first = run(input.clone()).unwrap();
        assert_eq!(first.ranked.len(), 50);
        assert_eq!(first.heavy_sentences.len(), 10);
        assert_eq!(first.bullet_points.len(), 5);
        assert!(!first.summary.is_empty());
        assert!(first.summary.split_whitespace().count() <= 80);
        assert!(first
            .bullet_points
            .iter()
            .all(|bullet| sentences.iter().any(|line| line == &bullet.text)));
        assert!(first.timings.total_ms.is_finite());
        for _ in 0..9 {
            let next = run(input.clone()).unwrap();
            assert_eq!(next.summary, first.summary);
            assert_eq!(next.ranked, first.ranked);
            assert_eq!(next.heavy_sentences, first.heavy_sentences);
            assert_eq!(next.bullet_points, first.bullet_points);
            assert_eq!(next.timeline, first.timeline);
            assert!(next.timings.total_ms.is_finite());
        }
    }

    #[test]
    fn bullet_count_modes_return_unique_source_sentences() {
        let lines = labeled_test_sentences(12);
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        for count in [3, 5, 10] {
            let result = run(request(
                &borrowed,
                orthogonal_embeddings(lines.len()),
                Options {
                    bullet_count: count,
                    ..Options::default()
                },
            ))
            .unwrap();
            assert_eq!(result.bullet_points.len(), count);
            for (index, bullet) in result.bullet_points.iter().enumerate() {
                assert!(lines.iter().any(|line| line == &bullet.text));
                assert!(result.bullet_points[..index]
                    .iter()
                    .all(|prior| prior.text != bullet.text));
            }
        }
    }

    #[test]
    fn summary_length_modes_stay_within_declared_word_bands_when_source_allows() {
        let lines = labeled_test_sentences(50);
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        for (mode, minimum, maximum) in [
            (SummaryLength::Short, 50, 80),
            (SummaryLength::Medium, 100, 150),
            (SummaryLength::Long, 200, 300),
        ] {
            let result = run(request(
                &borrowed,
                orthogonal_embeddings(lines.len()),
                Options {
                    summary_length: mode,
                    ..Options::default()
                },
            ))
            .unwrap();
            let words = word_count(&result.summary);
            assert!(
                (minimum..=maximum).contains(&words),
                "summary has {words} words, expected {minimum}..={maximum}"
            );
            for sentence in result.summary.split_inclusive('.') {
                assert!(lines.iter().any(|line| line == sentence.trim()));
            }
        }
    }

    #[test]
    fn synthetic_labeled_corpus_computes_precision_and_recall_transparently() {
        #[derive(Deserialize)]
        struct Corpus {
            dataset_type: String,
            label_semantics: String,
            embedding_note: String,
            cases: Vec<Case>,
        }
        #[derive(Deserialize)]
        struct Case {
            sentences: Vec<Label>,
        }
        #[derive(Deserialize)]
        struct Label {
            text: String,
            label: u8,
            embedding_group: String,
        }
        let corpus: Corpus =
            serde_json::from_str(include_str!("../fixtures/phrase_impact_labels.json")).unwrap();
        assert_eq!(
            corpus.dataset_type,
            "synthetic_developer_authored_not_human_evaluation"
        );
        assert!(corpus.label_semantics.contains("not human validation"));
        assert!(corpus.embedding_note.contains("not model embeddings"));
        assert_eq!(corpus.cases.len(), 33);
        let case_count = corpus.cases.len();
        let mut p5_hits = 0;
        let mut p10_hits = 0;
        let mut relevant = 0;
        let mut very_important = 0;
        let mut r10_hits = 0;
        for case in corpus.cases {
            let vectors = case
                .sentences
                .iter()
                .map(|sentence| match sentence.embedding_group.as_str() {
                    "topic" => vec![1., 0., 0.],
                    "outlier" => vec![0., 1., 0.],
                    _ => panic!("bad embedding group"),
                })
                .collect();
            let text = case
                .sentences
                .iter()
                .map(|sentence| sentence.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let result = run(Request {
                transcript: text,
                language: "en".into(),
                embeddings: vectors,
                timings: Vec::new(),
                options: Options {
                    min_heavy: 1,
                    max_heavy: 12,
                    ..Default::default()
                },
                embedding_ms: 0.0,
                lexical_only: false,
            })
            .unwrap();
            let labels: Vec<bool> = case
                .sentences
                .iter()
                .map(|sentence| {
                    assert!(sentence.label <= 2);
                    sentence.label > 0
                })
                .collect();
            assert_eq!(labels.len(), 12);
            assert!(labels.iter().filter(|&&label| label).count() >= 3);
            relevant += labels.iter().filter(|&&label| label).count();
            very_important += case
                .sentences
                .iter()
                .filter(|sentence| sentence.label == 2)
                .count();
            p5_hits += result
                .ranked
                .iter()
                .take(5)
                .filter(|sentence| labels[sentence.index])
                .count();
            p10_hits += result
                .ranked
                .iter()
                .take(10)
                .filter(|sentence| labels[sentence.index])
                .count();
            r10_hits += result
                .ranked
                .iter()
                .take(10)
                .filter(|sentence| labels[sentence.index])
                .count();
        }
        assert!(very_important > 0, "fixture must include label 2");
        let p5 = p5_hits as f64 / (case_count * 5) as f64;
        let p10 = p10_hits as f64 / (case_count * 10) as f64;
        let r10 = r10_hits as f64 / relevant as f64;
        eprintln!(
            "synthetic fixture only; not human evidence: P@5={p5:.3}, P@10={p10:.3}, R@10={r10:.3}"
        );
        // These loose checks protect only the synthetic harness wiring. They
        // are not evidence for the requested human-judged Precision@K targets.
        assert!(p5 > 0.45, "synthetic regression P@5={p5:.3}");
        assert!(p10 > 0.25, "synthetic regression P@10={p10:.3}");
        assert!(r10 >= 0.75, "synthetic regression R@10={r10:.3}");
    }
}
