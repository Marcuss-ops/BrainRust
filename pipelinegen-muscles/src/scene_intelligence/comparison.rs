//! Internal split of scene_intelligence (pure move, no behavior change).

use super::*;

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

pub(super) const COMPARISON_SIGNALS: &[&[&str]] = &[
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
pub(super) const COMPARATIVE_ADJ: &[&str] = &[
    "superiore",
    "inferiore",
    "maggiore",
    "minore",
    "migliore",
    "peggiore",
    "superior",
    "inferior",
    "greater",
    "lesser",
    "higher",
    "lower",
    "better",
    "worse",
    "höher",
    "niedriger",
    "meilleur",
    "pire",
    "superieur",
    "inférieur",
];

/// Negation tokens (sentence-scope heuristic: a negated marker yields NO
/// positive signal — fail-soft toward silence, never toward an inverted
/// claim).
pub(super) const NEGATION_TOKENS: &[&str] = &[
    "non", "no", "not", "nope", "mai", "nunca", "nicht", "kein", "keine", "ne", "pas", "jamais",
    "não", "nao", "niet",
];

/// True when a negation token scopes over a marker at `marker_pos` (within
/// the preceding 4 tokens or anywhere before a sentence-initial marker).
pub(super) fn is_negated(toks: &[String], marker_pos: usize) -> bool {
    let from = marker_pos.saturating_sub(4);
    toks[from..marker_pos.min(toks.len())]
        .iter()
        .any(|t| NEGATION_TOKENS.contains(&t.as_str()))
}

pub(super) const CONTRAST_VERBS: &[&str] = &[
    "cresce",
    "rallenta",
    "grows",
    "slows",
    "growing",
    "slowing",
    "steigt",
    "sinkt",
    "augmente",
    "ralentit",
    "aumenta",
    "disminuye",
    "crescono",
    "rallentano",
    "rose",
    "fell",
    "sale",
    "scende",
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
        let explicit = COMPARISON_SIGNALS.iter().any(|p| contains_phrase(&toks, p));
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

pub(super) fn capitalized_terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')) {
        if tok.chars().count() >= 2
            && tok
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
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
pub(super) fn normalize_surface(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub(super) fn ground_subjects(sentence: &str, entities: &[String]) -> Vec<String> {
    let caps = capitalized_terms(sentence);
    let mut out = Vec::new();
    let mut seen_norm = std::collections::BTreeSet::new();
    let push =
        |v: String, out: &mut Vec<String>, seen_norm: &mut std::collections::BTreeSet<String>| {
            let n = normalize_surface(&v);
            if n.is_empty() || !seen_norm.insert(n) {
                return;
            }
            out.push(v);
        };
    for e in entities {
        if caps
            .iter()
            .any(|c| c == e || c.contains(e.as_str()) || e.contains(c.as_str()))
        {
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

pub(super) fn guess_dimension(toks: &[String]) -> String {
    let dims = [
        (
            "investment",
            &[
                "investment",
                "investimenti",
                "investimento",
                "investition",
                "investissement",
            ][..],
        ),
        (
            "cost",
            &["cost", "costi", "costo", "kosten", "coût", "cout"][..],
        ),
        (
            "growth",
            &[
                "growth",
                "crescita",
                "wachstum",
                "croissance",
                "crecimiento",
            ][..],
        ),
        (
            "strategy",
            &["strategy", "strategia", "strategie", "stratégie"][..],
        ),
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
    for tok in
        text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c == '%'))
    {
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

