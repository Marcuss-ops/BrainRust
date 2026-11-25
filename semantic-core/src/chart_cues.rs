//! Data-driven chart cue lexicon.
//!
//! Every chart word list (denial vetoes, approximation cues, rate periods,
//! curated units, subject-label stopwords and repeated-metric stems) lives in
//! `lexicon/chart_cues.json`, not in Rust. The table is embedded so a data edit
//! changes the content fingerprint and invalidates cached manifests.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

pub const CHART_CUES_SCHEMA: &str = "chart_cues.v1";

#[derive(Debug, Deserialize)]
pub struct ChartCueFile {
    pub schema: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub global: GlobalChartCues,
    #[serde(default)]
    pub by_language: BTreeMap<String, LanguageChartCues>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GlobalChartCues {
    /// Word sequences that veto a chart (denials, debunking, unverified).
    #[serde(default)]
    pub hard_vetoes: Vec<Vec<String>>,
    #[serde(default)]
    pub approximators: Vec<String>,
    #[serde(default)]
    pub approximator_pairs: Vec<Vec<String>>,
    #[serde(default)]
    pub rate_cues: Vec<String>,
    /// Rank surfaces that must never turn a rank index into a chart value.
    #[serde(default)]
    pub rank_veto_cues: Vec<String>,
    #[serde(default)]
    pub subject_stopwords: Vec<String>,
    #[serde(default)]
    pub subject_verb_cutoffs: Vec<String>,
    #[serde(default)]
    pub currency_cues: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub length_cues: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub count_unit_cues: Vec<String>,
    /// Unit surfaces a displayed numeric label may absorb verbatim.
    #[serde(default)]
    pub display_unit_cues: Vec<String>,
    #[serde(default)]
    pub monthly_pairs: Vec<Vec<String>>,
    #[serde(default)]
    pub yearly_cues: Vec<String>,
    #[serde(default)]
    pub range_openers: Vec<String>,
    #[serde(default)]
    pub range_connectors: Vec<String>,
    /// Temporal words before a four-digit year ("nel", "in", "im Jahr").
    #[serde(default)]
    pub year_context_cues: Vec<String>,
    /// Articles that mark a modern bare year ("il 2024", "the 2024").
    #[serde(default)]
    pub year_article_cues: Vec<String>,
    /// Range openers within which a second four-digit number is still a year
    /// ("tra il 1998 e il 2004", "dal 2015 al 2018").
    #[serde(default)]
    pub year_range_openers: Vec<String>,
    /// Words after a four-digit year ("anno", "roku", "году").
    #[serde(default)]
    pub year_suffix_cues: Vec<String>,
    /// Ordered-series cues: before/after, ordinals, months and trend verbs.
    #[serde(default)]
    pub ordered_sequence_cues: Vec<String>,
    /// Label-safe subset of sequence cues: ordinals, before/after and
    /// sequence markers, never prepositions such as German `von`.
    #[serde(default)]
    pub series_label_cues: Vec<String>,
    /// Forecast/hypothesis cues. A chart may be wholly conditional, but it
    /// must never mix an asserted result with a forecast value.
    #[serde(default)]
    pub forecast_cues: Vec<String>,
    /// Multi-part series evidence: each entry requires at least one cue from
    /// every inner group, and optionally repeated units above `min_repeats`.
    #[serde(default)]
    pub paired_series_rules: Vec<PairedSeriesRule>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PairedSeriesRule {
    #[serde(default)]
    pub any_in_each: Vec<Vec<String>>,
    #[serde(default)]
    pub repeated_units: Vec<String>,
    #[serde(default)]
    pub min_repeats: usize,
}

#[derive(Debug, Default, Deserialize)]
pub struct LanguageChartCues {
    /// Prepositions that may introduce a value but must never become a bare
    /// chart label, even when sentence-initial capitalization makes them look
    /// like a proper noun (German "Von 1500", Dutch "Van 1500").
    #[serde(default)]
    pub label_prepositions: Vec<String>,
    #[serde(default)]
    pub series_context_cues: Vec<String>,
    /// Survey/sondage cues keep the existing percent-in-same-clause guard.
    #[serde(default)]
    pub survey_cues: Vec<String>,
    #[serde(default)]
    pub series_metric_stems: Vec<String>,
    #[serde(default)]
    pub semicolon_metric_stems: Vec<String>,
    #[serde(default)]
    pub share_context_cues: Vec<String>,
    #[serde(default)]
    pub growth_cues: Vec<String>,
    /// Hypothetical/forecast cues that split one comparison into scenarios.
    #[serde(default)]
    pub scenario_cues: Vec<String>,
}

fn parse() -> ChartCueFile {
    let file: ChartCueFile = serde_json::from_str(include_str!("../../lexicon/chart_cues.json"))
        .expect("embedded chart cue lexicon must be valid JSON");
    assert_eq!(
        file.schema, CHART_CUES_SCHEMA,
        "unsupported chart cue lexicon schema"
    );
    assert_eq!(
        file.languages.len(),
        11,
        "chart cue lexicon must cover all 11 supported languages"
    );
    file
}

pub fn chart_cues() -> &'static ChartCueFile {
    static FILE: OnceLock<ChartCueFile> = OnceLock::new();
    FILE.get_or_init(parse)
}

pub fn global() -> &'static GlobalChartCues {
    &chart_cues().global
}

