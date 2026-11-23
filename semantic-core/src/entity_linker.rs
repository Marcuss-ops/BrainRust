//! Deterministic, local-only entity linking for the v3 manifest projection.
//!
//! The index is a small, versioned seed set packaged with the binary. It is
//! deliberately not a claim of broad world coverage: it gives the MVP a
//! reproducible KB id, a short gloss and verifier exclusions for the release
//! corpus, with no network or filesystem I/O at analysis time.
//!
//! Ambiguous aliases are resolved by deterministic context scoring over the
//! scenes where the identity occurs. A tie, or missing evidence, returns
//! `ambiguous`; an alias outside the index returns `unlinked`. A surface slug
//! is never labelled as a linked KB id.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::identity::{
    editorial_decision, normalize_surface, role_of, stable_surface_key, CanonicalIdentity,
    EntityRole, LinkCandidate, LinkEvidenceSpan, LinkFeature, LinkStatus, LinkedEntity,
    SearchIntent,
};

/// Version recorded in analyzer provenance and health output.
pub const ENTITY_INDEX_SCHEMA: &str = "entity_index.v1";
const LINK_WEIGHTS_JSON: &str = include_str!("../../lexicon/link_weights_v1.json");
const TYPE_CUES_JSON: &str = include_str!("../../lexicon/type_cues.json");

#[derive(Debug, Deserialize)]
struct LinkWeights {
    version: String,
    alias_exactness_weight: i32,
    type_prior_weight: i32,
    context_overlap_weight: i32,
    counter_penalty_weight: i32,
    type_cue_weight: i32,
    discourse_prior_weight: i32,
    single_candidate_min_score: i32,
    multi_candidate_min_score: i32,
    multi_candidate_min_margin: i32,
    max_candidates: usize,
    max_features: usize,
}

#[derive(Debug, Deserialize)]
struct TypeCueFile {
    schema: String,
    languages: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

fn type_cues() -> &'static TypeCueFile {
    static CUES: OnceLock<TypeCueFile> = OnceLock::new();
    CUES.get_or_init(|| serde_json::from_str(TYPE_CUES_JSON).expect("valid entity type cues"))
}

fn link_weights() -> &'static LinkWeights {
    static WEIGHTS: OnceLock<LinkWeights> = OnceLock::new();
    WEIGHTS.get_or_init(|| serde_json::from_str(LINK_WEIGHTS_JSON).expect("valid linker weights"))
}

pub fn linker_version() -> &'static str {
    &link_weights().version
}

pub fn linker_weights_sha256() -> String {
    Sha256::digest(LINK_WEIGHTS_JSON.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
struct EntityIndexFile {
    version: String,
    entries: Vec<EntityIndexEntry>,
}

/// One auditable seed entry. `aliases` are keyed by BCP-47 language but matched
/// across languages so IT/EN/PT mentions converge on the same canonical id.
#[derive(Debug, Clone, Deserialize)]
pub struct EntityIndexEntry {
    pub id: String,
    pub canonical_name: String,
    pub entity_type: String,
    pub gloss: String,
    #[serde(default)]
    pub negative_constraints: Vec<String>,
    #[serde(default)]
    pub aliases: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub context_terms: Vec<String>,
    #[serde(default)]
    pub counter_terms: Vec<String>,
}

/// The immutable index. Aliases are normalized once at first use.
#[derive(Debug)]
pub struct EntityIndex {
    pub version: String,
    entries: Vec<EntityIndexEntry>,
    aliases: BTreeMap<String, Vec<usize>>,
}

impl EntityIndex {
    /// Number of auditable seed entries, reported by health/readiness.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn candidates(&self, surface: &str) -> Vec<&EntityIndexEntry> {
        let key = normalize_surface(surface);
        self.aliases
            .get(&key)
            .map(|indices| indices.iter().map(|index| &self.entries[*index]).collect())
            .unwrap_or_default()
    }
}

/// Loads the packaged index once per process. No network, no runtime file read.
pub fn entity_index() -> &'static EntityIndex {
    static INDEX: OnceLock<EntityIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let file: EntityIndexFile =
            serde_json::from_str(include_str!("../../lexicon/entity_index_v1.json"))
                .expect("embedded entity index must be valid JSON");
        let mut aliases: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, entry) in file.entries.iter().enumerate() {
            let mut add = |surface: &str| {
                let key = normalize_surface(surface);
                if key.is_empty() {
                    return;
                }
                let indices = aliases.entry(key).or_default();
                if !indices.contains(&index) {
                    indices.push(index);
                }
            };
            add(&entry.canonical_name);
            for alias in entry.aliases.values().flatten() {
                add(alias);
            }
        }
        EntityIndex {
            version: file.version,
            entries: file.entries,
            aliases,
        }
    })
}

