//! Conservative, clause-local event frames grounded in typed triggers and
//! already-grounded participants, dates, and place mentions.

use std::collections::{BTreeMap, HashMap, HashSet};

use semantic_core::{
    Candidate, EventEvidence, EventFrame, EventParticipant, EventTime, Place, SentenceSpan,
    TemporalFact,
};
use serde::Deserialize;

const EN: &str = include_str!("../../lexicon/events/en.json");
const IT: &str = include_str!("../../lexicon/events/it.json");
const ES: &str = include_str!("../../lexicon/events/es.json");
const FR: &str = include_str!("../../lexicon/events/fr.json");
const PT: &str = include_str!("../../lexicon/events/pt.json");
const DE: &str = include_str!("../../lexicon/events/de.json");
const NL: &str = include_str!("../../lexicon/events/nl.json");
const ID: &str = include_str!("../../lexicon/events/id.json");
const PL: &str = include_str!("../../lexicon/events/pl.json");
const RU: &str = include_str!("../../lexicon/events/ru.json");
const TR: &str = include_str!("../../lexicon/events/tr.json");
pub const MAX_EVENT_FRAMES: usize = 200;

pub(crate) struct SceneEventInput<'a> {
    pub scene_index: usize,
    pub scene_id: &'a str,
    pub source: &'a str,
    pub language: &'a str,
    pub candidates: &'a [Candidate],
    pub temporal_facts: &'a [TemporalFact],
    pub places: &'a [Place],
}

#[derive(Deserialize)]
struct TriggerFile {
    language: String,
    event_types: BTreeMap<String, Vec<TriggerEntry>>,
}

#[derive(Deserialize)]
struct TriggerEntry {
    #[serde(default)]
    surface: Option<String>,
    lemma: String,
    voice: String,
    roles: BTreeMap<String, String>,
    #[serde(default)]
    pro_drop_roles: Vec<String>,
}

fn lexicon(language: &str) -> Option<TriggerFile> {
    let base = language.split('-').next().unwrap_or(language);
    let data = match base {
        "en" => EN,
        "it" => IT,
        "es" => ES,
        "fr" => FR,
        "pt" => PT,
        "de" => DE,
        "nl" => NL,
        "id" => ID,
        "pl" => PL,
        "ru" => RU,
        "tr" => TR,
        _ => return None,
    };
    let file: TriggerFile = serde_json::from_str(data).ok()?;
    (file.language == base).then_some(file)
}

fn whole_surface_spans(text: &str, surface: &str) -> Vec<(usize, usize)> {
    let lower = text.to_lowercase();
    let needle = surface.to_lowercase();
    lower
        .match_indices(&needle)
        .filter_map(|(start, matched)| {
            let end = start + matched.len();
            let before = lower[..start].chars().next_back();
            let after = lower[end..].chars().next();
            let boundary = |ch: Option<char>| !ch.is_some_and(char::is_alphanumeric);
            (boundary(before) && boundary(after)).then_some((start, end))
        })
        .collect()
}

fn passive_context(sentence: &str, trigger_start: usize, language: &str) -> bool {
    let prefix = sentence.get(..trigger_start).unwrap_or("").to_lowercase();
    let markers: &[&str] = match language {
        "it" => &[
            "è stato ",
            "è stata ",
            "sono stati ",
            "sono state ",
            "fu ",
            "venne ",
        ],
        "es" => &["fue ", "fueron ", "ha sido ", "han sido "],
        "fr" => &[
            "a été ",
            "ont été ",
            "avait été ",
            "avaient été ",
            "fut ",
            "furent ",
        ],
        "pt" => &["foi ", "foram ", "tinha sido ", "tinham sido "],
        "de" => &["wurde ", "wurden ", "ist ", "sind ", "war ", "waren "],
        "nl" => &["werd ", "werden ", "is ", "zijn ", "was ", "waren "],
        "pl" => &["został ", "została ", "zostali ", "zostały "],
        "ru" => &["был ", "была ", "были ", "было "],
        "tr" => &["tarafından ", "oldu ", "edildi "],
        _ => &[
            "was ",
            "were ",
            "is ",
            "are ",
            "been ",
            "has been ",
            "had been ",
        ],
    };
    markers.iter().any(|marker| prefix.ends_with(marker))
}

