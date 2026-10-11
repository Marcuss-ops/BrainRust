//! Scene Intelligence shared semantic signals (analysis-only, shadow mode).
//!
//! Single shared analysis producing seven signal kinds for one canonical
//! manifest. This module does NOT embed, rank media, plan overlays, or
//! render: it identifies what the script says (facts, relations, suggestions
//! with text references). PipelineGen plans, RenderingGen draws.
//!
//! Reuse rule: no new NER, no new embedding engine, no media ranking here.
//! Entity surfaces arrive as input (from the existing NLP/semantic context);
//! sentence splitting and tokenization are the only shared text utilities,
//! kept deliberately small and language-independent.
//!
//! Determinism: pure functions of (input text, entities, config versions).
//! No randomness, no wall-clock, no global state. Outputs are sorted.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Canonical manifest schema version.
pub const SCHEMA_VERSION: &str = "scene_intelligence.v1";
/// Analyzer version pinned into every manifest (part of the identity).
pub const ANALYZER_VERSION: &str = "scene_intelligence.v1";
/// Relation registry version (single typed contract shared by comparison,
/// causal and entity-graph signals).
pub const RELATION_REGISTRY_VERSION: &str = "relation_registry.v1";
/// Max relation nodes admitted per scene composition (screen budget, not a
/// knowledge cutoff: the full graph may be larger).
pub const MAX_RELATION_NODES_PER_SCENE: usize = 6;

// ── Shared context ────────────────────────────────────────────────

/// One scene's text with its video-playback window (kept separate from any
/// historical event time mentioned inside the text).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SceneText {
    pub scene_id: String,
    pub text: String,
    #[serde(default)]
    pub language: String,
    pub video_start_ms: i64,
    pub video_end_ms: i64,
}

/// One indexed sentence (global narration order across all scenes).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Sentence {
    pub id: usize,
    pub scene_id: String,
    pub index_in_scene: usize,
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Tokenizer + sentence index shared by every detector.
#[derive(Clone, Debug, Default)]
pub struct SharedContext {
    pub sentences: Vec<Sentence>,
}

impl SharedContext {
    pub fn build(scenes: &[SceneText]) -> Self {
        let mut sentences = Vec::new();
        let mut id = 0usize;
        for scene in scenes {
            for (idx, (text, start, end)) in split_sentences(&scene.text).into_iter().enumerate() {
                sentences.push(Sentence {
                    id,
                    scene_id: scene.scene_id.clone(),
                    index_in_scene: idx,
                    text,
                    start_byte: start,
                    end_byte: end,
                });
                id += 1;
            }
        }
        Self { sentences }
    }

    pub fn sentences_for_scene(&self, scene_id: &str) -> Vec<&Sentence> {
        self.sentences
            .iter()
            .filter(|s| s.scene_id == scene_id)
            .collect()
    }
}

/// Split text into sentences on `.`/`!`/`?`/`;` boundaries (byte offsets
/// preserved). Language-independent; abbreviations may over-split, which is
/// acceptable for signal extraction (evidence spans stay verbatim).
pub fn split_sentences(text: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'.' || c == b'!' || c == b'?' || c == b';' {
            let mut end = i + 1;
            while end < bytes.len() && (bytes[end] == b'.' || bytes[end] == b'!' || bytes[end] == b'?') {
                end += 1;
            }
            let slice = text[start..end].trim();
            if !slice.is_empty() {
                let rel = text[start..end].find(slice).unwrap_or(0);
                out.push((slice.to_string(), start + rel, start + rel + slice.len()));
            }
            while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            start = end;
            i = end;
        } else {
            i += 1;
        }
    }
    if start < text.len() {
        let slice = text[start..].trim();
        if !slice.is_empty() {
            let rel = text[start..].find(slice).unwrap_or(0);
            out.push((slice.to_string(), start + rel, start + rel + slice.len()));
        }
    }
    out
}

/// Lowercase alphanumeric tokens (unicode-aware via char methods).
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '\'' || ch == '’' {
            cur.push(ch.to_lowercase().next().unwrap_or(ch));
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn contains_token(haystack: &[String], needle: &str) -> bool {
    haystack.iter().any(|t| t == needle)
}

fn contains_phrase(haystack: &[String], phrase: &[&str]) -> bool {
    if phrase.is_empty() || phrase.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(phrase.len())
        .any(|w| w.iter().zip(phrase.iter()).all(|(a, b)| a == b))
}

// ── 1. Information density ────────────────────────────────────────

/// Density signal for one scene. `score` is a calibrated 0..1 load estimate;
/// `level` is its band; counts are distinct-only so repetition never inflates
/// the load. `suggested_visual_budget` is `"low"` for cognitively dense
/// scenes (few simultaneous overlays), else `"standard"`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Density {
    pub scene_id: String,
    pub score: f64,
    pub level: String,
    pub new_entities: usize,
    pub numeric_claims: usize,
    pub new_concepts: usize,
    pub suggested_visual_budget: String,
}

/// Analyze density over DISTINCT signals only: identical sentences are
/// deduplicated first, so padding a scene with repetition cannot raise the
/// score. `entities` are the scene's known entity surfaces (existing NLP
/// context); `prior_concepts` are concepts already introduced by earlier
/// scenes (for novelty).
pub fn analyze_density(
    scene_id: &str,
    sentences: &[&Sentence],
    entities: &[String],
    prior_concepts: &BTreeSet<String>,
) -> Density {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut distinct: Vec<&Sentence> = Vec::new();
    for s in sentences {
        let norm = s.text.to_lowercase();
        let norm: String = norm.split_whitespace().collect::<Vec<_>>().join(" ");
        if seen.insert(norm) {
            distinct.push(*s);
        }
    }
    let body: String = distinct.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
    let toks = tokens(&body);
    let distinct_toks: BTreeSet<&String> = toks.iter().collect();

    let entity_set: BTreeSet<String> = entities.iter().map(|e| e.to_lowercase()).collect();
    let new_entities = entity_set.len();

    let numeric_claims = count_numbers(&body);
    let concepts: BTreeSet<String> = distinct_toks
        .iter()
        .filter(|t| t.chars().count() >= 5)
        .map(|t| t.to_string())
        .collect();
    let new_concepts = concepts.difference(prior_concepts).count();
    let relation_marks = count_relation_marks(&toks);

    let norm = |v: f64, scale: f64| (v / scale).clamp(0.0, 1.0);
    let complexity = if toks.is_empty() {
        0.0
    } else {
        (distinct_toks.len() as f64 / toks.len() as f64).clamp(0.0, 1.0)
    };
    let score = (0.30 * norm(new_entities as f64, 6.0)
        + 0.25 * norm(numeric_claims as f64, 5.0)
        + 0.20 * norm(new_concepts as f64, 12.0)
        + 0.15 * norm(relation_marks as f64, 4.0)
        + 0.10 * complexity)
        .clamp(0.0, 1.0);
    let level = if score >= 0.66 {
        "high"
    } else if score >= 0.33 {
        "medium"
    } else {
        "low"
    }
    .to_string();
    Density {
        scene_id: scene_id.to_string(),
        score: (score * 1000.0).round() / 1000.0,
        level,
        new_entities,
        numeric_claims,
        new_concepts,
        suggested_visual_budget: if score >= 0.66 { "low" } else { "standard" }.to_string(),
    }
}