/// Version of the packaged index, recorded on the manifest and health output.
pub fn entity_index_version() -> &'static str {
    &entity_index().version
}

/// Resolves every canonical identity against the packaged index and builds the
/// v3 projection. The output order follows registry order (deterministic).
pub fn link_all(
    language: &str,
    scenes: &[(&str, &str)],
    identities: &[CanonicalIdentity],
) -> Vec<LinkedEntity> {
    link_all_with_threads(language, scenes, identities, &[])
}

mod linking;
pub use linking::*;
fn explain_candidate(
    score: i32,
    entry: &EntityIndexEntry,
    language: &str,
    identity: &CanonicalIdentity,
    scenes: &[(&str, &str)],
    discourse_prior: Option<&DiscourseSense>,
) -> LinkCandidate {
    let exact_alias = entry.canonical_name == identity.canonical_surface
        || entry
            .aliases
            .values()
            .flatten()
            .any(|alias| alias == &identity.canonical_surface);
    let mut alias_spans: Vec<LinkEvidenceSpan> = identity
        .mentions
        .iter()
        .map(|mention| LinkEvidenceSpan {
            scene_id: mention.scene_id.clone(),
            start_byte: mention.start_byte,
            end_byte: mention.end_byte,
        })
        .collect();
    alias_spans.sort();
    alias_spans.dedup();
    let mut alias_terms: Vec<String> = identity
        .mentions
        .iter()
        .map(|mention| mention.surface.clone())
        .collect();
    alias_terms.push(identity.canonical_surface.clone());
    alias_terms.sort();
    alias_terms.dedup();
    let mut context_terms = Vec::new();
    let mut context_spans: Vec<LinkEvidenceSpan> = Vec::new();
    for term in &entry.context_terms {
        let spans = evidence_spans(term, identity, scenes);
        if !spans.is_empty() {
            context_terms.push(term.clone());
            context_spans.extend(spans);
        }
    }
    let mut counter_terms = Vec::new();
    let mut counter_spans: Vec<LinkEvidenceSpan> = Vec::new();
    for term in &entry.counter_terms {
        let spans = evidence_spans(term, identity, scenes);
        if !spans.is_empty() {
            counter_terms.push(term.clone());
            counter_spans.extend(spans);
        }
    }
    context_spans.sort();
    context_spans.dedup();
    counter_spans.sort();
    counter_spans.dedup();
    let type_match = entry.entity_type.eq_ignore_ascii_case(&identity.kind)
        || (entry.entity_type == "ORG" && identity.kind == "ORGANIZATION");
    let context = identity_context(identity, scenes);
    let cue_terms = matching_type_cues(language, &entry.entity_type, &context);
    let cue_spans: Vec<LinkEvidenceSpan> = cue_terms
        .iter()
        .flat_map(|term| evidence_spans(term, identity, scenes))
        .collect();

    let mut features = vec![
        LinkFeature {
            name: "alias_exactness".to_string(),
            value: if exact_alias {
                link_weights().alias_exactness_weight
            } else {
                0
            },
            terms: alias_terms,
            spans: alias_spans,
        },
        LinkFeature {
            name: "type_prior".to_string(),
            value: if type_match {
                link_weights().type_prior_weight
            } else {
                0
            },
            terms: vec![identity.kind.clone()],
            spans: Vec::new(),
        },
        LinkFeature {
            name: "context_overlap".to_string(),
            value: link_weights().context_overlap_weight * context_terms.len() as i32,
            terms: context_terms,
            spans: context_spans,
        },
        LinkFeature {
            name: "counter_penalty".to_string(),
            value: link_weights().counter_penalty_weight * counter_terms.len() as i32,
            terms: counter_terms,
            spans: counter_spans,
        },
        LinkFeature {
            name: "type_cue".to_string(),
            value: link_weights().type_cue_weight * cue_terms.len() as i32,
            terms: cue_terms,
            spans: cue_spans,
        },
    ];
    if let Some(prior) = discourse_prior {
        features.push(LinkFeature {
            name: "discourse_prior".to_string(),
            value: link_weights().discourse_prior_weight,
            terms: prior.terms.clone(),
            spans: prior.spans.clone(),
        });
    }
    features.truncate(link_weights().max_features);
    LinkCandidate {
        kb_id: entry.id.clone(),
        name: entry.canonical_name.clone(),
        score,
        features,
    }
}

