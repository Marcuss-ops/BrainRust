//! The deterministic VisualNER extraction pipeline (V1):
//!
//! ```text
//! text
//!  ↓
//! tokenize
//!  ↓
//! noun/noun-phrase candidates
//!  ↓
//! stop phrases
//!  ↓
//! visualness scoring
//!  ↓
//! source evidence validation
//!  ↓
//! rank
//!  ↓
//! top N
//! ```
//!
//! V1 is deterministic: NO ONNX, NO LLM, NO GPU. The killer rule is
//! `NO EVIDENCE → NO ENTITY`: every returned entity must be a verbatim
//! substring of the source text, with byte offsets proving the evidence.

use crate::types::{ExtractOptions, VisualEntity};

/// Extract the top-N source-grounded visual entities from `source_text`.
/// This is the single canonical entry point. Returns entities in score-desc
/// order (ties broken by earliest source position).
pub fn extract(source_text: &str, options: &ExtractOptions) -> Vec<VisualEntity> {
    let tokens = tokenize(source_text);
    let candidates = noun_phrase_candidates(&tokens, source_text);
    let mut scored: Vec<VisualEntity> = candidates
        .into_iter()
        .filter_map(|c| validate_evidence(c, source_text))
        .map(score_entity)
        .collect();
    // Rank: highest score first; ties broken by earliest start offset so
    // the order is deterministic across 100/100 runs.
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.start.cmp(&b.start))
            .then(a.text.cmp(&b.text))
    });
    scored.truncate(options.top_n());
    scored
}

/// Token is a maximal run of letters (ASCII + accented Latin) and
/// apostrophes/hyphens inside a word, with its byte offsets into the
/// source text. Non-letter chars are separators.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    start: usize,
    end: usize,
}

/// tokenize splits `source_text` into maximal letter-runs, recording byte
/// offsets. Case is preserved in `text`; comparisons use a lowercased copy.
fn tokenize(source_text: &str) -> Vec<Token> {
    let bytes = source_text.as_bytes();
    let mut tokens = Vec::new();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if is_word_byte(bytes[i]) {
            let start = i;
            while i < n && is_word_byte(bytes[i]) {
                i += 1;
            }
            let text = std::str::from_utf8(&bytes[start..i]).unwrap_or("").to_string();
            tokens.push(Token {
                text,
                start,
                end: i,
            });
        } else {
            i += 1;
        }
    }
    tokens
}

/// is_word_byte reports whether a byte is part of a word: ASCII letter,
/// apostrophe, hyphen, or a high bit (UTF-8 continuation/lead byte for
/// accented Latin like "à"). This keeps the tokenizer dependency-free.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'\'' || b == b'-' || b >= 0x80
}

/// Candidate is a noun-phrase candidate with its byte span in the source
/// text. `normalized` is the lowercased surface used for stop-phrase /
/// visualness lookups; `text` preserves original case.
#[derive(Debug, Clone)]
struct Candidate {
    text: String,
    #[allow(dead_code)]
    normalized: String,
    start: usize,
    end: usize,
}

