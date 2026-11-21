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

/// Links identities with an additive prior only when a resolved discourse
/// thread provides an unambiguous sense from another mention in that thread.
pub fn link_all_with_threads(
    language: &str,
    scenes: &[(&str, &str)],
    identities: &[CanonicalIdentity],
    threads: &[crate::narrative::EntityThread],
) -> Vec<LinkedEntity> {
    let index = entity_index();
    identities
        .iter()
        .map(|identity| {
            let discourse_prior = resolved_thread_sense(language, identity, scenes, threads, index);
            // Context is the sentence around each grounded mention, never the
            // whole script: an actor mentioned in the next sentence must not
            // penalize the basketball player's identity. It is used only for
            // deterministic disambiguation and never copied into a query.
            let mut context = String::new();
            for mention in &identity.mentions {
                if let Some((_, text)) = scenes
                    .iter()
                    .find(|(scene_id, _)| *scene_id == mention.scene_id)
                {
                    context.push_str(sentence_window(text, mention.start_byte, mention.end_byte));
                    context.push(' ');
                }
            }
            let mut scored: Vec<(i32, &EntityIndexEntry)> = index
                .candidates(&identity.canonical_surface)
                .into_iter()
                .map(|entry| {
                    let prior = discourse_prior
                        .as_ref()
                        .is_some_and(|prior| prior.candidate_id == entry.id);
                    (
                        context_score(language, entry, &identity.kind, &context)
                            + if prior {
                                link_weights().discourse_prior_weight
                            } else {
                                0
                            },
                        entry,
                    )
                })
                .collect();
            scored.sort_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| left.1.id.cmp(&right.1.id))
            });

            let link_candidates: Vec<LinkCandidate> = scored
                .iter()
                .take(link_weights().max_candidates)
                .map(|(score, entry)| {
                    let prior = discourse_prior
                        .as_ref()
                        .filter(|prior| prior.candidate_id == entry.id);
                    explain_candidate(*score, entry, language, identity, scenes, prior)
                })
                .collect();
            let link_margin = scored
                .get(1)
                .map(|second| scored[0].0.saturating_sub(second.0));

            let (status, entry) = match scored.as_slice() {
                [] => (LinkStatus::Unlinked, None),
                [(score, entry)] => {
                    if *score < link_weights().single_candidate_min_score {
                        (LinkStatus::Ambiguous, None)
                    } else {
                        (LinkStatus::Linked, Some(*entry))
                    }
                }
                [(best, entry), (second, _), ..] => {
                    // A type prior alone must not break a homonym tie: with
                    // two or more candidates, at least one real context term
                    // (a score of 3) is required to choose.
                    if best - second >= link_weights().multi_candidate_min_margin
                        && *best >= link_weights().multi_candidate_min_score
                    {
                        (LinkStatus::Linked, Some(*entry))
                    } else {
                        (LinkStatus::Ambiguous, None)
                    }
                }
            };

            let linked = status == LinkStatus::Linked;
            let entity_type = if linked {
                entry
                    .map(|entry| entry.entity_type.clone())
                    .unwrap_or_else(|| identity.kind.clone())
            } else {
                identity.kind.clone()
            };
            let decision = editorial_decision(&identity.canonical_surface, &entity_type);
            let role = role_of(&decision.kind);
            let canonical_name = if linked {
                entry
                    .map(|entry| entry.canonical_name.clone())
                    .unwrap_or_else(|| identity.canonical_surface.clone())
            } else {
                identity.canonical_surface.clone()
            };
            let search_intents =
                build_intents(language, identity, status, &canonical_name, identities);

            LinkedEntity {
                local_id: identity.id.clone(),
                surface_key: stable_surface_key(&identity.kind, &identity.canonical_surface),
                canonical_surface: identity.canonical_surface.clone(),
                canonical_id: linked
                    .then(|| entry.map(|entry| entry.id.clone()))
                    .flatten(),
                link_status: status,
                canonical_name,
                entity_type,
                role,
                disambiguation_gloss: linked
                    .then(|| entry.map(|entry| entry.gloss.clone()))
                    .flatten(),
                negative_constraints: if linked {
                    entry
                        .map(|entry| entry.negative_constraints.clone())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                },
                mentions: identity.mentions.clone(),
                search_intents,
                link_candidates,
                link_margin,
                link_rule: Some("entity_linker.v3".to_string()),
                editorial_surface: decision.surface,
                editorial_type: decision.kind,
                prompt_echo: decision.prompt_echo,
                visual_admission: decision.visual_admission,
            }
        })
        .collect()
}

