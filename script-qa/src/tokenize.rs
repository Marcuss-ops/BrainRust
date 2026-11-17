use std::collections::HashSet;

use semantic_core::language_profiles;

/// Lowercased word tokens. The separator is exactly the Go gate's rule:
/// anything that is not a Unicode letter or number.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for word in text.split(|ch: char| !(ch.is_alphabetic() || ch.is_numeric())) {
        let word = word.trim().to_lowercase();
        if !word.is_empty() {
            tokens.push(word);
        }
    }
    tokens
}

/// The gate's fallback stop-word set. Go's `DefaultStopWords` is the
/// `fallback` profile; Rust owns that same generated profile.
pub fn default_stop_words() -> HashSet<&'static str> {
    language_profiles::fallback()
        .map(|profile| profile.stopwords.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

pub fn filter_stop_words(tokens: &[String]) -> Vec<String> {
    let stop_words = default_stop_words();
    tokens
        .iter()
        .filter(|token| !stop_words.contains(token.as_str()))
        .cloned()
        .collect()
}

/// True when the generated text is a placeholder or generic fallback.
pub fn is_generic_text(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "lorem ipsum",
        "sample text",
        "placeholder",
        "todo:",
        "tbd",
        "insert text here",
        "your text here",
        "generated text",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

/// Ratio of generated content tokens that appear in the source. Stop words
/// are removed from both sides before the comparison.
pub fn compute_source_text_coverage(generated: &str, source: &str) -> f64 {
    let generated_tokens = filter_stop_words(&tokenize(generated));
    if generated_tokens.is_empty() {
        return 0.0;
    }

    let source_tokens: HashSet<String> = filter_stop_words(&tokenize(source)).into_iter().collect();
    if source_tokens.is_empty() {
        return 0.0;
    }

    let matches = generated_tokens
        .iter()
        .filter(|token| source_tokens.contains(token.as_str()))
        .count();
    matches as f64 / generated_tokens.len() as f64
}

/// Stop-word overlap language detector. Languages are visited in the same
/// stable sorted order as the Go registry; the first highest score wins.
pub fn detect_language(text: &str) -> String {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return String::new();
    }

    let mut best_score = 0.0;
    let mut best_code = String::new();
    for code in language_profiles::supported_languages() {
        if code == "fallback" {
            continue;
        }
        let Some(profile) = language_profiles::get(code) else {
            continue;
        };
        let score = language_match_score(&tokens, &profile.stopwords);
        if score > best_score {
            best_score = score;
            best_code = code.to_string();
        }
    }
    best_code
}

fn language_match_score(tokens: &[String], stop_words: &[String]) -> f64 {
    if tokens.is_empty() || stop_words.is_empty() {
        return 0.0;
    }
    let stop_words: HashSet<&str> = stop_words.iter().map(String::as_str).collect();
    let seen = tokens
        .iter()
        .filter(|token| stop_words.contains(token.as_str()))
        .count();
    seen as f64 / tokens.len() as f64
}