fn active_perfect_context(sentence: &str, trigger_start: usize, language: &str) -> bool {
    let prefix = sentence.get(..trigger_start).unwrap_or("").to_lowercase();
    let markers: &[&str] = match language {
        "it" => &[
            "ho ", "hai ", "ha ", "abbiamo ", "avete ", "hanno ", "aveva ", "avevano ", "abbia ",
            "abbiano ",
        ],
        "es" => &["he ", "has ", "ha ", "hemos ", "han ", "había ", "habían "],
        "fr" => &["ai ", "as ", "a ", "avons ", "ont ", "avait ", "avaient "],
        "pt" => &["tenho ", "tem ", "temos ", "têm ", "havia ", "haviam "],
        "de" => &["hat ", "haben ", "habe ", "hatte ", "hatten "],
        "nl" => &["heeft ", "hebben ", "heb ", "had ", "hadden "],
        _ => &["has ", "have ", "had "],
    };
    markers.iter().any(|marker| prefix.ends_with(marker))
}

fn nearest_candidate<'a>(
    candidates: &'a [Candidate],
    tokens: &[visualner::AnalyzedToken],
    start: usize,
    end: usize,
    sentence: &SentenceSpan,
    position: &str,
    allow_work_subject: bool,
) -> Option<&'a Candidate> {
    let trigger_token = tokens
        .iter()
        .find(|token| token.start == start && token.end == end);
    if let Some(trigger_id) = trigger_token.and_then(|token| token.id) {
        let expected = match position {
            "subject" => &["nsubj", "csubj"][..],
            "object" => &["obj", "iobj"][..],
            "agent" => &["obl:agent", "nmod:agent", "agent"][..],
            _ => &[][..],
        };
        let syntactic: Vec<&Candidate> = candidates
            .iter()
            .filter(|candidate| {
                candidate.start_byte >= sentence.start_byte
                    && candidate.end_byte <= sentence.end_byte
                    && (matches!(
                        candidate.kind.as_str(),
                        "PERSON" | "ORG" | "ORGANIZATION" | "FACILITY" | "LOCATION" | "VEHICLE"
                    ) || (matches!(candidate.kind.as_str(), "WORK" | "OBJECT" | "PRODUCT")
                        && (position == "object" || (position == "subject" && allow_work_subject))))
            })
            .filter(|candidate| {
                tokens.iter().any(|token| {
                    let candidate_token = token.start >= candidate.start_byte
                        && token.end <= candidate.end_byte
                        && token.sentence_id
                            == trigger_token.and_then(|trigger| trigger.sentence_id);
                    let dependency_role = token.head == Some(trigger_id)
                        && token.dependency.as_deref().is_some_and(|relation| {
                            expected.iter().any(|prefix| {
                                relation == *prefix || relation.starts_with(&format!("{prefix}:"))
                            })
                        });
                    let participial_antecedent = position == "subject"
                        && trigger_token.is_some_and(|trigger| {
                            trigger.head == token.id
                                && trigger.features.as_deref().is_some_and(|features| {
                                    features
                                        .split('|')
                                        .any(|feature| feature == "VerbForm=Part")
                                })
                                && trigger.dependency.as_deref().is_some_and(|relation| {
                                    relation == "acl" || relation.starts_with("acl:")
                                })
                        });
                    candidate_token && (dependency_role || participial_antecedent)
                })
            })
            .collect();
        if syntactic.len() == 1 {
            return syntactic.first().copied();
        }
        if position == "object" {
            let work_objects: Vec<&Candidate> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.kind == "WORK"
                        && candidate.start_byte >= end
                        && candidate.start_byte >= sentence.start_byte
                        && candidate.end_byte <= sentence.end_byte
                })
                .collect();
            if work_objects.len() == 1 {
                return work_objects.first().copied();
            }
        }
        if position == "subject" && allow_work_subject {
            let work_subjects: Vec<&Candidate> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.kind == "WORK"
                        && candidate.end_byte <= start
                        && candidate.start_byte >= sentence.start_byte
                        && candidate.end_byte <= sentence.end_byte
                })
                .collect();
            if work_subjects.len() == 1 {
                return work_subjects.first().copied();
            }
        }
        // A parsed sentence gives an explicit negative result; proximity
        // must not silently attach the nearest unrelated name. Heuristic
        // tokenizers may assign ids without dependencies, and those must not
        // discard the model-free proximity fallback.
        if tokens
            .iter()
            .any(|token| token.id.is_some() && token.dependency.is_some())
        {
            return None;
        }
    }
    let mut matches: Vec<(&Candidate, usize)> = candidates
        .iter()
        .filter(|candidate| {
            candidate.start_byte >= sentence.start_byte
                && candidate.end_byte <= sentence.end_byte
                && (candidate.end_byte <= start || candidate.start_byte >= end)
        })
        .filter(|candidate| {
            (matches!(
                candidate.kind.as_str(),
                "PERSON" | "ORG" | "ORGANIZATION" | "FACILITY" | "LOCATION" | "VEHICLE"
            ) || (matches!(candidate.kind.as_str(), "WORK" | "OBJECT" | "PRODUCT")
                && (position == "object" || (position == "subject" && allow_work_subject))))
        })
        .filter_map(|candidate| {
            let distance = match position {
                "subject" if candidate.end_byte <= start => start - candidate.end_byte,
                "object" if candidate.start_byte >= end => candidate.start_byte - end,
                // Fallback for nominal triggers without grammatical arguments:
                // bind only to an adjacent grounded participant.
                "near" if candidate.end_byte <= start => start - candidate.end_byte,
                "near" if candidate.start_byte >= end => candidate.start_byte - end,
                _ => return None,
            };
            // Sentence-local grounding already bounds the candidate. A fixed
            // 100-byte cutoff drops valid subjects in long biographical or
            // documentary sentences, so keep the nearest syntactic-position
            // candidate anywhere in that sentence.
            Some((candidate, distance))
        })
        .collect();
    matches.sort_by_key(|(candidate, distance)| (*distance, candidate.start_byte));
    if matches
        .get(1)
        .is_some_and(|(_, second)| *second == matches[0].1)
    {
        return None;
    }
    matches.first().map(|(candidate, _)| *candidate)
}

