//! Per-language pipeline profiles and the candidate-exclusion vocabularies:
//! what the mechanism must know about a language (treebank, date order,
//! number grouping) lives in data, never in a `match`.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Candidate-exclusion vocabularies (visualner stopword fallback).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Stopwords {
    pub stop_phrases: Vec<String>,
    pub title_words: Vec<String>,
    /// Narration boilerplate that must never become an entity
    /// ("Imagine the", "Get ready", ...).
    pub generic_phrases: Vec<String>,
    /// Articulated prepositions that UDPipe splits into preposition +
    /// article ("del" -> "di" + "il"). The tagger alignment uses this closed
    /// morphosyntactic class to keep the contraction's tag.
    #[serde(default)]
    pub articulated_prepositions: Vec<String>,
    /// Narration filler phrases ("non è così", "ancora oggi", "per questo").
    /// Their text stays in the scene inventory, but a surface containing one
    /// must never become a search query: it is discourse noise, not a visual
    /// subject. Curated per language inside one global table, because a
    /// phrase only ever matches its own language's text.
    #[serde(default)]
    pub filler_phrases: Vec<String>,
}

impl Stopwords {
    /// True when the surface contains a curated filler phrase as consecutive
    /// whole words. Comparison is per word, case-folded and punctuation-
    /// tolerant ("Non è così." matches "non è così"), so a filler is matched
    /// inside a longer sentence without ever matching inside a bigger word.
    pub fn contains_filler_phrase(&self, text: &str) -> bool {
        let words: Vec<String> = text
            .split_whitespace()
            .map(|word| {
                word.trim_matches(|ch: char| !ch.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|word| !word.is_empty())
            .collect();
        self.filler_phrases.iter().any(|phrase| {
            let filler: Vec<String> = phrase
                .split_whitespace()
                .map(|word| word.to_lowercase())
                .collect();
            !filler.is_empty()
                && words
                    .windows(filler.len())
                    .any(|window| window.iter().zip(&filler).all(|(word, part)| word == part))
        })
    }
}

/// Day/month order a numeric date uses when both readings are calendar-legal
/// (`03/04/2020`). Only the convention `date_order` records is applied; a
/// source that cannot be read either way stays unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateOrder {
    DayFirst,
    MonthFirst,
}

/// One language's pipeline profile.
///
/// These are the facts the *mechanism* needs to behave correctly in a
/// language: which UDPipe treebank supplies tags and lemmas, which date order
/// its writers use, how it groups and splits numbers, and whether its verbal
/// morphology is certified for predicate filtering. They live here, in data,
/// so adding a language is an entry — never a `match` in Rust.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LanguageProfile {
    /// UDPipe treebank whose model supplies POS tags and lemmas. Absent when
    /// no model is certified: the extractor then runs the model-free
    /// fallback instead of guessing.
    #[serde(default)]
    pub treebank: Option<String>,
    /// Writing system (key into [`LanguageProfiles::scripts`]). Mandatory for
    /// every non-`default` entry: the case behavior of the script decides
    /// whether an uppercase initial is evidence of a proper name.
    #[serde(default)]
    pub script: Option<String>,
    /// The orthography capitalizes common nouns (German). Capitalization then
    /// stops being evidence of a name: every noun is capitalized.
    #[serde(default)]
    pub capitalized_nouns: bool,
    #[serde(default)]
    pub date_order: Option<DateOrder>,
    #[serde(default)]
    pub thousands_separator: Option<char>,
    #[serde(default)]
    pub decimal_separator: Option<char>,
    /// Enables the verbal-predicate filter (tag `VERB`/`AUX` plus the lemma
    /// fallback). Off until a language's morphology is certified.
    #[serde(default)]
    pub verb_filter: bool,
    /// Infinitive endings that let the lemma prove an inflected form
    /// (`guarda` → `guardare`). Empty disables the lemma branch: the tag
    /// alone is never enough to overrule a noun with a nominal lemma.
    #[serde(default)]
    pub infinitive_suffixes: Vec<String>,
    /// The UDPipe model lowercases proper-noun lemmas as a convention
    /// (Spanish `Lucía` -> `lucía`, Portuguese -> `_`). A lowercase lemma then
    /// says nothing about the surface, so PROPN alone is name evidence.
    #[serde(default)]
    pub lowercase_propn_lemmas: bool,
}

/// One writing system's case behavior. Scripts without letter case (Han,
/// Japanese, Hangul, Arabic, Devanagari, Thai) have no uppercase initial, so
/// requiring one would make every proper name in those scripts invisible.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScriptProfile {
    /// Mandatory: a script that forgets to declare this must not be guessed.
    #[serde(default)]
    pub cased: Option<bool>,
}

/// Per-language profiles, keyed by ISO-639-1 base code, plus the writing
/// systems they use. The `default` entry supplies any field a language does
/// not override.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LanguageProfiles {
    #[serde(default)]
    pub profiles: BTreeMap<String, LanguageProfile>,
    #[serde(default)]
    pub scripts: BTreeMap<String, ScriptProfile>,
}