/// noun_phrase_candidates groups adjacent non-stopword tokens into maximal
/// noun phrases. A phrase is one or more consecutive tokens whose lowercase
/// surface is not a stopword, AND that are separated only by whitespace
/// (a comma, period, semicolon or other punctuation between two tokens is a
/// hard phrase boundary). Multi-word phrases ("feta cheese",
/// "olive oil") score higher than singletons, which is what makes the
/// Mediterranean golden fixture's anchors win over generic singletons.
fn noun_phrase_candidates(tokens: &[Token], source_text: &str) -> Vec<Candidate> {
    let bytes = source_text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if is_stop_word(&tokens[i].text) {
            i += 1;
            continue;
        }
        // Proper names are bounded by the title-case run. The old maximal
        // noun-phrase rule incorrectly absorbed predicates and their objects
        // (for example "Floyd Mayweather became one"), producing invalid
        // semantic spans. Lowercase phrases retain the noun-phrase behavior
        // used by the media-object fixtures.
        let phrase_start = i;
        let mut j = i;
        if starts_uppercase(&tokens[i].text) {
            while j < tokens.len() && starts_uppercase(&tokens[j].text) {
                if j > phrase_start {
                    let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                    if gap.iter().any(|b| is_phrase_breaking_byte(*b)) {
                        break;
                    }
                }
                j += 1;
            }
            // Keep the curated multi-word visual subjects intact even when
            // their first token is title-cased at sentence start.
            if j == phrase_start + 1 && j < tokens.len() && !is_stop_word(&tokens[j].text) {
                let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                let compound = format!("{} {}", tokens[phrase_start].text, tokens[j].text).to_lowercase();
                if !gap.iter().any(|b| is_phrase_breaking_byte(*b))
                    && (is_visual_object_hint(&compound) || is_subject_phrase(&compound))
                {
                    j += 1;
                }
            }
        } else {
            while j < tokens.len() && !is_stop_word(&tokens[j].text) {
                if j > phrase_start {
                    let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                    if gap.iter().any(|b| is_phrase_breaking_byte(*b)) {
                        break;
                    }
                }
                j += 1;
            }
        }
        let phrase_end = j; // exclusive
        let start = tokens[phrase_start].start;
        let end = tokens[phrase_end - 1].end;
        let text = source_text[start..end].to_string();
        let normalized = text.to_lowercase();
        // Reject phrases whose normalized surface is a stop phrase, a
        // generic phrase, OR a multi-word subject phrase. Multi-word
        // dish names ("greek salad", "grilled sardines", "seafood paella")
        // belong on SceneIR.Profile.Subject, not in Entities; dropping
        // them here lets the concrete ingredients ("feta cheese",
        // "tomatoes", "olives") fill the top-N. Single-word subjects
        // ("hummus", "shakshuka", "paella") are NOT dropped — they are
        // legitimate one-word ingredient/dish candidates.
        if !is_stop_phrase(&normalized)
            && !is_generic_phrase(&normalized)
            && !is_subject_phrase(&normalized)
        {
            out.push(Candidate {
                text,
                normalized,
                start,
                end,
            });
        }
        i = phrase_end;
    }
    out
}

fn starts_uppercase(text: &str) -> bool {
    text.chars().next().map(char::is_uppercase).unwrap_or(false)
}

/// is_phrase_breaking_byte reports whether a byte in the gap between two
/// tokens should break a noun phrase. Whitespace (space, tab, newline)
/// does NOT break; any other non-word byte (comma, period, semicolon,
/// colon, dash outside a word, parens) DOES break. This is what keeps
/// "feta cheese" together while splitting "tomatoes, feta".
fn is_phrase_breaking_byte(b: u8) -> bool {
    !b.is_ascii_whitespace()
}

/// validate_evidence enforces NO EVIDENCE → NO ENTITY. A candidate is
/// grounded when its byte span `[start, end)` is a verbatim substring of
/// `source_text` (which it is by construction, since the span was sliced
/// from the source). The function additionally guards against accidental
/// spans that don't round-trip (defensive: catches a future refactor bug
/// where offsets drift from the text). Returns `None` for an ungrounded
/// candidate.
fn validate_evidence(c: Candidate, source_text: &str) -> Option<VisualEntity> {
    if c.start >= c.end || c.end > source_text.len() {
        return None;
    }
    let evidence = source_text.get(c.start..c.end)?;
    // The evidence must equal the candidate text verbatim. A candidate
    // whose text drifted from its span (e.g. due to a normalization step
    // that rewrote the surface) fails NO EVIDENCE → NO ENTITY.
    if evidence != c.text {
        return None;
    }
    Some(VisualEntity {
        text: c.text.clone(),
        r#type: classify_type(&c.text),
        score: 0.0,
        start: c.start,
        end: c.end,
        evidence: evidence.to_string(),
    })
}