fn preceding_sentence_person(
    source: &str,
    candidates: &[Candidate],
    sentence_start: usize,
    language: &str,
) -> Option<String> {
    let mut context_start = sentence_start.saturating_sub(800);
    while !source.is_char_boundary(context_start) {
        context_start += 1;
    }
    let context = source.get(context_start..sentence_start)?;
    for span in semantic_core::split_sentences(context, language, None)
        .iter()
        .rev()
    {
        let start = context_start + span.start_byte;
        let end = context_start + span.end_byte;
        let people: std::collections::BTreeSet<&str> = candidates
            .iter()
            .filter(|candidate| {
                candidate.kind == "PERSON"
                    && candidate.start_byte >= start
                    && candidate.end_byte <= end
            })
            .map(|candidate| candidate.id.as_str())
            .collect();
        if !people.is_empty() {
            return (people.len() == 1).then(|| people.into_iter().next().unwrap().to_string());
        }
    }
    None
}

/// The one participant already credited as the agent of the same event type
/// earlier in the scene, if the scene has exactly one such person.
///
/// An omitted subject is normally resolved from the immediately preceding
/// sentence, but that sentence is often about the subject's family ("Era
/// sposato con Daniela Pasti e aveva una figlia, Natalia") and names two
/// people. Publication history still identifies the author: a scene that has
/// already attributed one publication to exactly one person attributes the
/// next omitted-subject publication to that same person. Two distinct agents
/// in the scene stay ambiguous and resolve to nothing.
fn established_event_agent(
    frames: &[EventFrame],
    event_type: &str,
    scene_id: &str,
) -> Option<String> {
    let mut agents: Vec<&str> = frames
        .iter()
        .filter(|frame| {
            frame.event_type == event_type && frame.scene_ids.iter().any(|id| id == scene_id)
        })
        .flat_map(|frame| frame.participants.iter())
        .filter(|participant| participant.role == "agent")
        .map(|participant| participant.entity_id.as_str())
        .collect();
    agents.sort_unstable();
    agents.dedup();
    match agents.as_slice() {
        [only] => Some((*only).to_string()),
        _ => None,
    }
}

