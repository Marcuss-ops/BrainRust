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

use crate::identity::{
    editorial_decision, normalize_surface, role_of, stable_surface_key, CanonicalIdentity,
    EntityRole, LinkStatus, LinkedEntity, SearchIntent,
};

/// Version recorded in analyzer provenance and health output.
pub const ENTITY_INDEX_SCHEMA: &str = "entity_index.v1";

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
    let index = entity_index();
    identities
        .iter()
        .map(|identity| {
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
                .map(|entry| (context_score(entry, &identity.kind, &context), entry))
                .collect();
            scored.sort_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| left.1.id.cmp(&right.1.id))
            });

            let (status, entry) = match scored.as_slice() {
                [] => (LinkStatus::Unlinked, None),
                [(score, entry)] => {
                    if *score < 0 {
                        (LinkStatus::Ambiguous, None)
                    } else {
                        (LinkStatus::Linked, Some(*entry))
                    }
                }
                [(best, entry), (second, _), ..] => {
                    // A type prior alone must not break a homonym tie: with
                    // two or more candidates, at least one real context term
                    // (a score of 3) is required to choose.
                    if best > second && *best >= 3 {
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
                editorial_surface: decision.surface,
                editorial_type: decision.kind,
                prompt_echo: decision.prompt_echo,
                visual_admission: decision.visual_admission,
            }
        })
        .collect()
}

/// Deterministic evidence score. Context terms add support, counter terms
/// remove it, a spelled-out canonical name is the strongest single signal, and
/// a matching type is a small prior. A tie is always ambiguous.
fn context_score(entry: &EntityIndexEntry, kind: &str, context: &str) -> i32 {
    let context = context.to_lowercase();
    let mut score = 0;
    for term in &entry.context_terms {
        if !term.is_empty() && context.contains(&term.to_lowercase()) {
            score += 3;
        }
    }
    for term in &entry.counter_terms {
        if !term.is_empty() && context.contains(&term.to_lowercase()) {
            score -= 4;
        }
    }
    if entry.entity_type.eq_ignore_ascii_case(kind)
        || (entry.entity_type == "ORG" && kind == "ORGANIZATION")
    {
        score += 1;
    }
    score
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
    text.get(sentence_start..sentence_end).unwrap_or("")
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