fn evidence_spans(
    term: &str,
    identity: &CanonicalIdentity,
    scenes: &[(&str, &str)],
) -> Vec<LinkEvidenceSpan> {
    if term.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    for mention in &identity.mentions {
        let Some((_, text)) = scenes
            .iter()
            .find(|(scene_id, _)| *scene_id == mention.scene_id)
        else {
            continue;
        };
        let (window_start, window_end) =
            sentence_window_bounds(text, mention.start_byte, mention.end_byte);
        let Some(window) = text.get(window_start..window_end) else {
            continue;
        };
        for (start, end) in term_ranges(window, term) {
            spans.push(LinkEvidenceSpan {
                scene_id: mention.scene_id.clone(),
                start_byte: window_start + start,
                end_byte: window_start + end,
            });
        }
    }
    spans.sort();
    spans.dedup();
    spans
}

/// Deterministic evidence score. Context terms add support, counter terms
/// remove it, a spelled-out canonical name is the strongest single signal, and
/// a matching type is a small prior. A tie is always ambiguous.
fn context_score(language: &str, entry: &EntityIndexEntry, kind: &str, context: &str) -> i32 {
    let mut score = 0;
    for term in &entry.context_terms {
        if !term.is_empty() && contains_term(context, term) {
            score += link_weights().context_overlap_weight;
        }
    }
    for term in &entry.counter_terms {
        if !term.is_empty() && contains_term(context, term) {
            score += link_weights().counter_penalty_weight;
        }
    }
    if entry.entity_type.eq_ignore_ascii_case(kind)
        || (entry.entity_type == "ORG" && kind == "ORGANIZATION")
    {
        score += link_weights().type_prior_weight;
    }
    score += link_weights().type_cue_weight
        * matching_type_cues(language, &entry.entity_type, context).len() as i32;
    score
}