fn has_possessive_cue(sentence: &str, language: &str) -> bool {
    let Some(possessives) = semantic_core::language_profiles::get(language)
        .and_then(|profile| profile.word_sets.get("possessives"))
    else {
        return false;
    };
    semantic_core::discourse::word_spans(sentence)
        .iter()
        .any(|word| possessives.iter().any(|cue| cue == &word.lowered))
}

fn status(sentence: &str, language: &str) -> &'static str {
    let words: Vec<String> = semantic_core::discourse::word_spans(sentence)
        .into_iter()
        .map(|word| word.lowered)
        .collect();
    let negation: &[&str] = match language {
        "it" => &["non", "mai"],
        "es" => &["no", "nunca", "jamás", "nunca jamás"],
        "fr" => &["ne", "pas", "jamais"],
        "pt" => &["não", "nunca", "jamais"],
        "de" => &["nicht", "nie", "kein", "keine", "keinen", "keiner"],
        "nl" => &["niet", "nooit", "geen"],
        "id" => &["tidak", "bukan", "belum"],
        "pl" => &["nie", "nigdy"],
        "ru" => &["не", "никогда", "ни"],
        "tr" => &["değil", "hiç", "yok"],
        _ => &["not", "never", "no longer"],
    };
    if negation.iter().any(|cue| contains_word_phrase(&words, cue)) {
        "negated"
    } else if [
        "allegedly",
        "reportedly",
        "presunto",
        "presuntamente",
        "supuestamente",
        "forse",
        "angeblich",
        "mutmaßlich",
        "vermutlich",
        "vermoedelijk",
        "zou",
        "zouden",
        "naar verluidt",
        "présumé",
        "présumée",
        "supostamente",
        "alegadamente",
        "talvez",
        "teria",
        "aurait",
        "serait",
        "might",
        "would",
        "kabarnya",
        "diduga",
        "prawdopodobnie",
        "rzekomo",
        "якобы",
        "предположительно",
        "muhtemelen",
        "iddialara göre",
    ]
    .iter()
    .any(|cue| contains_word_phrase(&words, cue))
    {
        "uncertain"
    } else {
        "asserted"
    }
}

fn contains_word_phrase(words: &[String], phrase: &str) -> bool {
    let needle: Vec<String> = semantic_core::discourse::word_spans(phrase)
        .into_iter()
        .map(|word| word.lowered)
        .collect();
    !needle.is_empty()
        && words
            .windows(needle.len())
            .any(|window| window == needle.as_slice())
}

fn temporal_slot(
    temporal_facts: &[TemporalFact],
    sentence: &SentenceSpan,
    trigger_start: usize,
    trigger_end: usize,
) -> Option<(usize, EventTime)> {
    temporal_facts
        .iter()
        .enumerate()
        .filter(|(_, fact)| {
            fact.start_byte >= sentence.start_byte && fact.end_byte <= sentence.end_byte
        })
        .min_by_key(|(_, fact)| {
            if fact.end_byte <= trigger_start {
                trigger_start - fact.end_byte
            } else if fact.start_byte >= trigger_end {
                fact.start_byte.saturating_sub(trigger_end)
            } else {
                0
            }
        })
        .map(|(index, fact)| {
            (
                index,
                EventTime {
                    fact_index: index,
                    normalized_date: fact.normalized.clone(),
                    derived: fact.derived,
                },
            )
        })
}

#[cfg(test)]
fn extract(
    scene_index: usize,
    scene_id: &str,
    source: &str,
    language: &str,
    candidates: &[Candidate],
    temporal_facts: &[TemporalFact],
    places: &[Place],
) -> Vec<EventFrame> {
    extract_bounded(
        SceneEventInput {
            scene_index,
            scene_id,
            source,
            language,
            candidates,
            temporal_facts,
            places,
        },
        MAX_EVENT_FRAMES,
    )
}

#[cfg(test)]
pub(crate) fn extract_bounded(input: SceneEventInput<'_>, max_frames: usize) -> Vec<EventFrame> {
    extract_bounded_with_tokens(input, &[], max_frames)
}

