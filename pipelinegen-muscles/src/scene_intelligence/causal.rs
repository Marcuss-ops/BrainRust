//! Internal split of scene_intelligence (pure move, no behavior change).

use super::*;


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
pub(super) const CAUSE_AFTER_MARKERS: &[&[&str]] = &[
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
pub(super) const CAUSE_BEFORE_MARKERS: &[&[&str]] = &[
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
pub(super) const HYPOTHETICAL_MARKERS: &[&str] = &[
    "avrebbe",
    "avrebbero",
    "would",
    "could",
    "might",
    "potrebbe",
    "potrebbero",
    "würde",
    "würden",
    "pourrait",
];

/// Attribution prefixes (verbatim, before the first comma).
pub(super) const ATTRIBUTION_MARKERS: &[&str] = &["secondo", "according", "selon", "laut", "según", "segun"];

pub(super) const CONTRIBUTE_MARKERS: &[&[&str]] = &[
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

pub(super) const SEQUENCE_MARKERS: &[&str] = &[
    "dopo", "after", "nach", "après", "apres", "después", "despues",
];

/// Marker hit with its token position and cause direction.
pub(super) fn find_causal_marker(toks: &[String]) -> Option<(bool, usize, usize)> {
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
pub(super) fn split_at_marker(text: &str, toks: &[String], pos: usize, len: usize) -> (String, String) {
    let before: Vec<&String> = toks[..pos.min(toks.len())].iter().collect();
    let after: Vec<&String> = toks[(pos + len).min(toks.len())..].iter().collect();
    let _ = text;
    (
        slug(
            before
                .into_iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        slug(
            after
                .into_iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
    )
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
        let (cause, effect) = if cause_after {
            (after, before)
        } else {
            (before, after)
        };
        if cause.is_empty() || effect.is_empty() {
            return None;
        }
        let hypothetical = HYPOTHETICAL_MARKERS
            .iter()
            .any(|m| contains_token(&toks, m));
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
        let hypothetical = HYPOTHETICAL_MARKERS
            .iter()
            .any(|m| contains_token(&toks, m));
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
pub(super) fn extract_attribution(text: &str) -> Option<String> {
    let first: String = text.split(',').next().unwrap_or("").trim().to_string();
    if first.is_empty() || first.len() >= text.trim().len() {
        return None;
    }
    let lt = first.to_lowercase();
    if ATTRIBUTION_MARKERS
        .iter()
        .any(|m| lt.split_whitespace().any(|w| w == *m))
    {
        return Some(first);
    }
    None
}

pub(super) fn split_around(text: &str) -> (String, String) {
    let lower = text.to_lowercase();
    for sep in [
        ", provocando",
        ", contribuendo",
        " ha ridotto",
        " hanno ridotto",
        " dopo ",
        " after ",
    ] {
        if let Some(pos) = lower.find(sep) {
            let a = text[..pos]
                .trim()
                .trim_matches(|c| c == '.' || c == ',' || c == ';')
                .to_string();
            let b = text[pos + sep.len()..]
                .trim()
                .trim_matches(|c| c == '.' || c == ',' || c == ';')
                .to_string();
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

pub(super) fn slug(s: String) -> String {
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