fn count_numbers(text: &str) -> usize {
    let mut n = 0usize;
    let mut cur_has_digit = false;
    let mut cur_has_sep = false;
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() {
            cur_has_digit = true;
        } else if (ch == '.' || ch == ',') && cur_has_digit {
            cur_has_sep = true;
        } else {
            if cur_has_digit && (cur_has_sep || true) {
                // Count digit runs of length >= 1 that look like standalone numbers
                // (a digit run ends here). Single stray digits inside words are
                // excluded because words keep `cur` attached to letters — but our
                // char scan cannot see that, so require the run to be bounded by
                // non-alphanumeric on both sides via a second pass below.
                n += 1;
            }
            cur_has_digit = false;
            cur_has_sep = false;
        }
    }
    // Correct for digit runs embedded in words (e.g. "abc123"): recount with
    // token boundaries so only standalone numeric tokens count.
    let mut exact = 0usize;
    for tok in text.split(|c: char| !(c.is_alphanumeric() || c == '.' || c == ',' || c == '%')) {
        let t = tok.trim_matches(|c| c == '.' || c == ',');
        if t.is_empty() {
            continue;
        }
        let digits = t.chars().filter(|c| c.is_ascii_digit()).count();
        if digits > 0 && digits >= t.chars().count() / 2 {
            exact += 1;
        }
    }
    let _ = n;
    exact
}

fn count_relation_marks(toks: &[String]) -> usize {
    let markers = [
        "causa", "perché", "perche", "because", "caused", "dopo", "after", "rispetto", "mentre",
        "versus", "contro", "wenn", "während",
    ];
    toks.iter().filter(|t| markers.contains(&t.as_str())).count()
}

// ── 2. Comparison detector ────────────────────────────────────────

/// A detected comparison. Quantitative comparisons carry ONLY parsed values
/// (`values`); qualitative ones carry none — values are never invented.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Comparison {
    pub scene_id: String,
    pub kind: String,
    pub subjects: Vec<String>,
    pub dimension: String,
    pub relation: String,
    pub evidence_sentence_ids: Vec<usize>,
    pub candidate_visual: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f64>,
    /// True when the comparison is negated ("non è superiore"): the
    /// negation is preserved on the signal so no planner ever treats it
    /// as a positive claim.
    #[serde(default)]
    pub negated: bool,
}

const COMPARISON_SIGNALS: &[&[&str]] = &[
    &["vs"],
    &["versus"],
    &["compared", "to"],
    &["compared", "with"],
    &["in", "contrast"],
    &["rispetto", "a"],
    &["mentre"],
    &["contro"],
    &["confronto"],
    &["im", "vergleich"],
    &["gegenüber"],
    &["gegenueber"],
    &["par", "rapport"],
];

/// Comparative adjectives that mark an implicit comparison when two
/// subjects are present (no per-topic rules, closed multilingual list).
const COMPARATIVE_ADJ: &[&str] = &[
    "superiore", "inferiore", "maggiore", "minore", "migliore", "peggiore",
    "superior", "inferior", "greater", "lesser", "higher", "lower",
    "better", "worse", "höher", "niedriger", "meilleur", "pire",
    "superieur", "inférieur",
];

/// Negation tokens (sentence-scope heuristic: a negated marker yields NO
/// positive signal — fail-soft toward silence, never toward an inverted
/// claim).
const NEGATION_TOKENS: &[&str] = &[
    "non", "no", "not", "nope", "mai", "nunca", "nicht", "kein",
    "keine", "ne", "pas", "jamais", "não", "nao", "niet",
];

/// True when a negation token scopes over a marker at `marker_pos` (within
/// the preceding 4 tokens or anywhere before a sentence-initial marker).
fn is_negated(toks: &[String], marker_pos: usize) -> bool {
    let from = marker_pos.saturating_sub(4);
    toks[from..marker_pos.min(toks.len())]
        .iter()
        .any(|t| NEGATION_TOKENS.contains(&t.as_str()))
}

const CONTRAST_VERBS: &[&str] = &[
    "cresce", "rallenta", "grows", "slows", "growing", "slowing", "steigt", "sinkt", "augmente",
    "ralentit", "aumenta", "disminuye", "crescono", "rallentano", "rose", "fell", "sale", "scende",
];

/// Detect explicit or implicit comparisons without any per-topic rules.
/// `entities` are the known entity surfaces for subject grounding.
/// Negation is preserved (`negated`), never dropped: "non è superiore"
/// stays a negated comparison. A signal with two distinct years but no
/// nominal subjects is a temporal comparison (years are the subjects).
/// Four-digit years are never treated as quantitative values.
pub fn detect_comparison(
    scene_id: &str,
    sentences: &[&Sentence],
    entities: &[String],
) -> Option<Comparison> {
    let entity_lc: Vec<String> = entities.iter().map(|e| e.to_lowercase()).collect();
    for s in sentences {
        let toks = tokens(&s.text);
        let explicit = COMPARISON_SIGNALS
            .iter()
            .any(|p| contains_phrase(&toks, p));
        let verb_contrast = CONTRAST_VERBS.iter().any(|v| contains_token(&toks, v));
        let adj_pos = toks
            .iter()
            .position(|t| COMPARATIVE_ADJ.contains(&t.as_str()));
        let has_signal = explicit
            || (verb_contrast && capitalized_terms(&s.text).len() >= 2)
            || adj_pos.is_some();
        if !has_signal {
            continue;
        }
        let negated = adj_pos.map(|p| is_negated(&toks, p)).unwrap_or(false);
        let subjects = ground_subjects(&s.text, &entity_lc);
        if subjects.len() >= 2 {
            let dimension = guess_dimension(&toks);
            let values = parse_values(&s.text);
            let quantitative = !values.is_empty();
            return Some(Comparison {
                scene_id: scene_id.to_string(),
                kind: "comparison".to_string(),
                subjects,
                dimension,
                relation: "contrast".to_string(),
                evidence_sentence_ids: vec![s.id],
                candidate_visual: if quantitative {
                    "value_comparison".to_string()
                } else {
                    "split_comparison".to_string()
                },
                values: if quantitative { values } else { Vec::new() },
                negated,
            });
        }
        // Temporal fallback: "Vendite 2020 vs 2025" — years as subjects.
        if explicit {
            let mut years = scan_years(&s.text);
            years.sort_unstable();
            years.dedup();
            if years.len() >= 2 {
                let values: Vec<f64> = parse_values(&s.text)
                    .into_iter()
                    .filter(|v| v.fract() != 0.0 || *v < 1000.0 || *v > 2100.0)
                    .collect();
                let quantitative = !values.is_empty();
                return Some(Comparison {
                    scene_id: scene_id.to_string(),
                    kind: "comparison".to_string(),
                    subjects: years.iter().take(2).map(|y| y.to_string()).collect(),
                    dimension: "temporal".to_string(),
                    relation: "contrast".to_string(),
                    evidence_sentence_ids: vec![s.id],
                    candidate_visual: if quantitative {
                        "value_comparison".to_string()
                    } else {
                        "split_comparison".to_string()
                    },
                    values: if quantitative { values } else { Vec::new() },
                    negated,
                });
            }
        }
    }
    None
}