/// Per-scene token lookups used by the trigger classifier. The former loop
/// rescanned every token for each trigger (participle, voice, dependency,
/// agent); these maps make one pass over the token list instead.
struct TokenIndex<'a> {
    by_start: HashMap<usize, &'a visualner::AnalyzedToken>,
    by_id: HashMap<i32, Vec<&'a visualner::AnalyzedToken>>,
    agent_heads: HashSet<(Option<i32>, i32)>,
}

impl<'a> TokenIndex<'a> {
    fn new(tokens: &'a [visualner::AnalyzedToken]) -> Self {
        let mut by_start = HashMap::with_capacity(tokens.len());
        let mut by_id: HashMap<i32, Vec<&'a visualner::AnalyzedToken>> = HashMap::new();
        let mut agent_heads = HashSet::new();
        for token in tokens {
            by_start.entry(token.start).or_insert(token);
            if let Some(id) = token.id {
                by_id.entry(id).or_default().push(token);
            }
            if token
                .dependency
                .as_deref()
                .is_some_and(|relation| matches!(relation, "obl:agent" | "nmod:agent" | "agent"))
            {
                if let Some(head) = token.head {
                    agent_heads.insert((token.sentence_id, head));
                }
            }
        }
        Self {
            by_start,
            by_id,
            agent_heads,
        }
    }

    fn at(&self, start: usize) -> Option<&'a visualner::AnalyzedToken> {
        self.by_start.get(&start).copied()
    }

    fn head_in_sentence(
        &self,
        id: i32,
        sentence: &SentenceSpan,
    ) -> Option<&'a visualner::AnalyzedToken> {
        self.by_id
            .get(&id)?
            .iter()
            .copied()
            .find(|token| token.start >= sentence.start_byte && token.end <= sentence.end_byte)
    }

    fn has_agent_child(&self, token: &visualner::AnalyzedToken) -> bool {
        token
            .id
            .is_some_and(|id| self.agent_heads.contains(&(token.sentence_id, id)))
    }
}

/// The trigger's morphosyntactic reading. Extracted from the loop body so the
/// six conditions are named and testable instead of nested `tokens.iter().any`
/// closures.
#[derive(Default)]
struct TriggerForm {
    attributive_participle: bool,
    participial_predicate: bool,
    modifier_like_trigger: bool,
    attributive_work: bool,
    passive_voice: bool,
    agent_child: bool,
}

impl TriggerForm {
    fn classify(
        token: Option<&visualner::AnalyzedToken>,
        index: &TokenIndex<'_>,
        candidates: &[Candidate],
        sentence: &SentenceSpan,
    ) -> Self {
        let Some(token) = token else {
            return Self::default();
        };
        let dependency = token.dependency.as_deref();
        let attributive_dependency = dependency.is_some_and(|relation| {
            relation == "amod" || relation == "acl" || relation.starts_with("acl:")
        });
        let participle = token.features.as_deref().is_some_and(|features| {
            features
                .split('|')
                .any(|feature| feature == "VerbForm=Part")
        });
        // Only a participle can modify a work attributively. The dependency
        // parser also attaches finite verbs to the noun they follow ("Nel 2024
        // pubblicò La vita s'impara" can come back as an advcl of the title),
        // and treating that as a passive participle hides the real active
        // agent.
        let attributive_work = participle
            && token
                .head
                .and_then(|head_id| index.head_in_sentence(head_id, sentence))
                .is_some_and(|head| {
                    candidates.iter().any(|candidate| {
                        candidate.kind == "WORK"
                            && candidate.start_byte <= head.start
                            && candidate.end_byte >= head.end
                    })
                });
        Self {
            attributive_participle: participle && attributive_dependency,
            participial_predicate: participle,
            modifier_like_trigger: token.upos.as_deref() == Some("ADJ") || attributive_dependency,
            attributive_work,
            passive_voice: token
                .features
                .as_deref()
                .is_some_and(|features| features.split('|').any(|feature| feature == "Voice=Pass"))
                || matches!(dependency, Some("nsubj:pass" | "csubj:pass")),
            agent_child: index.has_agent_child(token),
        }
    }
}