/// True when an ordered-series cue (`prima`, `poi`, an ordinal, a month, a
/// trend verb) occurs within `radius` bytes of the span. The window is
/// boundary-safe and cue matches are word-scoped.
pub fn ordered_series_cue_near(text: &str, start: usize, end: usize, radius: usize) -> bool {
    let mut window_start = start.saturating_sub(radius);
    while window_start < start && !text.is_char_boundary(window_start) {
        window_start += 1;
    }
    let mut window_end = end.saturating_add(radius).min(text.len());
    while window_end > end && !text.is_char_boundary(window_end) {
        window_end -= 1;
    }
    let Some(window) = text.get(window_start..window_end) else {
        return false;
    };
    let lower = window.to_lowercase();
    global()
        .ordered_sequence_cues
        .iter()
        .any(|cue| contains_word_phrase(&lower, &cue.to_lowercase()))
}

/// Contextual year test for a four-digit value. A following unit, scale or
/// count cue proves it is a quantity; a temporal preposition, a year suffix or
/// the sentence-initial position of a 20th/21st-century year proves it is a
/// date. The digits alone never decide.
pub fn year_context(text: &str, start: usize, end: usize, year: u32) -> bool {
    if !(1000..=2099).contains(&year) {
        return false;
    }
    if attached_year_suffix(text, end) {
        return true;
    }
    if (1900..=2099).contains(&year) && year_noun_follows(text, end) {
        return true;
    }
    if quantity_word_follows(text, end) {
        return false;
    }
    // A month name before the number is an explicit calendar anchor even
    // without a temporal preposition ("giugno 1988", "June 1988").
    let prefix = text.get(..start).unwrap_or_default().trim_end();
    let previous_word = prefix
        .split_whitespace()
        .last()
        .unwrap_or("")
        .to_lowercase();
    if crate::lexicon::lexicon()
        .month_typing_names
        .iter()
        .any(|month| month == &previous_word)
    {
        return true;
    }
    // A modern bare year takes the article ("il 2024", "the 2024") without
    // becoming a quantity; historical counts stay untouched.
    if (1900..=2099).contains(&year)
        && global()
            .year_article_cues
            .iter()
            .any(|article| article.eq_ignore_ascii_case(&previous_word))
    {
        return true;
    }
    // The second year of a range stays a date even when the conjunction
    // (`e`, `and`) is not a temporal preposition.
    if year_range_continues(prefix, year) {
        return true;
    }
    for cue in &global().year_context_cues {
        let Some(cue_start) = suffix_word_start(prefix, cue) else {
            continue;
        };
        // A year noun that follows an already-seen four-digit year is a
        // postfix ("w 2016 roku 1200"), not a cue for the next number. Only
        // nouns have postfix uses: a preposition after a quantity
        // ("1500 nel 2021") is still a cue.
        let postfix_capable = global()
            .year_suffix_cues
            .iter()
            .any(|suffix| suffix.eq_ignore_ascii_case(cue));
        let postfix = postfix_capable
            && prefix[..cue_start]
                .trim_end()
                .split_whitespace()
                .last()
                .is_some_and(|word| {
                    word.len() == 4
                        && word
                            .parse::<u32>()
                            .is_ok_and(|year| (1000..=2099).contains(&year))
                });
        if !postfix {
            return true;
        }
    }
    (1900..=2099).contains(&year) && at_sentence_start(text, start)
}

fn year_range_continues(prefix: &str, year: u32) -> bool {
    if !(1900..=2099).contains(&year) {
        return false;
    }
    let clause_start = prefix
        .char_indices()
        .filter(|(_, ch)| matches!(ch, '.' | ';' | ':' | '!' | '?' | '\n'))
        .map(|(index, ch)| index + ch.len_utf8())
        .next_back()
        .unwrap_or(0);
    let clause = prefix[clause_start..].to_lowercase();
    let trimmed = clause.trim_start();
    global().year_range_openers.iter().any(|cue| {
        let cue = cue.to_lowercase();
        trimmed.starts_with(&cue)
            && trimmed
                .get(cue.len()..)
                .is_none_or(|rest| rest.chars().next().is_none_or(|ch| !ch.is_alphanumeric()))
    })
}