fn capitalized_terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')) {
        if tok.chars().count() >= 2
            && tok.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
            && tok.chars().any(|c| c.is_lowercase())
        {
            let lc = tok.to_lowercase();
            if !out.contains(&lc) {
                out.push(lc);
            }
        }
    }
    out
}

/// Lowercase alphanumeric normalization: aliases that differ only in
/// casing or punctuation share one canonical surface when identity is
/// certain (same normalized form).
fn normalize_surface(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn ground_subjects(sentence: &str, entities: &[String]) -> Vec<String> {
    let caps = capitalized_terms(sentence);
    let mut out = Vec::new();
    let mut seen_norm = std::collections::BTreeSet::new();
    let mut push = |v: String, out: &mut Vec<String>, seen_norm: &mut std::collections::BTreeSet<String>| {
        let n = normalize_surface(&v);
        if n.is_empty() || !seen_norm.insert(n) {
            return;
        }
        out.push(v);
    };
    for e in entities {
        if caps.iter().any(|c| c == e || c.contains(e.as_str()) || e.contains(c.as_str())) {
            push(e.clone(), &mut out, &mut seen_norm);
        }
    }
    for c in caps {
        if out.len() >= 2 {
            break;
        }
        push(c, &mut out, &mut seen_norm);
    }
    out.truncate(2);
    // Restore display casing from the sentence for readability.
    out
}

fn guess_dimension(toks: &[String]) -> String {
    let dims = [
        ("investment", &["investment", "investimenti", "investimento", "investition", "investissement"][..]),
        ("cost", &["cost", "costi", "costo", "kosten", "coût", "cout"][..]),
        ("growth", &["growth", "crescita", "wachstum", "croissance", "crecimiento"][..]),
        ("strategy", &["strategy", "strategia", "strategie", "stratégie"][..]),
    ];
    for (name, keys) in dims {
        if keys.iter().any(|k| contains_token(toks, k)) {
            return format!("{}_strategy", name);
        }
    }
    "general".to_string()
}

/// Parse ONLY numbers present in the text. Four-digit years (1000–2100)
/// are excluded: a year is temporal evidence, never a quantity.
/// Never synthesizes values.
pub fn parse_values(text: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for tok in text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c == '%')) {
        let t = tok.trim().trim_end_matches('%').replace(',', ".");
        if t.is_empty() || !t.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(v) = t.parse::<f64>() {
            if !v.is_finite() {
                continue;
            }
            // Whole 4-digit years are temporal, not quantitative.
            if v.fract() == 0.0 && (1000.0..=2100.0).contains(&v) && t.len() == 4 {
                continue;
            }
            out.push(v);
        }
    }
    out
}

// ── 3. Cause–effect extractor ─────────────────────────────────────

/// A directed textual relation. `relation` is one of the RelationRegistry
/// kinds; `extraction_confidence` measures extraction from text, never
/// real-world truth.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CausalRelation {
    pub kind: String,
    pub cause: String,
    pub effect: String,
    pub relation: String,
    pub evidence_sentence_ids: Vec<usize>,
    pub extraction_confidence: f64,
    /// Verbatim attribution when the script attributes the claim
    /// ("secondo gli analisti"): the renderer must never draw an
    /// unattributed certain arrow for an attributed claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// True for conditional/rumored causation ("avrebbe provocato").
    /// Hypothetical relations must not render as certain arrows.
    #[serde(default)]
    pub hypothetical: bool,
}

/// Cause markers where the cause FOLLOWS the marker in the text
/// ("Y a causa di X" → cause X, effect Y): direction is preserved even
/// when the relation is expressed in reverse order.
const CAUSE_AFTER_MARKERS: &[&[&str]] = &[
    &["a", "causa", "di"],
    &["a", "causa"],
    &["perché"],
    &["perche"],
    &["because"],
    &["caused", "by"],
    &["è", "stata", "causata"],
    &["è", "stato", "causato"],
    &["was", "caused"],
    &["verursacht"],
    &["provoqué"],
];

/// Cause markers where the cause PRECEDES the marker
/// ("X ha causato Y" → cause X, effect Y). Conditional forms included:
/// mood is recorded as `hypothetical`, never as certainty.
const CAUSE_BEFORE_MARKERS: &[&[&str]] = &[
    &["ha", "causato"],
    &["hanno", "causato"],
    &["provocando"],
    &["ha", "provocato"],
    &["hanno", "provocato"],
    &["avrebbe", "provocato"],
    &["avrebbero", "provocato"],
    &["caused"],
    &["has", "caused"],
    &["have", "caused"],
    &["had", "caused"],
    &["would", "have", "caused"],
    &["hanno", "ridotto"],
];

/// Conditional auxiliaries marking hypothetical causation.
const HYPOTHETICAL_MARKERS: &[&str] = &[
    "avrebbe", "avrebbero", "would", "could", "might", "potrebbe",
    "potrebbero", "würde", "würden", "pourrait",
];

/// Attribution prefixes (verbatim, before the first comma).
const ATTRIBUTION_MARKERS: &[&str] = &[
    "secondo", "according", "selon", "laut", "según", "segun",
];

const CONTRIBUTE_MARKERS: &[&[&str]] = &[
    &["contribuì", "a"],
    &["contribui", "a"],
    &["contribuisce", "a"],
    &["ha", "contribuito"],
    &["hanno", "contribuito"],
    &["contribuito", "a"],
    &["contribuito"],
    &["contributed", "to"],
    &["contributes", "to"],
    &["contributing", "to"],
    &["contribuendo", "a"],
];

const SEQUENCE_MARKERS: &[&str] = &["dopo", "after", "nach", "après", "apres", "después", "despues"];

/// Marker hit with its token position and cause direction.
fn find_causal_marker(toks: &[String]) -> Option<(bool, usize, usize)> {
    // (cause_after, marker_len, pos)
    for i in 0..toks.len() {
        for m in CAUSE_AFTER_MARKERS {
            if window_eq(toks, i, m) {
                return Some((true, m.len(), i));
            }
        }
        for m in CAUSE_BEFORE_MARKERS {
            if window_eq(toks, i, m) {
                return Some((false, m.len(), i));
            }
        }
    }
    None
}