/// Uses the exact token/lemma annotations produced during VisualNER analysis.
/// Literal trigger surfaces remain the no-model fallback.
pub(crate) fn extract_bounded_with_tokens(
    input: SceneEventInput<'_>,
    tokens: &[visualner::AnalyzedToken],
    max_frames: usize,
) -> Vec<EventFrame> {
    let SceneEventInput {
        scene_index,
        scene_id,
        source,
        language,
        candidates,
        temporal_facts,
        places,
    } = input;
    if max_frames == 0 {
        return Vec::new();
    }
    let Some(file) = lexicon(language) else {
        return Vec::new();
    };
    let base_language = language.split('-').next().unwrap_or(language);
    let supports_pro_drop = semantic_core::lexicon::lexicon()
        .language_profiles
        .pro_drop_subjects(base_language);
    let sentences = semantic_core::split_sentences(source, language, None);
    let token_index = TokenIndex::new(tokens);
    let mut frames = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(sentence_text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        for (event_type, triggers) in &file.event_types {
            for trigger in triggers {
                for (local_start, local_end) in
                    trigger_spans(sentence_text, sentence.start_byte, trigger, tokens)
                {
                    let trigger_start = sentence.start_byte + local_start;
                    let trigger_end = sentence.start_byte + local_end;
                    let form = TriggerForm::classify(
                        token_index.at(trigger_start),
                        &token_index,
                        candidates,
                        sentence,
                    );
                    if form.attributive_participle && !form.attributive_work {
                        continue;
                    }
                    if (form.participial_predicate || form.modifier_like_trigger)
                        && !form.attributive_work
                        && event_type == "launch_release"
                        && !passive_context(sentence_text, local_start, base_language)
                        && !active_perfect_context(sentence_text, local_start, base_language)
                    {
                        continue;
                    }
                    let passive = form.attributive_work
                        || passive_context(sentence_text, local_start, base_language)
                        || form.passive_voice
                        || form.agent_child;
                    let duplicate_voice = triggers.iter().any(|other| {
                        let same_trigger = match (&other.surface, &trigger.surface) {
                            (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
                            _ => other.lemma.eq_ignore_ascii_case(&trigger.lemma),
                        };
                        same_trigger && other.voice != trigger.voice
                    });
                    if duplicate_voice && (trigger.voice == "passive") != passive {
                        continue;
                    }
                    let mut participants = Vec::new();
                    let mut pro_drop_resolved = false;
                    let mut discourse_agent_resolved = false;
                    for (role, position) in &trigger.roles {
                        let candidate = nearest_candidate(
                            candidates,
                            tokens,
                            trigger_start,
                            trigger_end,
                            sentence,
                            position,
                            *role == "patient" && trigger.voice == "passive",
                        );
                        if let Some(candidate) = candidate {
                            participants.push(EventParticipant {
                                role: role.clone(),
                                entity_id: candidate.id.clone(),
                            });
                        } else if position == "subject"
                            && trigger.pro_drop_roles.contains(role)
                            && supports_pro_drop
                            && (trigger.voice != "passive"
                                || role == "patient"
                                || (event_type == "launch_release"
                                    && role == "agent"
                                    && has_possessive_cue(sentence_text, base_language)))
                        {
                            // Resolve omitted subjects only from a unique PERSON
                            // mention in the immediately preceding sentence of
                            // the same paragraph. A document-wide frequency
                            // winner silently misattributes multi-person texts.
                            let local = preceding_sentence_person(
                                source,
                                candidates,
                                sentence.start_byte,
                                language,
                            );
                            if let Some(person_id) = local {
                                participants.push(EventParticipant {
                                    role: role.clone(),
                                    entity_id: person_id,
                                });
                                pro_drop_resolved = true;
                            } else if let Some(person_id) =
                                established_event_agent(&frames, event_type, scene_id)
                            {
                                participants.push(EventParticipant {
                                    role: role.clone(),
                                    entity_id: person_id,
                                });
                                discourse_agent_resolved = true;
                            }
                        }
                    }
                    participants
                        .sort_by(|a, b| a.role.cmp(&b.role).then(a.entity_id.cmp(&b.entity_id)));
                    participants.dedup_by(|a, b| a.role == b.role && a.entity_id == b.entity_id);

                    let temporal =
                        temporal_slot(temporal_facts, sentence, trigger_start, trigger_end);
                    let place = places
                        .iter()
                        .filter(|place| {
                            place.sentence_index == sentence_index
                                && place.role == "setting"
                                && place.start_byte >= sentence.start_byte
                                && place.end_byte <= sentence.end_byte
                        })
                        .min_by_key(|place| {
                            if place.end_byte <= trigger_start {
                                trigger_start - place.end_byte
                            } else if place.start_byte >= trigger_end {
                                place.start_byte.saturating_sub(trigger_end)
                            } else {
                                0
                            }
                        });
                    if participants.is_empty() && temporal.is_none() && place.is_none() {
                        continue;
                    }
                    let Some(trigger_surface) = source.get(trigger_start..trigger_end) else {
                        continue;
                    };
                    let Some(evidence_text) = source.get(sentence.start_byte..sentence.end_byte)
                    else {
                        continue;
                    };
                    let mut confidence_basis = vec!["trigger".to_string()];
                    if !participants.is_empty() {
                        confidence_basis.push("participant".to_string());
                    }
                    if pro_drop_resolved {
                        confidence_basis.push("pro_drop_coref".to_string());
                    }
                    if discourse_agent_resolved {
                        confidence_basis.push("discourse_agent_coref".to_string());
                    }
                    if temporal.is_some() {
                        confidence_basis.push("date".to_string());
                    }
                    if place.is_some() {
                        confidence_basis.push("place".to_string());
                    }
                    frames.push(EventFrame {
                        id: format!("ev_{scene_index:04}_{trigger_start:08}"),
                        event_type: event_type.clone(),
                        trigger: trigger_surface.to_string(),
                        trigger_start_byte: trigger_start,
                        trigger_end_byte: trigger_end,
                        participants,
                        place_id: place.map(|place| place.id.clone()),
                        time: temporal.map(|(_, time)| time),
                        status: status(sentence_text, base_language).to_string(),
                        scene_ids: vec![scene_id.to_string()],
                        evidence: vec![EventEvidence {
                            scene_id: scene_id.to_string(),
                            start_byte: sentence.start_byte,
                            end_byte: sentence.end_byte,
                            text: evidence_text.to_string(),
                        }],
                        confidence_basis,
                        source_fact_ids: Vec::new(),
                    });
                    if frames.len() > max_frames {
                        frames.sort_by(|a, b| {
                            a.trigger_start_byte
                                .cmp(&b.trigger_start_byte)
                                .then(a.event_type.cmp(&b.event_type))
                        });
                        frames.pop();
                    }
                }
            }
        }
    }
    // An active token can also match the passive entry of the same lexeme,
    // producing two frames over one span. The richer frame is the grounded
    // one: keep it visible when the classifier keeps only one.
    frames.sort_by(|a, b| {
        a.trigger_start_byte
            .cmp(&b.trigger_start_byte)
            .then(a.event_type.cmp(&b.event_type))
            .then(b.participants.len().cmp(&a.participants.len()))
            .then(a.trigger.cmp(&b.trigger))
    });
    frames.dedup_by(|a, b| {
        a.trigger_start_byte == b.trigger_start_byte && a.event_type == b.event_type
    });
    // A participial duplicate and its finite paraphrase can point at the same
    // date ("Nato ..." / "nacque ..."). Keep the frame with grounded people.
    let mut keep = vec![true; frames.len()];
    for index in 0..frames.len() {
        if !keep[index] || !frames[index].participants.is_empty() {
            continue;
        }
        if let Some(other) = frames.iter().enumerate().find_map(|(other_index, other)| {
            (other_index != index
                && other.event_type == frames[index].event_type
                && !other.participants.is_empty()
                && other.time.as_ref().map(|time| &time.normalized_date)
                    == frames[index]
                        .time
                        .as_ref()
                        .map(|time| &time.normalized_date))
            .then_some(other_index)
        }) {
            if other < index {
                keep[index] = false;
            } else {
                keep[index] = false;
                keep[other] = true;
            }
        }
    }
    frames = frames
        .into_iter()
        .zip(keep)
        .filter_map(|(frame, keep)| keep.then_some(frame))
        .collect();
    frames.truncate(max_frames);
    frames
}

fn trigger_spans(
    sentence: &str,
    sentence_start: usize,
    trigger: &TriggerEntry,
    tokens: &[visualner::AnalyzedToken],
) -> Vec<(usize, usize)> {
    if trigger.voice == "nominal" {
        return trigger
            .surface
            .as_deref()
            .map(|surface| whole_surface_spans(sentence, surface))
            .unwrap_or_default();
    }
    let lemma = trigger.lemma.to_lowercase();
    let tagged: Vec<(usize, usize)> = tokens
        .iter()
        .filter(|token| {
            token
                .lemma
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(&lemma))
                && token
                    .upos
                    .as_deref()
                    .is_some_and(|upos| matches!(upos, "VERB" | "AUX" | "ADJ"))
        })
        .filter(|token| {
            token.start >= sentence_start && token.end <= sentence_start + sentence.len()
        })
        .map(|token| (token.start, token.end))
        .map(|(start, end)| (start - sentence_start, end - sentence_start))
        .filter(|(start, end)| sentence.is_char_boundary(*start) && sentence.is_char_boundary(*end))
        .collect();
    if tagged.is_empty() {
        trigger
            .surface
            .as_deref()
            .map(|surface| whole_surface_spans(sentence, surface))
            .unwrap_or_default()
    } else {
        tagged
    }
}

