//! Named-set language profiles.
//!
//! `lexicon/tools/build_language_packs.py` emits one profile per
//! `lexicon/languages/<lang>` directory. Unlike the fused extractor pack in
//! [`crate::language_packs`], this profile keeps the inventories separate:
//! stop words, function words, entity blocklist, negative particles, visual
//! verbs, verb suffixes, the named word sets and the phrase policy. Consumers
//! that need one specific set must not infer it from the fused closed class.
//!
//! This is the Rust authority for the former Go `LexiconRegistry` data.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PhrasePolicy {
    pub min_words: usize,
    pub max_words: usize,
    pub max_results: usize,
    pub reject_verbs_when_all: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LanguageProfile {
    pub schema: String,
    pub language: String,
    pub source: String,
    pub stopwords: Vec<String>,
    pub function_words: Vec<String>,
    pub entity_blocklist: Vec<String>,
    /// Whole queries the map/geocode gate rejects: discourse words a tagger
    /// can title-case into a name, and institutions that are never a pin.
    #[serde(default)]
    pub place_candidate_blocklist: Vec<String>,
    /// Tokens that disqualify any map candidate containing them, matched
    /// inside multiword extractions ("Suprema Corte", "La Cassazione").
    #[serde(default)]
    pub place_candidate_token_blocklist: Vec<String>,
    pub negative_particles: Vec<String>,
    pub visual_verbs: Vec<String>,
    pub verb_suffixes: Vec<String>,
    pub word_sets: HashMap<String, Vec<String>>,
    pub phrase_policy: PhrasePolicy,
}

fn profiles() -> &'static HashMap<String, LanguageProfile> {
    static PROFILES: OnceLock<HashMap<String, LanguageProfile>> = OnceLock::new();
    PROFILES.get_or_init(|| {
        let mut out = HashMap::new();
        for (language, json) in crate::generated_language_profiles::PROFILES {
            let Ok(profile) = serde_json::from_str::<LanguageProfile>(json) else {
                continue;
            };
            out.insert((*language).to_string(), profile);
        }
        out
    })
}

/// Resolves a BCP-47 tag (`it-IT`) to its base profile (`it`).
pub fn get(language: &str) -> Option<&'static LanguageProfile> {
    let key = language.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    let base = key
        .split(['-', '_'])
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(&key);
    profiles().get(&key).or_else(|| profiles().get(base))
}

/// The cross-linguistic profile used only when a caller explicitly asks for
/// it. Unknown languages do not silently inherit another language's words.
pub fn fallback() -> Option<&'static LanguageProfile> {
    profiles().get("fallback")
}

/// Languages with a committed profile, for diagnostics and coverage tests.
pub fn supported_languages() -> Vec<&'static str> {
    let mut languages: Vec<&'static str> = profiles().keys().map(String::as_str).collect();
    languages.sort_unstable();
    languages
}

/// Deterministic schema version over the committed profile set. Go validates
/// this before installing a snapshot so a stale build fails closed.
pub fn schema_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        // Hash the exact committed JSON strings, not a re-serialized map:
        // HashMap iteration order would make the version nondeterministic.
        let mut hash: u64 = 0xcbf29ce484222325;
        for (language, json) in crate::generated_language_profiles::PROFILES {
            for byte in language.as_bytes().iter().chain(json.as_bytes()) {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        format!("language_profiles.v1-{hash:016x}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_carries_named_sets_not_a_fused_bag() {
        let profile = get("it").expect("it profile");
        assert_eq!(profile.schema, "language_profile.v1");
        assert!(!profile.stopwords.is_empty());
        assert!(!profile.verb_suffixes.is_empty());
        assert!(profile.word_sets.contains_key("pronouns"));
        assert!(profile.word_sets.contains_key("subordinate_markers"));
    }

    #[test]
    fn map_candidate_gate_ships_as_data() {
        let profile = get("it").expect("it profile");
        assert!(profile
            .place_candidate_blocklist
            .contains(&"infine".to_string()));
        assert!(profile
            .place_candidate_blocklist
            .contains(&"tribunale".to_string()));
        assert!(profile
            .place_candidate_token_blocklist
            .contains(&"cassazione".to_string()));
        // A language with no map sources ships an empty gate rather than
        // inheriting the Italian words.
        let en = get("en").expect("en profile");
        assert!(en.place_candidate_blocklist.is_empty());
        assert!(en.place_candidate_token_blocklist.is_empty());
    }

    #[test]
    fn fallback_profile_is_shipped_for_unknown_languages() {
        assert!(get("zz").is_none());
        let profile = fallback().expect("fallback profile");
        assert_eq!(profile.language, "fallback");
        assert!(profile.word_sets.contains_key("numeric_overlay_words"));
        assert!(profile.word_sets.contains_key("numeric_overlay_context"));
    }

    #[test]
    fn policy_is_parsed_per_language() {
        assert_eq!(get("it").expect("it").phrase_policy.max_words, 6);
        assert_eq!(
            get("fallback").expect("fallback").phrase_policy.max_words,
            16
        );
    }

    #[test]
    fn shared_languages_ship_named_sets_and_morphology() {
        for language in ["de", "en", "es", "fr", "it", "pt"] {
            let profile = get(language).unwrap_or_else(|| panic!("{language} profile"));
            assert!(!profile.stopwords.is_empty(), "{language} stopwords");
            assert!(
                !profile.function_words.is_empty(),
                "{language} function words"
            );
            assert!(!profile.word_sets.is_empty(), "{language} named word sets");
        }
    }

    #[test]
    fn version_is_stable_and_non_empty() {
        assert!(schema_version().starts_with("language_profiles.v1-"));
        assert_eq!(schema_version(), schema_version());
    }
}