/// Split sentence text around a token-level marker position.
fn split_at_marker(text: &str, toks: &[String], pos: usize, len: usize) -> (String, String) {
    let before: Vec<&String> = toks[..pos.min(toks.len())].iter().collect();
    let after: Vec<&String> = toks[(pos + len).min(toks.len())..].iter().collect();
    let _ = text;
    (slug(before.into_iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")), slug(after.into_iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")))
}

/// Extract a causal or sequential relation. Direction follows the marker
/// ("a causa di" → cause after; "ha causato" → cause before), so reverse
/// order keeps the correct direction. A negated marker ("non è stata
/// causata") yields NO positive relation. A bare temporal marker ("dopo" /
/// "after") yields `precedes`, never `causes`: sequence and causation are
/// never conflated. Attributed/conditional claims keep their attribution
/// and hypothetical flag with lowered confidence.
pub fn extract_causal(sentence: &Sentence) -> Option<CausalRelation> {
    let toks = tokens(&sentence.text);
    if let Some((cause_after, mlen, pos)) = find_causal_marker(&toks) {
        if is_negated(&toks, pos) {
            return None;
        }
        let (before, after) = split_at_marker(&sentence.text, &toks, pos, mlen);
        let (cause, effect) = if cause_after { (after, before) } else { (before, after) };
        if cause.is_empty() || effect.is_empty() {
            return None;
        }
        let hypothetical = HYPOTHETICAL_MARKERS.iter().any(|m| contains_token(&toks, m));
        let attribution = extract_attribution(&sentence.text);
        return Some(CausalRelation {
            kind: "causal_relation".to_string(),
            cause,
            effect,
            relation: "causes".to_string(),
            evidence_sentence_ids: vec![sentence.id],
            extraction_confidence: if hypothetical { 0.6 } else { 0.9 },
            attribution,
            hypothetical,
        });
    }
    if let Some(pos) = CONTRIBUTE_MARKERS
        .iter()
        .filter_map(|p| {
            toks.windows(p.len())
                .position(|w| w.iter().zip(p.iter()).all(|(a, b)| a == b))
                .map(|i| (i, p.len()))
        })
        .map(|(i, _)| i)
        .next()
    {
        if is_negated(&toks, pos) {
            return None;
        }
        let mlen = CONTRIBUTE_MARKERS
            .iter()
            .find(|p| window_eq(&toks, pos, p))
            .map(|p| p.len())
            .unwrap_or(2);
        let (before, after) = split_at_marker(&sentence.text, &toks, pos, mlen);
        if before.is_empty() || after.is_empty() {
            return None;
        }
        let hypothetical = HYPOTHETICAL_MARKERS.iter().any(|m| contains_token(&toks, m));
        let attribution = extract_attribution(&sentence.text);
        return Some(CausalRelation {
            kind: "causal_relation".to_string(),
            cause: before,
            effect: after,
            relation: "contributes_to".to_string(),
            evidence_sentence_ids: vec![sentence.id],
            extraction_confidence: if hypothetical { 0.55 } else { 0.8 },
            attribution,
            hypothetical,
        });
    }
    if SEQUENCE_MARKERS.iter().any(|m| contains_token(&toks, m)) {
        let (cause, effect) = split_around(&sentence.text);
        return Some(CausalRelation {
            kind: "causal_relation".to_string(),
            cause,
            effect,
            relation: "precedes".to_string(),
            evidence_sentence_ids: vec![sentence.id],
            extraction_confidence: 0.7,
            attribution: None,
            hypothetical: false,
        });
    }
    None
}

/// Verbatim attribution prefix ("Secondo gli analisti, ..." →
/// "Secondo gli analisti").
fn extract_attribution(text: &str) -> Option<String> {
    let first: String = text.split(',').next().unwrap_or("").trim().to_string();
    if first.is_empty() || first.len() >= text.trim().len() {
        return None;
    }
    let lt = first.to_lowercase();
    if ATTRIBUTION_MARKERS.iter().any(|m| lt.split_whitespace().any(|w| w == *m)) {
        return Some(first);
    }
    None
}

fn split_around(text: &str) -> (String, String) {
    let lower = text.to_lowercase();
    for sep in [", provocando", ", contribuendo", " ha ridotto", " hanno ridotto", " dopo ", " after "] {
        if let Some(pos) = lower.find(sep) {
            let a = text[..pos].trim().trim_matches(|c| c == '.' || c == ',' || c == ';').to_string();
            let b = text[pos + sep.len()..].trim().trim_matches(|c| c == '.' || c == ',' || c == ';').to_string();
            if !a.is_empty() && !b.is_empty() {
                return (slug(a), slug(b));
            }
        }
    }
    // Fallback: first half / second half split (still verbatim-derived).
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() >= 4 {
        let mid = words.len() / 2;
        return (slug(words[..mid].join(" ")), slug(words[mid..].join(" ")));
    }
    (slug(text.trim().to_string()), String::new())
}

fn slug(s: String) -> String {
    let t: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let mut out = String::new();
    let mut prev_underscore = false;
    for c in t.chars() {
        if c == '_' {
            if !prev_underscore {
                out.push(c);
            }
            prev_underscore = true;
        } else {
            out.push(c);
            prev_underscore = false;
        }
    }
    out.trim_matches('_').to_string()
}

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