/// Reconciles repeated, asserted event mentions across scenes. A shared
/// participant is required, as is an exact shared date or place; conflicting
/// explicit dates prevent a merge.
pub(crate) fn merge_cross_scene(
    frames: Vec<EventFrame>,
    threads: &[semantic_core::EntityThread],
) -> Vec<EventFrame> {
    let mut merged: Vec<EventFrame> = Vec::new();
    for mut frame in frames {
        let match_index = merged.iter().position(|existing| {
            existing.event_type == frame.event_type
                && existing.status == "asserted"
                && frame.status == "asserted"
                && shares_threaded_participant(existing, &frame, threads)
                && compatible_dates(existing, &frame)
                && (same_date(existing, &frame) || same_place(existing, &frame))
        });
        if let Some(index) = match_index {
            let existing = &mut merged[index];
            for scene_id in frame.scene_ids.drain(..) {
                if !existing.scene_ids.contains(&scene_id) {
                    existing.scene_ids.push(scene_id);
                }
            }
            existing.evidence.append(&mut frame.evidence);
            existing.source_fact_ids.append(&mut frame.source_fact_ids);
            existing.source_fact_ids.sort();
            existing.source_fact_ids.dedup();
            for basis in frame.confidence_basis {
                if !existing.confidence_basis.contains(&basis) {
                    existing.confidence_basis.push(basis);
                }
            }
        } else {
            merged.push(frame);
        }
    }
    merged
}

fn shares_threaded_participant(
    left: &EventFrame,
    right: &EventFrame,
    threads: &[semantic_core::EntityThread],
) -> bool {
    left.participants.iter().any(|a| {
        right.participants.iter().any(|b| {
            a.entity_id == b.entity_id
                && threads.iter().any(|thread| {
                    thread.entity_id == a.entity_id
                        && left
                            .scene_ids
                            .iter()
                            .any(|scene| thread.scene_ids.contains(scene))
                        && right
                            .scene_ids
                            .iter()
                            .any(|scene| thread.scene_ids.contains(scene))
                })
        })
    })
}

fn date(frame: &EventFrame) -> Option<&str> {
    frame.time.as_ref()?.normalized_date.as_deref()
}

fn compatible_dates(left: &EventFrame, right: &EventFrame) -> bool {
    !matches!((date(left), date(right)), (Some(a), Some(b)) if a != b)
}

fn same_date(left: &EventFrame, right: &EventFrame) -> bool {
    matches!((date(left), date(right)), (Some(a), Some(b)) if a == b)
}

fn same_place(left: &EventFrame, right: &EventFrame) -> bool {
    matches!((&left.place_id, &right.place_id), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
