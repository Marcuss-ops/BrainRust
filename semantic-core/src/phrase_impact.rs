use serde::{Deserialize, Serialize};

use std::collections::{HashMap, HashSet};
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
/// A scene bullet must reach this many words to be published: two-word tails
/// like "impronta 33." are fragments, not points. Scripts without spaces are
/// measured by MIN_BULLET_CHARS instead, so a whole CJK sentence stays
/// eligible where word counting cannot see token boundaries.
const MIN_BULLET_WORDS: usize = 4;
/// Alphanumeric-character floor for the same substance rule (CJK text).
const MIN_BULLET_CHARS: usize = 16;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLength {
    Short,
    #[default]
    Medium,
    Long,
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
    /// Verbatim numeric tokens extracted from this scene for grounded charts.
    #[serde(default)]
    pub chart_values: Vec<String>,
    /// True when scene slice verified against transcript (global offsets valid).
    pub globally_indexed: bool,
}

/// Extracts strict numeric literals from source text; no number words or
/// inferred values are introduced. Runtime must still match these tokens to
/// grounded, audio-timed annotations before rendering a chart.
pub fn extract_chart_values(text: &str, language: &str) -> Vec<String> {
    let mut values = Vec::new();
    for token in text.split_whitespace() {
        let candidate = token.trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '[' | ']' | '{' | '}' | ':' | ';' | '"' | '\'' | '.' | ',' | '!' | '?'
            )
        });
        // Phrase Impact retains its literal-span policy, while the shared
        // semantic core owns locale-aware parsing and ambiguity rejection.
        let numeric = candidate.trim_matches(|c: char| matches!(c, '%' | '€' | '$' | '£' | '¥'));
        if crate::parse_unambiguous_value(numeric, language).is_none()
            || values.iter().any(|existing| existing == candidate)
        {
            continue;
        }
        values.push(candidate.to_string());
        if values.len() >= 6 {
            break;
        }
    }
    values
}

fn scene_chart_values(scene: &SceneInput, language: &str) -> Vec<String> {
    let values = extract_chart_values(&scene.text, language);
    if values.len() >= 2 {
        values
    } else {
        Vec::new()
    }
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
    /// Optional source-grounded candidates from the unified semantic engine.
    /// `None` preserves the legacy candidate source; `Some([])` deliberately
    /// means there are no eligible semantic bullets for this transcript.
    #[serde(default)]
    pub semantic_bullet_spans: Option<Vec<SemanticBulletSpan>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct SemanticBulletSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
    /// Shared salience from `crate::bullets`. Absent on callers that only
    /// ground spans (legacy standalone worker); the semantic worker fills it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
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

mod chapters;
mod graph;
mod highlights;
mod scoring;
mod selection;
mod sentences;

use chapters::*;
use graph::*;
use highlights::*;
use scoring::*;
use selection::*;
pub use sentences::*;

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
        request.semantic_bullet_spans.as_deref(),
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
    if !resolved_scenes.is_empty()
        && !resolved_topics.is_empty()
        && resolved_topics.len() != resolved_scenes.len()
    {
        // Scene metadata is optional context for chapter labels, never a
        // reason to alter or suppress the legacy extractive products.
        resolved_scenes = &[];
        resolved_topics = &[];
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
    // Global bullets obey the same extraction contract as the scene and
    // chapter surfaces: a bullet is a complete, substantive sentence — never a
    // fragment and never a mid-sentence chunk of a sentence longer than
    // MAX_SEGMENT_WORDS.
    let segment_texts: Vec<&str> = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    let (allowed_semantic_bullets, semantic_bullet_scores) = match &request.semantic_bullet_spans {
        None => (None, HashMap::new()),
        Some(spans) => {
            let mut allowed = std::collections::HashSet::new();
            let mut scores: HashMap<usize, f32> = HashMap::new();
            for span in spans {
                if span.start_byte >= span.end_byte
                    || span.end_byte > request.transcript.len()
                    || !request.transcript.is_char_boundary(span.start_byte)
                    || !request.transcript.is_char_boundary(span.end_byte)
                    || request.transcript[span.start_byte..span.end_byte] != span.text
                {
                    return Err("invalid semantic bullet span".into());
                }
                for (index, segment) in segments.iter().enumerate() {
                    if segment.start_byte == span.start_byte && segment.end_byte == span.end_byte {
                        allowed.insert(index);
                        if let Some(score) = span.score {
                            scores.insert(index, score);
                        }
                    }
                }
            }
            (Some(allowed), scores)
        }
    };
    let mut bullet_candidates: Vec<usize> = rank_indices
        .iter()
        .copied()
        .filter(|&index| {
            bullet_starts_sentence(&segment_texts, index)
                && bullet_completes_sentence(&segment_texts, index)
                && bullet_has_substance(segment_texts[index])
                && allowed_semantic_bullets
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(&index))
        })
        .collect();
    // The semantic worker already ranked these candidates with the shared
    // `bullets` salience. Combine it with the embedding importance through the
    // same `editorial_priority` function instead of keeping two rankings.
    if !semantic_bullet_scores.is_empty() {
        bullet_candidates.sort_by(|&a, &b| {
            let priority = |index: usize| {
                crate::bullets::editorial_priority(
                    source_order[index].importance,
                    semantic_bullet_scores.get(&index).copied().unwrap_or(0.0),
                )
            };
            priority(b).total_cmp(&priority(a)).then(a.cmp(&b))
        });
    }
    let bullets = select_diverse(&bullet_candidates, &source_order, &vectors, bullet_limit)
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
#[path = "phrase_impact_tests/mod.rs"]
mod tests;