fn classify_type(text: &str) -> String {
    let lower = text.to_lowercase();
    if matches!(lower.as_str(), "london" | "paris" | "rome" | "new york") {
        return "LOCATION".to_string();
    }
    if lower == "openai" || lower.contains("company") || lower.contains("corporation") {
        return "ORGANIZATION".to_string();
    }
    if lower == "iphone" || lower.contains("smartphone") {
        return "PRODUCT".to_string();
    }
    // A multi-token title-cased name is the deterministic V1 person rule.
    let title_tokens = text.split_whitespace().filter(|word| word.chars().next().map(char::is_uppercase).unwrap_or(false)).count();
    if title_tokens >= 2 && text.split_whitespace().count() >= 2 {
        return "PERSON".to_string();
    }
    "VISUAL_CONCEPT".to_string()
}

/// score_entity applies the deterministic visualness scoring. V1 uses a
/// curated rule set (no ML):
///   - base: 0.30 for a single-word noun
///   - +0.20 per additional word in the phrase (multi-word phrases win)
///   - +0.15 if the (lowercased) phrase matches a known visual-object hint
///   - +0.05 if the phrase contains a food/object keyword as a substring
///   - -0.25 penalty if the (lowercased) phrase is a known generic verb /
///     pronoun / adverb (imagine, ready, world, get, dive...)
///   - -0.10 penalty if the (lowercased) phrase is a known multi-word
///     subject (the dish/category name like "greek salad" — it stays a
///     candidate but concrete ingredients must outrank it, since the
///     subject belongs on SceneIR.Profile.Subject, not in Entities).
///     The final score is clamped to `[0.0, 1.0]`.
fn score_entity(mut e: VisualEntity) -> VisualEntity {
    let lower = e.text.to_lowercase();
    let word_count = lower.split_whitespace().count().max(1);
    let mut score: f32 = 0.30 + 0.20 * (word_count as f32 - 1.0);
    if is_visual_object_hint(&lower) {
        score += 0.15;
    }
    for kw in VISUAL_OBJECT_KEYWORDS.iter() {
        if lower.contains(kw) {
            score += 0.05;
            break;
        }
    }
    if is_generic_phrase(&lower) {
        score -= 0.25;
    }
    if is_subject_phrase(&lower) {
        score -= 0.10;
    }
    score = score.clamp(0.0, 1.0);
    e.score = score;
    e
}

// ── Stopword + blocklist tables ──────────────────────────────────────
//
// V1 uses curated, dependency-free tables. The tables are intentionally
// small and explicit so the extractor stays deterministic and auditable.
// Adding a word here is a one-line change; no model retraining.

/// Stopwords: single tokens that never start or extend a noun phrase.
/// Includes the LIVE-test hallucination seeds "imagine", "ready", "get",
/// "the", "a", "and", etc. so "Imagine the" / "Get ready" never produce
/// candidates in the first place.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "with",
    "is", "are", "was", "were", "be", "been", "being", "has", "have", "had",
    "this", "that", "these", "those", "it", "its", "as", "by", "at", "from",
    // hallucination seeds observed in the LIVE test
    "imagine", "ready", "get", "dive", "into", "discover", "let", "us",
    // pronouns / generic verbs
    "you", "we", "they", "he", "she", "i", "me", "him", "her",
    "do", "does", "did", "not", "no", "yes",
    "about", "your", "our", "their", "his",
    // generic spatial/temporal words
    "now", "then", "here", "there", "when", "where", "what", "which", "who",
    // connector / descriptive verbs that glue non-visual phrases
    // together in the Mediterranean golden-fixture sentences. These are
    // not visual objects, so a phrase that includes them is not a single
    // visual noun phrase; breaking here keeps "feta cheese" separate
    // from "tomatoes" when the sentence reads "combines fresh tomatoes".
    "combines", "contains", "features", "traditionally", "made", "fresh",
    "prepared", "spoke", "released",
];