struct DiscourseSense {
    candidate_id: String,
    terms: Vec<String>,
    spans: Vec<crate::identity::LinkEvidenceSpan>,
}

fn resolved_thread_sense(
    language: &str,
    identity: &CanonicalIdentity,
    scenes: &[(&str, &str)],
    threads: &[crate::narrative::EntityThread],
    index: &EntityIndex,
) -> Option<DiscourseSense> {
    let thread = threads.iter().find(|thread| {
        thread.entity_id == identity.id
            && thread.entity_type.eq_ignore_ascii_case(&identity.kind)
            && thread.scene_ids.len() > 1
    })?;
    let candidates = index.candidates(&identity.canonical_surface);
    let mut resolved = BTreeSet::new();
    let mut evidence = BTreeMap::<String, Vec<crate::identity::LinkEvidenceSpan>>::new();
    for mention in identity
        .mentions
        .iter()
        .filter(|mention| thread.scene_ids.contains(&mention.scene_id))
    {
        let Some((_, text)) = scenes
            .iter()
            .find(|(scene_id, _)| *scene_id == mention.scene_id)
        else {
            continue;
        };
        let context = sentence_window(text, mention.start_byte, mention.end_byte);
        let mut local: Vec<(&EntityIndexEntry, i32)> = index
            .candidates(&mention.surface)
            .into_iter()
            .map(|entry| {
                (
                    entry,
                    context_score(language, entry, &identity.kind, context),
                )
            })
            .collect();
        local.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.id.cmp(&right.0.id))
        });
        let Some((winner, best)) = local.first().copied() else {
            continue;
        };
        let margin = local
            .get(1)
            .map(|second| best.saturating_sub(second.1))
            .unwrap_or(best);
        if best < link_weights().multi_candidate_min_score
            || margin < link_weights().multi_candidate_min_margin
            || !candidates.iter().any(|candidate| candidate.id == winner.id)
        {
            continue;
        }
        resolved.insert(winner.id.clone());
        let terms = winner
            .context_terms
            .iter()
            .filter(|term| contains_term(context, term))
            .cloned()
            .collect::<Vec<_>>();
        for term in terms {
            evidence
                .entry(winner.id.clone())
                .or_default()
                .extend(mention_evidence_spans(&term, mention, scenes));
        }
    }
    if resolved.len() != 1 {
        return None;
    }
    let candidate_id = resolved.into_iter().next()?;
    let mut spans = evidence.remove(&candidate_id).unwrap_or_default();
    spans.sort();
    spans.dedup();
    let terms = spans
        .iter()
        .filter_map(|span| {
            scenes
                .iter()
                .find(|(scene_id, _)| *scene_id == span.scene_id)
                .and_then(|(_, text)| text.get(span.start_byte..span.end_byte))
                .map(str::to_string)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    (!spans.is_empty()).then_some(DiscourseSense {
        candidate_id,
        terms,
        spans,
    })
}

fn mention_evidence_spans(
    term: &str,
    mention: &crate::identity::Mention,
    scenes: &[(&str, &str)],
) -> Vec<crate::identity::LinkEvidenceSpan> {
    let Some((_, text)) = scenes
        .iter()
        .find(|(scene_id, _)| *scene_id == mention.scene_id)
    else {
        return Vec::new();
    };
    let (window_start, window_end) =
        sentence_window_bounds(text, mention.start_byte, mention.end_byte);
    let Some(window) = text.get(window_start..window_end) else {
        return Vec::new();
    };
    term_ranges(window, term)
        .into_iter()
        .map(|(start, end)| crate::identity::LinkEvidenceSpan {
            scene_id: mention.scene_id.clone(),
            start_byte: window_start + start,
            end_byte: window_start + end,
        })
        .collect()
}

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
mod tests {
    use super::*;
    use crate::identity::Mention;

    fn identity(id: &str, surface: &str, kind: &str, scene: &str) -> CanonicalIdentity {
        CanonicalIdentity {
            id: id.to_string(),
            canonical_id: format!("org:{}", surface.to_lowercase()),
            canonical_surface: surface.to_string(),
            kind: kind.to_string(),
            mentions: vec![Mention {
                scene_id: scene.to_string(),
                surface: surface.to_string(),
                start_byte: 0,
                end_byte: surface.len(),
            }],
        }
    }

    fn link_one(text: &str, surface: &str, kind: &str) -> LinkedEntity {
        let identities = vec![identity("org_01", surface, kind, "s1")];
        link_all("it", &[("s1", text)], &identities)
            .pop()
            .expect("one linked entity")
    }

    #[test]
    fn context_and_counter_terms_match_whole_words_and_ground_the_exact_surface() {
        let entry = EntityIndexEntry {
            id: "test:film".into(),
            canonical_name: "Film".into(),
            entity_type: "OBJECT".into(),
            gloss: String::new(),
            negative_constraints: Vec::new(),
            aliases: BTreeMap::new(),
            context_terms: vec!["film".into()],
            counter_terms: vec!["city".into()],
        };
        assert_eq!(context_score("en", &entry, "PERSON", "filmmaker"), 0);
        assert_eq!(context_score("en", &entry, "PERSON", "film"), 3);
        assert_eq!(context_score("en", &entry, "PERSON", "cityscape"), 0);
        assert_eq!(context_score("en", &entry, "PERSON", "city"), -4);
        assert_eq!(
            term_ranges("filmmaker", "film"),
            Vec::<(usize, usize)>::new()
        );
        assert_eq!(term_ranges("at the Film festival", "film"), vec![(7, 11)]);
    }

    #[test]
    fn another_language_alias_matches_the_same_entry() {
        // The index matches normalized aliases across languages, so the same
        // KB id is returned regardless of the mention language.
        let it = link_one(
            "L'Agenzia Spaziale Europea studia Marte.",
            "Agenzia Spaziale Europea",
            "ORG",
        );
        assert_eq!(it.canonical_id.as_deref(), Some("wd:Q42262"));
    }

    #[test]
    fn ambiguous_alias_without_context_is_not_linked() {
        let nike = link_one("Nike ha annunciato una nuova iniziativa.", "Nike", "BRAND");
        assert_eq!(nike.link_status, LinkStatus::Ambiguous);
        assert!(nike.canonical_id.is_none());
    }

    #[test]
    fn context_resolves_nike_to_sportswear() {
        let nike = link_one(
            "Nike ha presentato le nuove scarpe da running per gli atleti.",
            "Nike",
            "BRAND",
        );
        assert_eq!(nike.link_status, LinkStatus::Linked);
        assert_eq!(nike.canonical_id.as_deref(), Some("wd:Q483859"));
        assert!(!nike.negative_constraints.is_empty());
    }

    #[test]
    fn context_resolves_esa_away_from_video_games() {
        let esa = link_one(
            "ESA e NASA studiano Marte con nuove missioni spaziali.",
            "ESA",
            "ORG",
        );
        assert_eq!(esa.canonical_id.as_deref(), Some("wd:Q42262"));
        assert!(esa
            .negative_constraints
            .iter()
            .any(|constraint| constraint.contains("video game")));
    }

    #[test]
    fn discourse_prior_carries_a_resolved_sense_only_within_its_thread() {
        let first = "Mercury has an orbit around the sun.";
        let second = "Mercury was mentioned again.";
        let identity = CanonicalIdentity {
            id: "loc_mercury".into(),
            canonical_id: "location:mercury".into(),
            canonical_surface: "Mercury".into(),
            kind: "LOCATION".into(),
            mentions: vec![
                Mention {
                    scene_id: "s1".into(),
                    surface: "Mercury".into(),
                    start_byte: 0,
                    end_byte: 7,
                },
                Mention {
                    scene_id: "s2".into(),
                    surface: "Mercury".into(),
                    start_byte: 0,
                    end_byte: 7,
                },
            ],
        };
        let scenes = [("s1", first), ("s2", second)];
        let thread = crate::narrative::EntityThread {
            entity_id: "loc_mercury".into(),
            canonical_surface: "Mercury".into(),
            entity_type: "LOCATION".into(),
            scene_ids: vec!["s1".into(), "s2".into()],
            first_scene_id: "s1".into(),
            last_scene_id: "s2".into(),
            mention_count: 2,
            return_count: 0,
            beats: Vec::new(),
        };
        let linked = link_all_with_threads("en", &scenes, &[identity.clone()], &[thread.clone()])
            .pop()
            .unwrap();
        assert_eq!(linked.canonical_id.as_deref(), Some("wd:Q308"));
        let prior = linked.link_candidates[0]
            .features
            .iter()
            .find(|feature| feature.name == "discourse_prior")
            .expect("winning sense should expose its discourse prior");
        assert_eq!(prior.value, link_weights().discourse_prior_weight);
        assert_eq!(prior.terms, ["orbit"]);
        assert_eq!(prior.spans.len(), 1);
        assert_eq!(
            &first[prior.spans[0].start_byte..prior.spans[0].end_byte],
            "orbit"
        );

        let wrong_thread = crate::narrative::EntityThread {
            entity_id: "another_entity".into(),
            ..thread
        };
        let without_prior =
            link_all_with_threads("en", &scenes, &[identity.clone()], &[wrong_thread])
                .pop()
                .unwrap();
        assert!(without_prior.link_candidates[0]
            .features
            .iter()
            .all(|feature| feature.name != "discourse_prior"));

        let wrong_type = crate::narrative::EntityThread {
            entity_type: "PERSON".into(),
            ..crate::narrative::EntityThread {
                entity_id: "loc_mercury".into(),
                canonical_surface: "Mercury".into(),
                entity_type: "LOCATION".into(),
                scene_ids: vec!["s1".into(), "s2".into()],
                first_scene_id: "s1".into(),
                last_scene_id: "s2".into(),
                mention_count: 2,
                return_count: 0,
                beats: Vec::new(),
            }
        };
        let without_cross_type_prior =
            link_all_with_threads("en", &scenes, &[identity], &[wrong_type])
                .pop()
                .unwrap();
        assert!(without_cross_type_prior.link_candidates[0]
            .features
            .iter()
            .all(|feature| feature.name != "discourse_prior"));
    }

    #[test]
    fn linked_entities_expose_bounded_candidates_and_byte_exact_evidence() {
        let text = "ESA e NASA studiano Marte con nuove missioni spaziali.";
        let entity = link_one(text, "ESA", "ORG");
        assert_eq!(entity.canonical_id.as_deref(), Some("wd:Q42262"));
        assert_eq!(entity.link_rule.as_deref(), Some("entity_linker.v3"));
        assert_eq!(entity.link_candidates.len(), 2);
        assert_eq!(entity.link_candidates[0].kb_id, "wd:Q42262");
        assert!(entity.link_margin.is_some_and(|margin| margin > 0));
        assert!(entity
            .link_candidates
            .iter()
            .all(|candidate| candidate.features.len() <= 6));
        assert!(entity.link_candidates[0]
            .features
            .iter()
            .any(|feature| feature.name == "context_overlap" && !feature.spans.is_empty()));
        for candidate in &entity.link_candidates {
            for feature in &candidate.features {
                for span in &feature.spans {
                    let evidence = text
                        .get(span.start_byte..span.end_byte)
                        .expect("evidence span is UTF-8 aligned");
                    assert!(feature
                        .terms
                        .iter()
                        .any(|term| evidence.eq_ignore_ascii_case(term)));
                }
            }
        }
    }

    #[test]
    fn entity_type_cues_are_localized_and_keep_exact_evidence_spans() {
        let text = "ESA è un'agenzia spaziale.";
        let identity = identity("org_01", "ESA", "ORG", "s1");
        let terms = matching_type_cues("it", "ORG", text);
        assert!(terms.iter().any(|term| term == "agenzia"));
        let spans = evidence_spans("agenzia", &identity, &[("s1", text)]);
        assert_eq!(spans.len(), 1);
        assert_eq!(&text[spans[0].start_byte..spans[0].end_byte], "agenzia");
        assert!(matching_type_cues("en", "ORG", "ESA è un'agenzia spaziale.").is_empty());
    }

    #[test]
    fn every_confusable_index_sense_has_three_positive_and_negative_contexts() {
        #[derive(Deserialize)]
        struct FixtureFile {
            schema_version: String,
            surfaces: Vec<SurfaceFixture>,
        }
        #[derive(Deserialize)]
        struct SurfaceFixture {
            surface: String,
            senses: Vec<SenseFixture>,
        }
        #[derive(Deserialize)]
        struct SenseFixture {
            id: String,
            entity_type: String,
            positive_contexts: Vec<String>,
            negative_contexts: Vec<NegativeFixture>,
        }
        #[derive(Deserialize)]
        struct NegativeFixture {
            context: String,
            expected_id: String,
        }

        let fixtures: FixtureFile = serde_json::from_str(include_str!(
            "../../lexicon/entity_index_confusables_v1.json"
        ))
        .expect("confusable index fixtures must be valid JSON");
        assert_eq!(fixtures.schema_version, "entity_index_confusables.v1");
        let mut positive_cases = 0;
        let mut negative_cases = 0;

        for surface in &fixtures.surfaces {
            assert!(surface.senses.len() >= 2 && surface.senses.len() <= 3);
            for sense in &surface.senses {
                assert_eq!(
                    sense.positive_contexts.len(),
                    3,
                    "{} / {}",
                    surface.surface,
                    sense.id
                );
                assert_eq!(
                    sense.negative_contexts.len(),
                    3,
                    "{} / {}",
                    surface.surface,
                    sense.id
                );
                for context in &sense.positive_contexts {
                    let linked = link_one(context, &surface.surface, &sense.entity_type);
                    assert_eq!(
                        linked.canonical_id.as_deref(),
                        Some(sense.id.as_str()),
                        "{context}"
                    );
                    positive_cases += 1;
                }
                for case in &sense.negative_contexts {
                    let linked = link_one(&case.context, &surface.surface, &sense.entity_type);
                    assert_eq!(
                        linked.canonical_id.as_deref(),
                        Some(case.expected_id.as_str()),
                        "{}",
                        case.context
                    );
                    negative_cases += 1;
                }
            }
            let first = &surface.senses[0];
            let linked = link_one(&surface.surface, &surface.surface, &first.entity_type);
            assert_eq!(
                linked.link_status,
                LinkStatus::Ambiguous,
                "{} without context",
                surface.surface
            );
        }

        assert!(fixtures.surfaces.len() >= 40);
        assert!(
            positive_cases >= 240,
            "only {positive_cases} positive contexts tested"
        );
        assert!(
            negative_cases >= 240,
            "only {negative_cases} negative contexts tested"
        );
    }

    #[test]
    fn intents_are_identity_first_and_bounded() {
        let jordan = link_one(
            "Michael Jordan ha parlato con la NASA durante una missione.",
            "Michael Jordan",
            "PERSON",
        );
        assert_eq!(jordan.search_intents[0].kind, "identity");
        assert!(jordan.search_intents.len() <= 3);
        assert!(jordan
            .search_intents
            .iter()
            .all(|intent| !intent.query.contains('.')));
    }

    #[test]
    fn contextual_intents_are_anchored_to_the_owner_identity() {
        let owner = CanonicalIdentity {
            id: "person_augias".into(),
            canonical_id: "person:augias".into(),
            canonical_surface: "Corrado Augias".into(),
            kind: "PERSON".into(),
            mentions: vec![Mention {
                scene_id: "s1".into(),
                surface: "Corrado Augias".into(),
                start_byte: 0,
                end_byte: 14,
            }],
        };
        let place = CanonicalIdentity {
            id: "loc_paris".into(),
            canonical_id: "location:paris".into(),
            canonical_surface: "Paris".into(),
            kind: "LOCATION".into(),
            mentions: vec![Mention {
                scene_id: "s1".into(),
                surface: "Paris".into(),
                start_byte: 20,
                end_byte: 25,
            }],
        };
        let intents = build_intents(
            "it",
            &owner,
            LinkStatus::Linked,
            "Corrado Augias",
            &[owner.clone(), place],
        );
        assert!(intents.iter().any(|intent| intent.kind == "contextual"));
        assert!(intents
            .iter()
            .filter(|intent| intent.kind == "contextual")
            .all(|intent| intent.query.contains("Corrado Augias")));
    }

    #[test]
    fn a_visual_concept_never_gets_an_identity_intent() {
        let concept = link_one(
            "le scarpe da running",
            "scarpe da running",
            "VISUAL_CONCEPT",
        );
        assert!(concept
            .search_intents
            .iter()
            .all(|intent| intent.kind != "identity"));
        assert_eq!(concept.role, EntityRole::VisualConcept);
    }
}
