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
}

fn parse() -> ChartCueFile {
    let file: ChartCueFile =
        serde_json::from_str(include_str!("../../lexicon/chart_cues.json"))
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