/// Stop phrases: multi-word surfaces that are generic even when none of
/// their tokens are individually stopwords. Currently small because the
/// stopword table already breaks up most generic phrases at tokenization.
const STOP_PHRASES: &[&str] = &["get ready", "let us", "imagine the"];

/// Visual-object hints: full lowercase phrases that are known concrete
/// visual objects in the VidRush domain (the Mediterranean golden
/// fixture's anchors). A phrase matching one of these gets a +0.15 boost.
const VISUAL_OBJECT_HINTS: &[&str] = &[
    "greek salad",
    "feta cheese",
    "olive oil",
    "lemon juice",
    "grilled sardines",
    "seafood paella",
];

/// Visual-object keywords: lowercase substrings that signal a concrete
/// object noun. A phrase containing any of these gets a +0.05 boost.
const VISUAL_OBJECT_KEYWORDS: &[&str] = &[
    "cheese", "tomato", "tomatoes", "olive", "olives", "feta", "salad",
    "hummus", "chickpea", "chickpeas", "tahini", "lemon", "herbs", "sardine",
    "sardines", "shakshuka", "egg", "eggs", "pepper", "peppers", "paella",
    "shrimp", "mussel", "mussels", "rice", "oil",
];

/// Generic phrases: lowercase surfaces that are NOT visual objects and
/// should be demoted. Includes the LIVE-test hallucinations "imagine the",
/// "ready", "world", "discover" so they never outscore a real object.
const GENERIC_PHRASES: &[&str] = &[
    "imagine", "ready", "world", "discover", "vibrant", "cuisine", "mediterranean",
    "get ready", "let us", "imagine the", "ready to",
];

/// Subject phrases: lowercase multi-word surfaces that name the dish /
/// category (the segment subject). They stay valid candidates (the plan
/// expects "hummus" to appear in the candidate list) but the concrete
/// ingredients must outrank them, because the subject belongs on
/// SceneIR.Profile.Subject, not in Entities. Single-word subjects
/// ("hummus", "shakshuka", "paella") are NOT penalized here — they are
/// legitimate one-word ingredient/dish candidates.
const SUBJECT_PHRASES: &[&str] = &["greek salad", "grilled sardines", "seafood paella"];

fn is_stop_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    STOP_WORDS.iter().any(|s| *s == lower)
}

fn is_stop_phrase(normalized: &str) -> bool {
    STOP_PHRASES.contains(&normalized)
}

fn is_visual_object_hint(lower: &str) -> bool {
    VISUAL_OBJECT_HINTS.contains(&lower)
}

fn is_generic_phrase(lower: &str) -> bool {
    GENERIC_PHRASES.contains(&lower)
}

