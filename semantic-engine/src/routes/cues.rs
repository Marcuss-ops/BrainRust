//! Route cue packs and lexical helpers.

use semantic_core::Place;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const CUES_JSON: &str = include_str!("../../../lexicon/route_cues.json");

#[derive(Deserialize)]
struct CueFile {
    languages: BTreeMap<String, RouteCues>,
}

#[derive(Deserialize)]
pub(super) struct RouteCues {
    pub(super) origin: Vec<String>,
    pub(super) destination: Vec<String>,
    pub(super) motion: Vec<String>,
    pub(super) air: Vec<String>,
    pub(super) road: Vec<String>,
    pub(super) rail: Vec<String>,
    pub(super) sea: Vec<String>,
    #[serde(default)]
    pub(super) waypoint: Vec<String>,
}

pub(super) fn cues(language: &str) -> Option<&'static RouteCues> {
    static CUES: OnceLock<CueFile> = OnceLock::new();
    CUES.get_or_init(|| serde_json::from_str(CUES_JSON).expect("route cue data must be valid"))
        .languages
        .get(language.split('-').next().unwrap_or(language))
}

pub(super) fn prefix_cue_start(prefix: &str, terms: &[String]) -> Option<usize> {
    let trimmed = prefix.trim_end();
    let lower = trimmed.to_lowercase();
    terms
        .iter()
        .filter_map(|term| {
            let term = term.to_lowercase();
            let start = lower.strip_suffix(&term)?.len();
            (start == 0
                || lower[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|ch| ch.is_whitespace() || ch == '\''))
            .then_some(start)
        })
        .max_by_key(|start| *start)
}

fn contains_term(text: &str, term: &str) -> bool {
    let lower = text.to_lowercase();
    let term = term.to_lowercase();
    lower.match_indices(&term).any(|(start, matched)| {
        let end = start + matched.len();
        let before = lower[..start].chars().next_back();
        let after = lower[end..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

pub(super) fn has_any(text: &str, terms: &[String]) -> bool {
    terms.iter().any(|term| contains_term(text, term))
}

pub(super) fn first_term_offset(text: &str, terms: &[String]) -> Option<usize> {
    let lower = text.to_lowercase();
    terms
        .iter()
        .filter_map(|term| {
            let term = term.to_lowercase();
            lower.match_indices(&term).find_map(|(start, matched)| {
                let end = start + matched.len();
                let before = lower[..start].chars().next_back();
                let after = lower[end..].chars().next();
                (!before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric))
                .then_some(start)
            })
        })
        .min()
}

pub(super) fn term_offsets(text: &str, terms: &[String]) -> Vec<usize> {
    let lower = text.to_lowercase();
    let mut offsets: Vec<usize> = terms
        .iter()
        .flat_map(|term| {
            let term = term.to_lowercase();
            let lower = &lower;
            lower
                .match_indices(&term)
                .filter_map(move |(start, matched)| {
                    let end = start + matched.len();
                    let before = lower[..start].chars().next_back();
                    let after = lower[end..].chars().next();
                    (!before.is_some_and(char::is_alphanumeric)
                        && !after.is_some_and(char::is_alphanumeric))
                    .then_some(start)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

pub(super) fn same_clause(text: &str, left: usize, right: usize) -> bool {
    text.get(left.min(right)..left.max(right))
        .is_some_and(|gap| {
            !gap.chars()
                .any(|ch| matches!(ch, ',' | ';' | ':' | '!' | '?'))
        })
}

pub(super) fn status(evidence: &str) -> &'static str {
    let lower = evidence.to_lowercase();
    if [
        " not ", " never ", " non ", " mai ", " nunca ", " nie ", " не ",
    ]
    .iter()
    .any(|cue| format!(" {lower} ").contains(cue))
    {
        "negated"
    } else if [" would ", " might ", " hypothetically ", " forse ", " se "]
        .iter()
        .any(|cue| format!(" {lower} ").contains(cue))
    {
        "uncertain"
    } else {
        "asserted"
    }
}

pub(super) fn mode(text: &str, cues: &RouteCues) -> &'static str {
    if has_any(text, &cues.air) {
        "air"
    } else if has_any(text, &cues.road) {
        "road"
    } else if has_any(text, &cues.rail) {
        "rail"
    } else if has_any(text, &cues.sea) {
        "sea"
    } else {
        "unknown"
    }
}

pub(super) fn distance_km(origin: &Place, destination: &Place) -> Option<f64> {
    let (lat1, lon1, lat2, lon2) = (origin.lat?, origin.lon?, destination.lat?, destination.lon?);
    let (lat1, lat2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = lat2 - lat1;
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    Some(6371.0088 * 2.0 * a.sqrt().atan2((1.0 - a).sqrt()))
}