fn contains_word_phrase(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(index) = haystack[from..].find(needle) {
        let start = from + index;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_alphanumeric());
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = start + needle.chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn attached_year_suffix(text: &str, end: usize) -> bool {
    let tail = text.get(end..).unwrap_or_default();
    let mut chars = tail.chars();
    if !matches!(chars.next(), Some('\'' | '’')) {
        return false;
    }
    let count = chars.take_while(|ch| ch.is_alphabetic()).take(4).count();
    (2..=4).contains(&count)
}

fn year_noun_follows(text: &str, end: usize) -> bool {
    let tail = text
        .get(end..)
        .unwrap_or_default()
        .trim_start()
        .to_lowercase();
    global()
        .year_suffix_cues
        .iter()
        .any(|cue| tail.starts_with(cue.as_str()) && has_word_boundary(&tail, cue.len()))
}

/// The first word after a number proves it is a quantity when it names a
/// scale, unit, measurement, currency, percentage or curated count unit.
fn quantity_word_follows(text: &str, end: usize) -> bool {
    let tail = text
        .get(end..)
        .unwrap_or_default()
        .trim_start()
        .to_lowercase();
    let Some(word) = tail
        .split(|ch: char| !ch.is_alphanumeric())
        .find(|word| !word.is_empty())
    else {
        return false;
    };
    let scale = &crate::lexicon::lexicon().scale_words;
    scale.words.iter().any(|cue| cue.as_str() == word)
        || scale
            .english_scale_words
            .iter()
            .any(|cue| cue.as_str() == word)
        || scale.unit_words.iter().any(|cue| cue.as_str() == word)
        || scale
            .measurement_units
            .iter()
            .any(|cue| cue.as_str() == word)
        || scale.currency_words.iter().any(|cue| cue.as_str() == word)
        || scale
            .percentage_words
            .iter()
            .any(|cue| cue.as_str() == word)
        || global()
            .count_unit_cues
            .iter()
            .any(|cue| cue.as_str() == word)
}

fn suffix_word_start(text: &str, cue: &str) -> Option<usize> {
    if cue.is_empty() {
        return None;
    }
    let start = text.len().checked_sub(cue.len())?;
    if !text.is_char_boundary(start) || !text[start..].eq_ignore_ascii_case(cue) {
        return None;
    }
    // The cue must not be the tail of an apostrophe-attached year suffix:
    // Turkish `2018'de` must not make the following value a year via `de`.
    if start > 0 && matches!(text[..start].chars().next_back(), Some('\'' | '’')) {
        return None;
    }
    if start > 0
        && text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    Some(start)
}

fn at_sentence_start(text: &str, start: usize) -> bool {
    let prefix = text.get(..start).unwrap_or_default().trim_end();
    prefix.is_empty()
        || prefix.ends_with('.')
        || prefix.ends_with('!')
        || prefix.ends_with('?')
        || prefix.ends_with(':')
        || prefix.ends_with(';')
        || prefix.ends_with('\n')
}

fn has_word_boundary(text: &str, end: usize) -> bool {
    text.get(end..)
        .and_then(|tail| tail.chars().next())
        .is_none_or(|next| !next.is_alphanumeric())
}

/// Language-scoped cues, falling back to English for an unsupported tag. The
/// global table is always available through [`global`].
pub fn language_cues(language: &str) -> &'static LanguageChartCues {
    static EMPTY: OnceLock<LanguageChartCues> = OnceLock::new();
    let base = language.split(['-', '_']).next().unwrap_or(language);
    chart_cues()
        .by_language
        .get(base)
        .or_else(|| chart_cues().by_language.get("en"))
        .unwrap_or_else(|| EMPTY.get_or_init(LanguageChartCues::default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_language_has_chart_cues() {
        let file = chart_cues();
        for language in [
            "de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr",
        ] {
            assert!(
                file.by_language.contains_key(language),
                "missing chart cues for {language}"
            );
            let cues = language_cues(language);
            assert!(!cues.series_metric_stems.is_empty(), "{language}");
            assert!(!cues.semicolon_metric_stems.is_empty(), "{language}");
            assert!(!cues.share_context_cues.is_empty(), "{language}");
        }
    }

    #[test]
    fn global_vetoes_and_units_are_data_not_code() {
        let global = global();
        assert!(global
            .hard_vetoes
            .iter()
            .any(|cue| cue == &["not".to_string(), "true".to_string()]));
        assert!(global.currency_cues.contains_key("EUR"));
        assert!(global.count_unit_cues.iter().any(|cue| cue == "veicoli"));
        assert!(global.rate_cues.iter().any(|cue| cue == "al mese"));
    }
}