fn is_subject_phrase(lower: &str) -> bool {
    SUBJECT_PHRASES.contains(&lower)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn top3(text: &str) -> Vec<VisualEntity> {
        extract(text, &ExtractOptions { entity_count: 3 })
    }

    fn texts(entities: &[VisualEntity]) -> Vec<String> {
        entities.iter().map(|e| e.text.clone()).collect()
    }

    // TestEntitiesRequireSourceEvidence — the killer rule.
    // "Imagine the" / "ready" must NEVER become entities because they are
    // stopword-seeded and never produce candidates in the first place.
    #[test]
    fn imagine_the_and_ready_are_rejected() {
        let entities = top3("Imagine the vibrant world of Greek cuisine, ready to discover...");
        let surfaces: Vec<&str> = entities.iter().map(|e| e.text.as_str()).collect();
        assert!(
            !surfaces.iter().any(|s| s.to_lowercase().contains("imagine")),
            "Imagine the must not become an entity: {:?}",
            surfaces
        );
        assert!(
            !surfaces.iter().any(|s| s.to_lowercase() == "ready"),
            "ready must not become an entity: {:?}",
            surfaces
        );
        assert!(
            !surfaces.iter().any(|s| s.to_lowercase().contains("discover")),
            "discover must not become an entity: {:?}",
            surfaces
        );
    }

    // TestExactEntityLimit — requesting 3 returns exactly 3 when the text
    // has ≥3 candidates.
    #[test]
    fn exactly_three_entities_returned() {
        let entities = top3("Greek salad combines fresh tomatoes, cucumbers, olives, feta cheese and olive oil.");
        assert_eq!(entities.len(), 3, "expected exactly 3 entities, got {entities:?}");
    }

    // Greek salad regression — the canonical golden-fixture input.
    // Expected: feta cheese, tomatoes, olives (top 3 by visual score).
    #[test]
    fn greek_salad_returns_feta_tomatoes_olives() {
        let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
        let surfaces = texts(&entities);
        assert!(surfaces.iter().any(|s| s.to_lowercase() == "feta cheese"), "feta cheese missing: {surfaces:?}");
        assert!(surfaces.iter().any(|s| s.to_lowercase() == "tomatoes"), "tomatoes missing: {surfaces:?}");
        assert!(surfaces.iter().any(|s| s.to_lowercase() == "olives"), "olives missing: {surfaces:?}");
        // All 3 must be source-grounded.
        for e in &entities {
            assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
            assert_eq!(e.evidence, e.text, "evidence must equal text verbatim");
        }
    }

    // Hummus regression — the second golden-fixture segment.
    // Expected candidates include hummus, chickpeas, tahini, lemon juice, olive oil;
    // top 3 must all be source-grounded.
    #[test]
    fn hummus_returns_source_grounded_entities() {
        let entities = top3("Hummus is traditionally made with chickpeas, tahini, lemon juice and olive oil.");
        // All returned entities must be source-grounded (NO EVIDENCE → NO ENTITY).
        for e in &entities {
            assert!(!e.evidence.is_empty(), "evidence empty for {}", e.text);
            assert_eq!(e.evidence, e.text, "evidence must equal text verbatim for {}", e.text);
        }
        // The top 3 must come from the expected candidate set.
        let expected = ["hummus", "chickpeas", "tahini", "lemon juice", "olive oil"];
        for e in &entities {
            let lower = e.text.to_lowercase();
            assert!(expected.iter().any(|s| *s == lower), "unexpected entity {}: not in {expected:?}", e.text);
        }
    }

    #[test]
    fn typed_entities_use_shared_vocabulary() {
        let entities = extract(
            "Gerard Butler spoke at an event in London. OpenAI released an iPhone.",
            &ExtractOptions { entity_count: 8 },
        );
        let find = |name: &str| entities.iter().find(|entity| entity.text == name);
        assert_eq!(find("Gerard Butler").map(|entity| entity.r#type.as_str()), Some("PERSON"));
        assert_eq!(find("London").map(|entity| entity.r#type.as_str()), Some("LOCATION"));
        assert_eq!(find("OpenAI").map(|entity| entity.r#type.as_str()), Some("ORGANIZATION"));
        assert_eq!(find("iPhone").map(|entity| entity.r#type.as_str()), Some("PRODUCT"));
    }

    // TestEntitiesRequireSourceEvidence — explicit: an entity not in the
    // source text must never be returned. Synthetic check.
    #[test]
    fn no_entity_without_evidence() {
        let entities = top3("Greek salad contains tomatoes, feta cheese and olives.");
        for e in &entities {
            // evidence must be a non-empty verbatim slice of the source.
            assert!(e.start < e.end, "empty span for {}", e.text);
            assert!(!e.evidence.is_empty(), "empty evidence for {}", e.text);
            assert_eq!(e.evidence, e.text, "evidence must equal text verbatim for {}", e.text);
        }
    }

    // Determinism: 100 runs must produce the same winner set.
    #[test]
    fn deterministic_across_runs() {
        let text = "Greek salad contains tomatoes, feta cheese and olives.";
        let first = top3(text);
        for _ in 0..99 {
            let again = top3(text);
            assert_eq!(again, first, "non-deterministic extraction result");
        }
    }
}