impl LanguageProfiles {
    fn entry(&self, language: &str) -> Option<&LanguageProfile> {
        let key = language
            .trim()
            .split(['-', '_'])
            .next()?
            .to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        self.profiles.get(&key)
    }

    /// The merged view: the language's own entry, falling back to `default`.
    fn effective(&self, language: &str) -> Option<&LanguageProfile> {
        self.entry(language).or_else(|| self.entry("default"))
    }

    /// UDPipe treebank for a BCP-47 tag. Deliberately does NOT fall back to
    /// `default`: a language without a certified model must run the
    /// model-free path, never another language's model.
    pub fn treebank(&self, language: &str) -> Option<&str> {
        self.entry(language)?.treebank.as_deref()
    }

    /// Writing system of a language, when the table declares one.
    pub fn script(&self, language: &str) -> Option<&ScriptProfile> {
        let name = self.entry(language)?.script.as_deref()?;
        self.scripts.get(name)
    }

    /// True when the language's script has letter case. An unknown language
    /// keeps the historical cased behavior: only an explicit uncased script
    /// entry removes the capitalization requirement.
    pub fn is_cased(&self, language: &str) -> bool {
        self.script(language)
            .is_none_or(|script| script.cased.unwrap_or(true))
    }

    /// True when the orthography capitalizes common nouns (German).
    pub fn capitalized_nouns(&self, language: &str) -> bool {
        self.entry(language)
            .is_some_and(|profile| profile.capitalized_nouns)
    }

    /// True when the certified model lowercases proper-noun lemmas.
    pub fn lowercase_propn_lemmas(&self, language: &str) -> bool {
        self.entry(language)
            .is_some_and(|profile| profile.lowercase_propn_lemmas)
    }

    /// The pivot: does an uppercase initial prove a proper name? Only in a
    /// cased script that does not capitalize ordinary nouns. In German every
    /// noun is capitalized, and in Han/Japanese/Arabic/Thai there is no case,
    /// so the capitalization test carries no information there.
    pub fn uppercase_is_name_evidence(&self, language: &str) -> bool {
        self.is_cased(language) && !self.capitalized_nouns(language)
    }

    /// Day/month order for numeric dates (day-first when unconfigured).
    pub fn date_order(&self, language: &str) -> DateOrder {
        self.effective(language)
            .and_then(|profile| profile.date_order)
            .unwrap_or(DateOrder::DayFirst)
    }

    pub fn thousands_separator(&self, language: &str) -> char {
        self.effective(language)
            .and_then(|profile| profile.thousands_separator)
            .unwrap_or('.')
    }

    pub fn decimal_separator(&self, language: &str) -> char {
        self.effective(language)
            .and_then(|profile| profile.decimal_separator)
            .unwrap_or(',')
    }

    pub fn verb_filter(&self, language: &str) -> bool {
        self.effective(language)
            .is_some_and(|profile| profile.verb_filter)
    }

    pub fn infinitive_suffixes(&self, language: &str) -> &[String] {
        self.entry(language)
            .map(|profile| profile.infinitive_suffixes.as_slice())
            .unwrap_or(&[])
    }

    /// Checks the table's own invariants. An empty result means every profile
    /// is usable; a non-empty result names the broken entry, so a data edit
    /// fails a test instead of silently degrading an analysis.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.scripts.is_empty() {
            problems.push("no `scripts` table".to_string());
        }
        for (code, script) in &self.scripts {
            if code.trim().is_empty() {
                problems.push("a script entry has an empty name".to_string());
            }
            if script.cased.is_none() {
                problems.push(format!("script {code:?}: missing mandatory `cased`"));
            }
        }
        for (code, profile) in &self.profiles {
            let is_default = code == "default";
            if !is_default && (code.len() != 2 || !code.chars().all(|c| c.is_ascii_lowercase())) {
                problems.push(format!(
                    "profile {code:?}: expected a two-letter lowercase language code"
                ));
            }
            if is_default {
                continue;
            }
            match profile.script.as_deref() {
                None => problems.push(format!("profile {code:?}: missing mandatory `script`")),
                Some(script) if !self.scripts.contains_key(script) => {
                    problems.push(format!("profile {code:?}: unknown script {script:?}"))
                }
                Some(_) => {}
            }
            if let Some(treebank) = profile.treebank.as_deref() {
                if treebank.trim().is_empty() {
                    problems.push(format!("profile {code:?}: empty `treebank`"));
                }
            }
            if profile.verb_filter && profile.infinitive_suffixes.is_empty() {
                problems.push(format!(
                    "profile {code:?}: `verb_filter` needs at least one `infinitive_suffixes` entry"
                ));
            }
            if profile
                .infinitive_suffixes
                .iter()
                .any(|suffix| suffix.trim().is_empty())
            {
                problems.push(format!(
                    "profile {code:?}: an `infinitive_suffixes` entry is empty"
                ));
            }
            if let (Some(thousands), Some(decimal)) =
                (profile.thousands_separator, profile.decimal_separator)
            {
                if thousands == decimal {
                    problems.push(format!(
                        "profile {code:?}: thousands and decimal separators are both {thousands:?}"
                    ));
                }
            }
        }
        problems
    }
}