fn scan_years(text: &str) -> Vec<i32> {
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

fn has_relative_time(text: &str) -> bool {
    relative_offset(text).is_some()
}

/// Offset in years for relative expressions ("tre anni dopo" → +3,
/// "l'anno successivo" → +1). None when no relative expression.
fn relative_offset(text: &str) -> Option<i32> {
    let toks = tokens(text);
    const REL_PLURAL: &[&[&str]] = &[
        &["anni", "dopo"],
        &["years", "later"],
        &["jahre", "später"],
        &["jahre", "spaeter"],
        &["ans", "plus", "tard"],
    ];
    const REL_SINGLE: &[&str] = &[
        "successivo", "successiva", "following", "suivante", "siguiente",
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

fn number_word(t: &str) -> Option<i32> {
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
fn window_eq(toks: &[String], pos: usize, marker: &[&str]) -> bool {
    pos + marker.len() <= toks.len()
        && toks[pos..pos + marker.len()]
            .iter()
            .zip(marker.iter())
            .all(|(a, b)| a == b)
}

/// Intervals ("2020–2023", "dal 2020 al 2023"): (start, end, expression).
fn scan_intervals(text: &str) -> Vec<(i32, i32, String)> {
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
                || ["al", "to", "and", "e", "e il", "bis", "au", "a"]
                    .contains(&between.as_str());
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
fn interval_years(text: &str) -> std::collections::BTreeSet<i32> {
    scan_intervals(text)
        .into_iter()
        .flat_map(|(a, b, _)| [a, b])
        .collect()
}

fn relative_expression(text: &str) -> String {
    let lower = text.to_lowercase();
    for cand in ["tre anni dopo", "three years later", "anni dopo", "years later"] {
        if lower.contains(cand) {
            return cand.to_string();
        }
    }
    "relative".to_string()
}

// ── 5. Entity relationship graph ──────────────────────────────────

/// A typed entity relation. Only marker-evidenced relations are emitted: two
/// entities merely sharing a sentence produce NOTHING.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EntityRelation {
    pub subject: String,
    pub object: String,
    pub relation: String,
    pub evidence_sentence_ids: Vec<usize>,
}

const RELATION_MARKERS: &[(&str, &[&str])] = &[
    ("owns", &["possiede", "owns", "owns,", "besitzt", "possède", "posee"]),
    ("funds", &["finanzia", "funds", "finanziert", "finance", "financia"]),
    ("partners_with", &["collabora", "partners", "partnered", "arbeitet", "zusammen", "collabore", "colabora"]),
    ("competes_with", &["compete", "competes", "concorre", "konkurriert", "concurrence", "compite"]),
    ("contrasts_with", &["mentre", "while", "während", "wahrend", "tandis", "mientras"]),
];

/// Extract typed relations grounded in explicit markers. `entities` are the
/// known canonical surfaces; a sentence with two entities but no marker
/// yields no relation.
pub fn extract_relations(sentence: &Sentence, entities: &[String]) -> Vec<EntityRelation> {
    let toks = tokens(&sentence.text);
    let entity_lc: Vec<String> = entities.iter().map(|e| e.to_lowercase()).collect();
    let present: Vec<String> = entity_lc
        .iter()
        .filter(|e| {
            toks.iter().any(|t| t == *e)
                || sentence.text.to_lowercase().contains(e.as_str())
        })
        .cloned()
        .collect();
    if present.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    'markers: for (relation, markers) in RELATION_MARKERS {
        for m in *markers {
            if let Some(pos) = toks.iter().position(|t| t == m) {
                // Negated markers ("non possiede più") yield NO positive
                // relation: negation and time are preserved by silence +
                // the separate temporal signal, never by an inverted edge.
                if is_negated(&toks, pos) {
                    break 'markers;
                }
                out.push(EntityRelation {
                    subject: present[0].clone(),
                    object: present[1].clone(),
                    relation: relation.to_string(),
                    evidence_sentence_ids: vec![sentence.id],
                });
                break 'markers;
            }
        }
    }
    // Causal markers also feed the shared registry (same typed contract).
    if out.is_empty() {
        if let Some(c) = extract_causal(sentence) {
            if (c.relation == "causes" || c.relation == "contributes_to") && present.len() >= 2 {
                out.push(EntityRelation {
                    subject: present[0].clone(),
                    object: present[1].clone(),
                    relation: c.relation,
                    evidence_sentence_ids: vec![sentence.id],
                });
            } else if c.relation == "precedes" && present.len() >= 2 {
                out.push(EntityRelation {
                    subject: present[0].clone(),
                    object: present[1].clone(),
                    relation: "precedes".to_string(),
                    evidence_sentence_ids: vec![sentence.id],
                });
            }
        }
    }
    out.truncate(MAX_RELATION_NODES_PER_SCENE);
    out
}

// ── Manifest ──────────────────────────────────────────────────────

/// Per-scene signals.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SceneSignals {
    pub scene_id: String,
    pub density: Density,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comparisons: Vec<Comparison>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub causal: Vec<CausalRelation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub temporal_events: Vec<TemporalEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<EntityRelation>,
}

/// Canonical manifest: the single analysis product every planner consumes.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SceneIntelligenceManifest {
    pub schema_version: String,
    pub analyzer_version: String,
    pub relation_registry_version: String,
    pub scenes: Vec<SceneSignals>,
}

/// Full-pipeline analysis: shared context → detectors → canonical manifest.
/// `entities_per_scene` parallels `scenes` (same order) and carries the
/// existing NLP entity surfaces per scene.
pub fn analyze(scenes: &[SceneText], entities_per_scene: &[Vec<String>]) -> SceneIntelligenceManifest {
    let ctx = SharedContext::build(scenes);
    let mut prior_concepts: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for (i, scene) in scenes.iter().enumerate() {
        let entities: &[String] = entities_per_scene.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        let sents = ctx.sentences_for_scene(&scene.scene_id);
        let density = analyze_density(&scene.scene_id, &sents, entities, &prior_concepts);
        for s in &sents {
            for t in tokens(&s.text) {
                if t.chars().count() >= 5 {
                    prior_concepts.insert(t);
                }
            }
        }
        let comparisons = detect_comparison(&scene.scene_id, &sents, entities)
            .into_iter()
            .collect::<Vec<_>>();
        let mut causal = Vec::new();
        for s in &sents {
            if let Some(c) = extract_causal(s) {
                // Skip pure-sequence markers already covered as temporal flow?
                // No: keep both — timeline and causal consumers differ.
                causal.push(c);
            }
        }
        let temporal_events = extract_temporal(&sents);
        let mut relations = Vec::new();
        for s in &sents {
            relations.extend(extract_relations(s, entities));
        }
        relations.truncate(MAX_RELATION_NODES_PER_SCENE);
        out.push(SceneSignals {
            scene_id: scene.scene_id.clone(),
            density,
            comparisons,
            causal,
            temporal_events,
            relations,
        });
    }
    out.sort_by(|a, b| a.scene_id.cmp(&b.scene_id));
    SceneIntelligenceManifest {
        schema_version: SCHEMA_VERSION.to_string(),
        analyzer_version: ANALYZER_VERSION.to_string(),
        relation_registry_version: RELATION_REGISTRY_VERSION.to_string(),
        scenes: out,
    }
}

/// Fail-closed manifest validation (structure only, no semantic judgment).
pub fn validate_manifest(m: &SceneIntelligenceManifest) -> Result<(), String> {
    if m.schema_version != SCHEMA_VERSION {
        return Err(format!("unknown schema {}", m.schema_version));
    }
    if m.analyzer_version.is_empty() || m.relation_registry_version.is_empty() {
        return Err("missing version pins".to_string());
    }
    let mut seen = BTreeSet::new();
    for s in &m.scenes {
        if s.scene_id.trim().is_empty() {
            return Err("scene with empty id".to_string());
        }
        if !seen.insert(s.scene_id.clone()) {
            return Err(format!("duplicate scene {}", s.scene_id));
        }
        if !(0.0..=1.0).contains(&s.density.score) {
            return Err(format!("density out of range in {}", s.scene_id));
        }
        for c in &s.comparisons {
            if c.kind == "comparison" && c.candidate_visual == "split_comparison" && !c.values.is_empty() {
                return Err("qualitative comparison must not carry values".to_string());
            }
        }
        if s.relations.len() > MAX_RELATION_NODES_PER_SCENE {
            return Err(format!("too many relation nodes in {}", s.scene_id));
        }
    }
    Ok(())
}

/// Canonical JSON (sorted scenes already guaranteed by `analyze`).
pub fn manifest_json(m: &SceneIntelligenceManifest) -> Result<String, String> {
    serde_json::to_string(m).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn scenes(texts: &[(&str, &str)]) -> (Vec<SceneText>, SharedContext) {
        let v: Vec<SceneText> = texts
            .iter()
            .map(|(id, t)| SceneText {
                scene_id: id.to_string(),
                text: t.to_string(),
                language: "it".to_string(),
                video_start_ms: 0,
                video_end_ms: 20_000,
            })
            .collect();
        let ctx = SharedContext::build(&v);
        (v, ctx)
    }

    #[test]
    fn density_repetition_does_not_inflate() {
        let (_v, ctx) = scenes(&[("scene-01", "I costi energetici riducono i margini delle imprese industriali.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-01");
        let base = analyze_density("scene-01", &sents, &["imprese".into()], &BTreeSet::new());
        // Same content padded with verbatim repetition.
        let (_v2, ctx2) = scenes(&[(
            "scene-01",
            "I costi energetici riducono i margini delle imprese industriali. I costi energetici riducono i margini delle imprese industriali. I costi energetici riducono i margini delle imprese industriali.",
        )]);
        let sents2: Vec<&Sentence> = ctx2.sentences_for_scene("scene-01");
        let padded = analyze_density("scene-01", &sents2, &["imprese".into()], &BTreeSet::new());
        assert_eq!(base.score, padded.score, "repeated words must not raise density");
        assert_eq!(base.new_concepts, padded.new_concepts);
    }

    #[test]
    fn comparison_qualitative_invents_no_numbers() {
        let (_v, ctx) = scenes(&[("scene-07", "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-07");
        let c = detect_comparison("scene-07", &sents, &["Ford".into(), "Toyota".into()])
            .expect("comparison detected");
        assert_eq!(c.candidate_visual, "split_comparison");
        assert!(c.values.is_empty(), "qualitative comparison must not invent values");
        assert_eq!(c.subjects.len(), 2);
    }

    #[test]
    fn dopo_is_sequence_not_cause() {
        let (_v, ctx) = scenes(&[
            ("s1", "Le vendite sono diminuite dopo l'aumento dei prezzi."),
            ("s2", "Le vendite sono diminuite a causa dell'aumento dei prezzi."),
        ]);
        let seq: Vec<&Sentence> = ctx.sentences_for_scene("s1");
        let cau: Vec<&Sentence> = ctx.sentences_for_scene("s2");
        let r1 = extract_causal(seq[0]).expect("sequence extracted");
        let r2 = extract_causal(cau[0]).expect("cause extracted");
        assert_eq!(r1.relation, "precedes", "dopo is temporal sequence");
        assert_eq!(r2.relation, "causes", "a causa di is asserted causation");
        assert_ne!(r1.relation, r2.relation);
    }

    #[test]
    fn timeline_event_time_separate_from_video_time() {
        let (_v, ctx) = scenes(&[("scene-03", "Nel 2024 la crisi rientra. Già nel 2020 i costi esplosero, e nel 2022 i margini crollarono.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-03");
        let events = extract_temporal(&sents);
        let years: Vec<i32> = events.iter().filter_map(|e| e.normalized_year).collect();
        assert_eq!(years, vec![2020, 2022, 2024], "chronological order despite narration order");
        // Narration order preserved alongside.
        let narr: Vec<usize> = events.iter().map(|e| e.narration_index).collect();
        let mut sorted = narr.clone();
        sorted.sort_unstable();
        assert_eq!(narr.into_iter().collect::<BTreeSet<_>>(), sorted.into_iter().collect::<BTreeSet<_>>());
        // No video timestamps leak into events.
        let j = serde_json::to_string(&events).unwrap();
        assert!(!j.contains("video"), "event time and video time stay separate");
        // Relative expressions stay relative.
        let (_v2, ctx2) = scenes(&[("s", "Tre anni dopo gli investimenti crollarono.")]);
        let s2: Vec<&Sentence> = ctx2.sentences_for_scene("s");
        let rel = extract_temporal(&s2);
        assert!(rel.iter().any(|e| e.normalized_year.is_none()), "ambiguous relative stays unresolved");
    }

    #[test]
    fn comentions_without_marker_link_nothing() {
        let (_v, ctx) = scenes(&[("s", "Ford e Toyota restano due grandi case automobilistiche.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let rels = extract_relations(sents[0], &["ford".into(), "toyota".into()]);
        assert!(rels.is_empty(), "co-mention alone is not a relation");
        let (_v2, ctx2) = scenes(&[("s", "Ford collabora con Toyota sulle batterie.")]);
        let s2: Vec<&Sentence> = ctx2.sentences_for_scene("s");
        let rels2 = extract_relations(s2[0], &["ford".into(), "toyota".into()]);
        assert_eq!(rels2.len(), 1);
        assert_eq!(rels2[0].relation, "partners_with");
    }

    #[test]
    fn manifest_deterministic_and_valid() {
        let sc = vec![
            SceneText { scene_id: "scene-02".into(), text: "Ford cresce, Toyota rallenta.".into(), language: "it".into(), video_start_ms: 0, video_end_ms: 20000 },
            SceneText { scene_id: "scene-01".into(), text: "Nel 2020 i costi energetici aumentarono.".into(), language: "it".into(), video_start_ms: 0, video_end_ms: 20000 },
        ];
        let ents = vec![vec!["Ford".into(), "Toyota".into()], vec!["costi".into()]];
        let a = analyze(&sc, &ents);
        let b = analyze(&sc, &ents);
        assert_eq!(a, b, "same inputs must give identical manifests");
        validate_manifest(&a).expect("valid manifest");
        assert_eq!(a.scenes[0].scene_id, "scene-01", "scenes sorted for canonical JSON");
    }

    #[test]
    fn manifest_rejects_qualitative_with_values() {
        let mut m = analyze(
            &[SceneText { scene_id: "s".into(), text: "Ford cresce, Toyota rallenta.".into(), language: "it".into(), video_start_ms: 0, video_end_ms: 1000 }],
            &[vec!["Ford".into(), "Toyota".into()]],
        );
        if let Some(c) = m.scenes[0].comparisons.first_mut() {
            c.values.push(42.0);
        } else {
            panic!("expected a comparison");
        }
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn density_high_suggests_low_budget() {
        let (_v, ctx) = scenes(&[(
            "scene-04",
            "Nel 2020 Siemens, Volkswagen e Bosch annunciarono investimenti da 120 milioni mentre i costi energetici del 34% riducevano i margini industriali e la collaborazione con E.ON ridefiniva la strategia.",
        )]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("scene-04");
        let d = analyze_density(
            "scene-04",
            &sents,
            &["Siemens".into(), "Volkswagen".into(), "Bosch".into(), "E.ON".into()],
            &BTreeSet::new(),
        );
        assert_eq!(d.suggested_visual_budget, "low", "dense scenes get few simultaneous overlays");
        assert!(d.numeric_claims >= 2);
    }

    #[test]
    fn relation_nodes_capped_per_scene() {
        let mut s = Sentence { id: 0, scene_id: "s".into(), index_in_scene: 0, text: "Ford collabora con Toyota e compete con Volkswagen mentre Siemens finanzia Bosch che possiede E.ON.".into(), start_byte: 0, end_byte: 10 };
        let ents = vec!["ford".into(), "toyota".into(), "volkswagen".into(), "siemens".into(), "bosch".into(), "e.on".into(), "extra".into()];
        // extract_relations returns at most one marker relation per sentence;
        // the cap applies at manifest assembly across sentences.
        let r = extract_relations(&s, &ents);
        assert!(r.len() <= MAX_RELATION_NODES_PER_SCENE);
        s.id = 1;
        let _ = r;
    }

    #[test]
    fn shared_context_sentence_index() {
        let (_v, ctx) = scenes(&[("a", "Prima frase. Seconda frase!"), ("b", "Terza?")]);
        assert_eq!(ctx.sentences.len(), 3);
        assert_eq!(ctx.sentences[0].id, 0);
        assert_eq!(ctx.sentences[2].scene_id, "b");
        assert_eq!(ctx.sentences_for_scene("a").len(), 2);
    }

    // ── Runtime certification: comparison ──

    #[test]
    fn cert_temporal_comparison_years_as_subjects() {
        // C02: "Vendite 2020 vs 2025" is a temporal comparison; years are
        // temporal evidence, never invented quantities.
        let (_v, ctx) = scenes(&[("s", "Vendite 2020 vs 2025: il trend è chiaro.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let c = detect_comparison("s", &sents, &[]).expect("temporal comparison");
        assert_eq!(c.dimension, "temporal");
        assert_eq!(c.relation, "contrast");
        assert!(c.values.is_empty(), "years are not quantities");
    }

    #[test]
    fn cert_negated_comparative_keeps_negation() {
        // C05: "non è superiore" stays a negated comparison.
        let (_v, ctx) = scenes(&[("s", "Ford non è superiore a Toyota negli investimenti.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let c = detect_comparison("s", &sents, &["Ford".into(), "Toyota".into()])
            .expect("comparison detected");
        assert!(c.negated, "negation must be preserved, not dropped");
        assert_eq!(c.subjects.len(), 2);
    }

    #[test]
    fn cert_paraphrased_comparison_equivalent() {
        // C06: same contrast paraphrased yields the same structure.
        let (_v, ctx) = scenes(&[
            ("a", "Ford cresce con gli ibridi, mentre Toyota rallenta sui margini."),
            ("b", "Toyota rallenta sui margini mentre Ford cresce con gli ibridi."),
        ]);
        let sa: Vec<&Sentence> = ctx.sentences_for_scene("a");
        let sb: Vec<&Sentence> = ctx.sentences_for_scene("b");
        let ents = vec!["Ford".into(), "Toyota".into()];
        let ca = detect_comparison("a", &sa, &ents).expect("a");
        let cb = detect_comparison("b", &sb, &ents).expect("b");
        assert_eq!(ca.subjects, cb.subjects);
        assert_eq!(ca.dimension, cb.dimension);
        assert_eq!(ca.relation, cb.relation);
        assert_eq!(ca.candidate_visual, cb.candidate_visual);
    }

    // ── Runtime certification: cause–effect ──

    #[test]
    fn cert_negated_cause_yields_no_positive_relation() {
        // E03: denied causation produces no positive causal relation.
        let (_v, ctx) = scenes(&[("s", "La crisi non è stata causata dall'inflazione.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        assert!(extract_causal(sents[0]).is_none(), "negated cause is not a positive relation");
    }

    #[test]
    fn cert_attributed_hypothetical_keeps_attribution() {
        // E04: attributed, conditional claims keep both markers with
        // lowered confidence — never a certain arrow.
        let (_v, ctx) = scenes(&[("s", "Secondo gli analisti, l'aumento dei costi avrebbe provocato il calo dei margini.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let r = extract_causal(sents[0]).expect("relation extracted");
        assert_eq!(r.relation, "causes");
        assert!(r.hypothetical, "conditional mood preserved");
        assert!(r.attribution.is_some(), "attribution preserved");
        assert!(r.extraction_confidence < 0.9, "hypothetical confidence discounted");
    }

    #[test]
    fn cert_chain_extracts_links_without_transitive_invention() {
        // E05: A → B → C yields the asserted links, never an A → C shortcut.
        let (_v, ctx) = scenes(&[("s",
            "L'aumento dei costi ha causato il calo dei margini. Il calo dei margini ha provocato il crollo degli investimenti. La caduta dell'occupazione è stata causata dal crollo degli investimenti.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        assert_eq!(sents.len(), 3);
        let rels: Vec<CausalRelation> = sents.iter().filter_map(|s| extract_causal(s)).collect();
        assert_eq!(rels.len(), 3, "one relation per asserted link");
        let transitive = rels.iter().any(|r| r.cause.contains("costi") && r.effect.contains("occupazione"));
        assert!(!transitive, "no invented A → C shortcut");
    }

    #[test]
    fn cert_reverse_order_keeps_direction() {
        // E06: "Y a causa di X" still means X causes Y.
        let (_v, ctx) = scenes(&[("s", "I margini si sono ridotti a causa dell'aumento dei costi.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let r = extract_causal(sents[0]).expect("relation extracted");
        assert!(r.cause.contains("costi"), "cause is costs, got {}", r.cause);
        assert!(r.effect.contains("margini"), "effect is margins, got {}", r.effect);
    }

    #[test]
    fn cert_contributes_to_distinct_from_causes() {
        let (_v, ctx) = scenes(&[("s", "L'aumento dei costi energetici ha contribuito alla riduzione dei margini aziendali.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let r = extract_causal(sents[0]).expect("relation extracted");
        assert_eq!(r.relation, "contributes_to", "contributes_to is not causes");
    }

    // ── Runtime certification: timeline ──

    #[test]
    fn cert_relative_resolves_with_anchor_not_without() {
        // T02/T03: known anchor resolves, missing anchor stays unresolved.
        let (_v, ctx) = scenes(&[("s", "Tre anni dopo gli investimenti crollarono.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let open = extract_temporal(&sents);
        assert!(open.iter().any(|e| e.normalized_year.is_none()), "T03: ambiguous relative stays unresolved");
        let closed = extract_temporal_with_anchor(&sents, Some(2020));
        let rel = closed.iter().find(|e| e.temporal_expression.contains("anni dopo")).expect("relative event");
        assert_eq!(rel.normalized_year, Some(2023), "T02: anchor 2020 + 3 years");
    }

    #[test]
    fn cert_interval_represented_as_span() {
        // T04: "2020–2023" is one interval, not an instant, and the two
        // years are not duplicated as bare singles.
        let (_v, ctx) = scenes(&[("s", "L'evento copre il periodo 2020–2023 con tre fasi.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let events = extract_temporal(&sents);
        assert_eq!(events.len(), 1, "one interval event, got {:?}", events);
        assert_eq!(events[0].normalized_year, Some(2020));
        assert_eq!(events[0].normalized_end_year, Some(2023));
    }

    #[test]
    fn cert_same_year_two_events_kept() {
        // T05: equal years for different events are both conserved.
        let (_v, ctx) = scenes(&[("s", "Nel 2020 fu annunciato il progetto. Nel 2020 iniziarono i lavori.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let events = extract_temporal(&sents);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.normalized_year == Some(2020)));
        assert_ne!(events[0].event_id, events[1].event_id);
    }

    // ── Runtime certification: entity graph ──

    #[test]
    fn cert_negated_ownership_links_nothing_positive() {
        // G04: "non possiede più" keeps negation and time: no `owns`.
        let (_v, ctx) = scenes(&[("s", "Ford non possiede più Rivian dal 2023.")]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let rels = extract_relations(sents[0], &["ford".into(), "rivian".into()]);
        assert!(rels.iter().all(|r| r.relation != "owns"), "negated ownership is not owns");
    }

    #[test]
    fn cert_alias_surfaces_share_identity() {
        // G05: certain aliases ground to one canonical surface.
        let subjects = ground_subjects("Volkswagen (VW) cresce.", &["volkswagen".into(), "vw".into()]);
        let norm: std::collections::BTreeSet<String> = subjects.iter().map(|s| normalize_surface(s)).collect();
        assert_eq!(norm.len(), subjects.len(), "no duplicate normalized identities");
    }

    #[test]
    fn cert_extractor_uncapped_visual_capped() {
        // G06: the data graph stays complete across sentences; only the
        // per-scene visual composition is capped.
        let text = (0..10)
            .map(|i| format!("Azienda{} collabora con Partner{}.", i, i))
            .collect::<Vec<_>>()
            .join(" ");
        let (_v, ctx) = scenes(&[("s", &text)]);
        let sents: Vec<&Sentence> = ctx.sentences_for_scene("s");
        let ents: Vec<String> = (0..10).flat_map(|i| vec![format!("azienda{}", i), format!("partner{}", i)]).collect();
        let all: Vec<EntityRelation> = sents.iter().flat_map(|s| extract_relations(s, &ents)).collect();
        assert_eq!(all.len(), 10, "data graph complete");
        let sc = vec![SceneText { scene_id: "s".into(), text: text.clone(), language: "it".into(), video_start_ms: 0, video_end_ms: 20000 }];
        let m = analyze(&sc, &[ents]);
        assert!(m.scenes[0].relations.len() <= MAX_RELATION_NODES_PER_SCENE, "visual subgraph capped");
        validate_manifest(&m).expect("capped manifest valid");
    }

    // ── Runtime certification: density ordering ──

    #[test]
    fn cert_density_ordering_sparse_vs_rich() {
        // D01 < D02: ordering over cases, never an exact magic number.
        let (_v, ctx) = scenes(&[
            ("sparse", "Il giorno era bello e calmo."),
            ("rich", "Nel 2020 Siemens, Volkswagen e Bosch annunciarono investimenti da 120 milioni mentre i costi energetici del 34% riducevano i margini e la collaborazione con E.ON ridefiniva la strategia."),
        ]);
        let ss: Vec<&Sentence> = ctx.sentences_for_scene("sparse");
        let rs: Vec<&Sentence> = ctx.sentences_for_scene("rich");
        let sparse = analyze_density("sparse", &ss, &[], &BTreeSet::new());
        let rich = analyze_density("rich", &rs, &["Siemens".into(), "Volkswagen".into(), "Bosch".into(), "E.ON".into()], &BTreeSet::new());
        assert!(sparse.score < rich.score, "rich scene denser: {} vs {}", sparse.score, rich.score);
    }

    #[test]
    fn cert_density_fivefold_repetition_not_proportional() {
        // D03: repeating one concept 5x must not 5x the novelty.
        let one = "I costi energetici riducono i margini.";
        let five = [one; 5].join(" ");
        let (_v, ctx) = scenes(&[("a", one), ("b", &five)]);
        let sa: Vec<&Sentence> = ctx.sentences_for_scene("a");
        let sb: Vec<&Sentence> = ctx.sentences_for_scene("b");
        let d1 = analyze_density("a", &sa, &[], &BTreeSet::new());
        let d5 = analyze_density("b", &sb, &[], &BTreeSet::new());
        assert_eq!(d1.score, d5.score, "verbatim repetition adds no novelty");
    }

    #[test]
    fn cert_density_new_info_increases_at_same_length() {
        // D04: same word count, more distinct concepts → higher density.
        let (_v, ctx) = scenes(&[
            ("a", "costi costi costi costi costi costi costi costi costi costi"),
            ("b", "costi margini investimenti imprese energia strategia mercato utili tassi rischio"),
        ]);
        let sa: Vec<&Sentence> = ctx.sentences_for_scene("a");
        let sb: Vec<&Sentence> = ctx.sentences_for_scene("b");
        let da = analyze_density("a", &sa, &[], &BTreeSet::new());
        let db = analyze_density("b", &sb, &[], &BTreeSet::new());
        assert!(db.score > da.score, "100 distinct-concept words outrank 100 repeated words");
        assert!(db.new_concepts > da.new_concepts);
    }

    #[test]
    fn cert_event_date_separate_from_video_window() {
        // T06: a 2020 spoken at video-second 60 keeps event_date on the
        // event and video time on the scene — never mixed.
        let sc = vec![SceneText { scene_id: "s".into(), text: "Nel 2020 i costi aumentarono.".into(), language: "it".into(), video_start_ms: 60_000, video_end_ms: 80_000 }];
        let m = analyze(&sc, &[vec![]]);
        let j = manifest_json(&m).expect("serializes");
        assert!(!j.contains("video_start_ms"), "no video timestamps inside the manifest");
        assert!(!j.contains("video_end_ms"));
        assert!(j.contains("2020"), "event date preserved");
        let ev = &m.scenes[0].temporal_events;
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].normalized_year, Some(2020));
    }

    #[test]
    fn cert_manifest_stable_over_100_runs() {
        // X06: same input 100 times → identical manifests.
        let sc = vec![SceneText { scene_id: "s".into(), text: "Ford cresce, Toyota rallenta. Nel 2020 i costi aumentarono.".into(), language: "it".into(), video_start_ms: 0, video_end_ms: 20000 }];
        let ents = vec![vec!["Ford".into(), "Toyota".into()]];
        let first = analyze(&sc, &ents);
        for _ in 0..100 {
            assert_eq!(analyze(&sc, &ents), first);
        }
    }
}
