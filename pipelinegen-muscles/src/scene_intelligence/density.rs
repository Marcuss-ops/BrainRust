//! Internal split of scene_intelligence (pure move, no behavior change).

use super::*;

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
    let body: String = distinct
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
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

pub(super) fn count_numbers(text: &str) -> usize {
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

pub(super) fn count_relation_marks(toks: &[String]) -> usize {
    let markers = [
        "causa", "perché", "perche", "because", "caused", "dopo", "after", "rispetto", "mentre",
        "versus", "contro", "wenn", "während",
    ];
    toks.iter()
        .filter(|t| markers.contains(&t.as_str()))
        .count()
}

// ── 2. Comparison detector ────────────────────────────────────────