fn identity_context(identity: &CanonicalIdentity, scenes: &[(&str, &str)]) -> String {
    identity
        .mentions
        .iter()
        .filter_map(|mention| {
            scenes
                .iter()
                .find(|(scene_id, _)| *scene_id == mention.scene_id)
                .map(|(_, text)| sentence_window(text, mention.start_byte, mention.end_byte))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn matching_type_cues(language: &str, entity_type: &str, context: &str) -> Vec<String> {
    let file = type_cues();
    debug_assert_eq!(file.schema, "entity_type_cues.v1");
    let cues = file
        .languages
        .get(language)
        .or_else(|| file.languages.get("*"));
    let class = if entity_type.eq_ignore_ascii_case("ORGANIZATION") {
        "ORG"
    } else {
        entity_type
    };
    let Some(terms) = cues.and_then(|classes| classes.get(&class.to_ascii_uppercase())) else {
        return Vec::new();
    };
    terms
        .iter()
        .filter(|term| contains_term(context, term))
        .cloned()
        .collect()
}

fn contains_term(text: &str, term: &str) -> bool {
    let lower = text.to_lowercase();
    let needle = term.to_lowercase();
    lower.match_indices(&needle).any(|(start, _)| {
        let end = start + needle.len();
        let before_ok = start == 0
            || !lower[..start]
                .chars()
                .last()
                .is_some_and(char::is_alphanumeric);
        let after_ok = end == lower.len()
            || !lower[end..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        before_ok && after_ok
    })
}

/// Find case-insensitive whole-word occurrences while retaining byte offsets
/// into the original source. Context and counter terms are lexical cues, so
/// `film` must not match inside `filmmaker` or `filmography`.
fn term_ranges(text: &str, term: &str) -> Vec<(usize, usize)> {
    if term.is_empty() {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    for (start, _) in text.char_indices() {
        let Some(end) = start.checked_add(term.len()) else {
            continue;
        };
        let Some(candidate) = text.get(start..end) else {
            continue;
        };
        if !candidate.eq_ignore_ascii_case(term) && candidate.to_lowercase() != term.to_lowercase()
        {
            continue;
        }
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if before.is_some_and(char::is_alphanumeric) || after.is_some_and(char::is_alphanumeric) {
            continue;
        }
        ranges.push((start, end));
    }
    ranges
}

/// Builds at most three ordered intents. Identity is always first for a named
/// entity; contextual intents use typed co-mentioned identities only. Generic
/// visual concepts get a single contextual B-roll intent and never an identity
/// intent, so they cannot enter the VLM identity gate.
fn build_intents(
    language: &str,
    identity: &CanonicalIdentity,
    status: LinkStatus,
    canonical_name: &str,
    identities: &[CanonicalIdentity],
) -> Vec<SearchIntent> {
    let mut intents = Vec::new();
    let role = role_of(&identity.kind);

    if role == EntityRole::NamedEntity {
        let mut query = if status == LinkStatus::Linked {
            canonical_name.to_string()
        } else {
            identity.canonical_surface.clone()
        };
        if status == LinkStatus::Linked
            && is_acronym(&identity.canonical_surface)
            && !query
                .to_lowercase()
                .contains(&identity.canonical_surface.to_lowercase())
        {
            query.push(' ');
            query.push_str(&identity.canonical_surface);
        }
        if clean_query(&query, 5) {
            intents.push(SearchIntent {
                kind: "identity".to_string(),
                query,
                lang: language.to_string(),
            });
        }
    }

    // A named entity without a usable identity query must not publish
    // contextual-only intents: the consumer contract requires the first
    // intent of a named entity to be its identity anchor, and a long or
    // punctuated surface ("Corrado Augias, giornalista") can be rejected by
    // clean_query below. No anchor means no intents at all.
    if role == EntityRole::NamedEntity && intents.is_empty() {
        return intents;
    }

    let mut co_mentions = BTreeSet::new();
    for mention in &identity.mentions {
        for other in identities {
            if other.id == identity.id || role_of(&other.kind) != EntityRole::NamedEntity {
                continue;
            }
            if other
                .mentions
                .iter()
                .any(|other_mention| other_mention.scene_id == mention.scene_id)
            {
                co_mentions.insert(other.canonical_surface.clone());
            }
        }
    }
    for surface in co_mentions {
        if intents.len() >= 3 {
            break;
        }
        let query = format!("{} {}", identity.canonical_surface, surface);
        if clean_query(&query, 6) {
            intents.push(SearchIntent {
                kind: "contextual".to_string(),
                query,
                lang: language.to_string(),
            });
        }
    }

    if role == EntityRole::VisualConcept && intents.is_empty() {
        let query = identity.canonical_surface.clone();
        if clean_query(&query, 6) {
            intents.push(SearchIntent {
                kind: "contextual".to_string(),
                query,
                lang: language.to_string(),
            });
        }
    }

    intents.truncate(3);
    intents
}

/// Returns the sentence that contains `[start, end)`. Terminators are ASCII,
/// so slicing at their byte boundaries is UTF-8 safe. Abbreviations inside the
/// mention (the `B.` of `Michael B. Jordan`) are behind `end` and cannot split
/// the window.
fn sentence_window(text: &str, start: usize, end: usize) -> &str {
    let (start, end) = sentence_window_bounds(text, start, end);
    text.get(start..end).unwrap_or("")
}

fn sentence_window_bounds(text: &str, start: usize, end: usize) -> (usize, usize) {
    let bytes = text.as_bytes();
    let start = start.min(bytes.len());
    let end = end.min(bytes.len());
    let sentence_start = (0..start)
        .rev()
        .find(|index| matches!(bytes[*index], b'.' | b'!' | b'?' | b'\n'))
        .map(|index| index + 1)
        .unwrap_or(0);
    let sentence_end = (end..bytes.len())
        .find(|index| matches!(bytes[*index], b'.' | b'!' | b'?' | b'\n'))
        .map(|index| index + 1)
        .unwrap_or(bytes.len());
    (sentence_start, sentence_end)
}

fn is_acronym(surface: &str) -> bool {
    let letters: Vec<char> = surface
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    letters.len() >= 2 && letters.iter().all(|character| character.is_uppercase())
}

/// Enforces the query-shape limits in the intent builder itself: no sentence
/// punctuation, no newlines, and a hard word cap.
fn clean_query(query: &str, max_words: usize) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed.split_whitespace().count() > max_words {
        return false;
    }
    !trimmed.chars().any(|character| {
        matches!(
            character,
            '.' | '?' | '!' | ',' | ';' | ':' | '\n' | '\r' | '"' | '“' | '”'
        )
    })
}

#[cfg(test)]
#[path = "entity_linker_tests.rs"]
mod tests;
