//! Conservative, clause-local event frames grounded in typed triggers and
//! already-grounded participants, dates, and place mentions.

use std::collections::BTreeMap;

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
        // must not silently attach the nearest unrelated name.
        if tokens.iter().any(|token| token.id.is_some()) {
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
                    let attributive_participle = tokens.iter().any(|token| {
                        token.start == trigger_start
                            && token.features.as_deref().is_some_and(|features| {
                                features
                                    .split('|')
                                    .any(|feature| feature == "VerbForm=Part")
                            })
                            && token.dependency.as_deref().is_some_and(|relation| {
                                relation == "amod"
                                    || relation == "acl"
                                    || relation.starts_with("acl:")
                            })
                    });
                    let participial_predicate = tokens.iter().any(|token| {
                        token.start == trigger_start
                            && token.features.as_deref().is_some_and(|features| {
                                features
                                    .split('|')
                                    .any(|feature| feature == "VerbForm=Part")
                            })
                    });
                    let modifier_like_trigger = tokens.iter().any(|token| {
                        token.start == trigger_start
                            && (token.upos.as_deref() == Some("ADJ")
                                || token.dependency.as_deref().is_some_and(|relation| {
                                    relation == "amod"
                                        || relation == "acl"
                                        || relation.starts_with("acl:")
                                }))
                    });
                    // Only a participle can modify a work attributively. The
                    // dependency parser also attaches finite verbs to the noun
                    // they follow ("Nel 2024 pubblicò La vita s'impara" can come
                    // back as an advcl of the title), and treating that as a
                    // passive participle hides the real active agent.
                    let attributive_work = participial_predicate
                        && tokens.iter().any(|trigger_token| {
                            trigger_token.start == trigger_start
                                && trigger_token.head.is_some_and(|head_id| {
                                    tokens.iter().any(|head| {
                                        head.id == Some(head_id)
                                            && head.start >= sentence.start_byte
                                            && head.end <= sentence.end_byte
                                            && candidates.iter().any(|candidate| {
                                                candidate.kind == "WORK"
                                                    && candidate.start_byte <= head.start
                                                    && candidate.end_byte >= head.end
                                            })
                                    })
                                })
                        });
                    if attributive_participle && !attributive_work {
                        continue;
                    }
                    if (participial_predicate || modifier_like_trigger)
                        && !attributive_work
                        && event_type == "launch_release"
                        && !passive_context(sentence_text, local_start, base_language)
                        && !active_perfect_context(sentence_text, local_start, base_language)
                    {
                        continue;
                    }
                    let passive = attributive_work
                        || passive_context(sentence_text, local_start, base_language)
                        || tokens.iter().any(|token| {
                            token.start == trigger_start
                                && (token.features.as_deref().is_some_and(|features| {
                                    features.split('|').any(|feature| feature == "Voice=Pass")
                                }) || token.dependency.as_deref().is_some_and(|relation| {
                                    relation == "nsubj:pass" || relation == "csubj:pass"
                                }))
                        })
                        || tokens.iter().any(|token| {
                            token.start == trigger_start
                                && token.id.is_some_and(|trigger_id| {
                                    tokens.iter().any(|argument| {
                                        argument.sentence_id == token.sentence_id
                                            && argument.head == Some(trigger_id)
                                            && argument.dependency.as_deref().is_some_and(
                                                |relation| {
                                                    relation == "obl:agent"
                                                        || relation == "nmod:agent"
                                                        || relation == "agent"
                                                },
                                            )
                                    })
                                })
                        });
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
                                established_event_agent(&frames, &event_type, scene_id)
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
    frames.sort_by(|a, b| {
        a.trigger_start_byte
            .cmp(&b.trigger_start_byte)
            .then(a.event_type.cmp(&b.event_type))
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
mod tests {
    use super::*;

    #[test]
    fn event_matching_uses_shared_udpipe_lemma_and_offsets() {
        let source = "Augias nacque a Roma.";
        let start = source.find("nacque").unwrap();
        let trigger = TriggerEntry {
            surface: Some("è nato".into()),
            lemma: "nascere".into(),
            voice: "active".into(),
            roles: BTreeMap::new(),
            pro_drop_roles: Vec::new(),
        };
        let tokens = vec![visualner::AnalyzedToken {
            text: "nacque".into(),
            lemma: Some("nascere".into()),
            upos: Some("VERB".into()),
            features: None,
            dependency: Some("root".into()),
            id: Some(2),
            head: Some(0),
            sentence_id: Some(0),
            start,
            end: start + "nacque".len(),
        }];
        assert_eq!(
            trigger_spans(source, 0, &trigger, &tokens),
            vec![(start, start + "nacque".len())]
        );
    }

    #[test]
    fn event_participant_uses_udpipe_subject_dependency() {
        let source = "Anna Bianchi nacque a Roma.";
        let event_start = source.find("nacque").unwrap();
        let person = candidate(source, "Anna Bianchi", "person_1", "PERSON");
        let tokens = vec![
            visualner::AnalyzedToken {
                text: "Anna".into(),
                lemma: Some("Anna".into()),
                upos: Some("PROPN".into()),
                features: None,
                dependency: Some("nsubj".into()),
                id: Some(1),
                head: Some(3),
                sentence_id: Some(0),
                start: 0,
                end: 4,
            },
            visualner::AnalyzedToken {
                text: "Bianchi".into(),
                lemma: Some("Bianchi".into()),
                upos: Some("PROPN".into()),
                features: None,
                dependency: Some("flat:name".into()),
                id: Some(2),
                head: Some(1),
                sentence_id: Some(0),
                start: 5,
                end: 12,
            },
            visualner::AnalyzedToken {
                text: "nacque".into(),
                lemma: Some("nascere".into()),
                upos: Some("VERB".into()),
                features: Some("VerbForm=Fin".into()),
                dependency: Some("root".into()),
                id: Some(3),
                head: Some(0),
                sentence_id: Some(0),
                start: event_start,
                end: event_start + 6,
            },
        ];
        let sentences = semantic_core::split_sentences(source, "it", None);
        let candidates = [person];
        let found = nearest_candidate(
            &candidates,
            &tokens,
            event_start,
            event_start + 6,
            &sentences[0],
            "subject",
            false,
        );
        assert_eq!(found.map(|value| value.id.as_str()), Some("person_1"));
    }

    #[test]
    fn passive_event_links_the_agent_from_udpipe_oblique_agent() {
        let source = "La Repubblica fu fondata da Eugenio Scalfari.";
        let trigger_start = source.find("fondata").unwrap();
        let organization = candidate(source, "Repubblica", "org_1", "ORGANIZATION");
        let person = candidate(source, "Eugenio Scalfari", "person_1", "PERSON");
        let tokens = vec![
            visualner::AnalyzedToken {
                text: "Repubblica".into(),
                lemma: Some("Repubblica".into()),
                upos: Some("PROPN".into()),
                features: None,
                dependency: Some("nsubj:pass".into()),
                id: Some(2),
                head: Some(4),
                sentence_id: Some(0),
                start: source.find("Repubblica").unwrap(),
                end: source.find("Repubblica").unwrap() + "Repubblica".len(),
            },
            visualner::AnalyzedToken {
                text: "fondata".into(),
                lemma: Some("fondare".into()),
                upos: Some("VERB".into()),
                features: Some("Voice=Pass".into()),
                dependency: Some("root".into()),
                id: Some(4),
                head: Some(0),
                sentence_id: Some(0),
                start: trigger_start,
                end: trigger_start + "fondata".len(),
            },
            visualner::AnalyzedToken {
                text: "Scalfari".into(),
                lemma: Some("Scalfari".into()),
                upos: Some("PROPN".into()),
                features: None,
                dependency: Some("obl:agent".into()),
                id: Some(8),
                head: Some(4),
                sentence_id: Some(0),
                start: source.find("Scalfari").unwrap(),
                end: source.find("Scalfari").unwrap() + "Scalfari".len(),
            },
        ];
        let sentences = semantic_core::split_sentences(source, "it", None);
        let candidates = [organization, person];
        let patient = nearest_candidate(
            &candidates,
            &tokens,
            trigger_start,
            trigger_start + "fondata".len(),
            &sentences[0],
            "subject",
            false,
        );
        let agent = nearest_candidate(
            &candidates,
            &tokens,
            trigger_start,
            trigger_start + "fondata".len(),
            &sentences[0],
            "agent",
            false,
        );
        assert_eq!(patient.map(|value| value.id.as_str()), Some("org_1"));
        assert_eq!(agent.map(|value| value.id.as_str()), Some("person_1"));
    }

    fn event(scene: &str, date: &str, person: &str) -> EventFrame {
        EventFrame {
            id: format!("ev_{scene}"),
            event_type: "arrest".to_string(),
            trigger: "arrested".to_string(),
            trigger_start_byte: 0,
            trigger_end_byte: 8,
            participants: vec![EventParticipant {
                role: "patient".to_string(),
                entity_id: person.to_string(),
            }],
            place_id: None,
            time: Some(EventTime {
                fact_index: 0,
                normalized_date: Some(date.to_string()),
                derived: false,
            }),
            status: "asserted".to_string(),
            scene_ids: vec![scene.to_string()],
            evidence: vec![EventEvidence {
                scene_id: scene.to_string(),
                start_byte: 0,
                end_byte: 20,
                text: "Maya was arrested.".to_string(),
            }],
            confidence_basis: vec!["trigger".to_string(), "participant".to_string()],
            source_fact_ids: vec![format!("fact_{scene}")],
        }
    }

    fn thread(entity_id: &str, scene_ids: &[&str]) -> semantic_core::EntityThread {
        semantic_core::EntityThread {
            entity_id: entity_id.to_string(),
            canonical_surface: "Maya Chen".to_string(),
            entity_type: "PERSON".to_string(),
            scene_ids: scene_ids.iter().map(|scene| (*scene).to_string()).collect(),
            first_scene_id: scene_ids[0].to_string(),
            last_scene_id: scene_ids[scene_ids.len() - 1].to_string(),
            mention_count: scene_ids.len(),
            return_count: 0,
            beats: Vec::new(),
        }
    }

    #[test]
    fn cross_scene_events_merge_with_shared_identity_and_date_only() {
        let frames = vec![
            event("s1", "2019", "per_maya"),
            event("s2", "2019", "per_maya"),
        ];
        let no_thread = merge_cross_scene(frames.clone(), &[]);
        assert_eq!(
            no_thread.len(),
            2,
            "identity reuse without a discourse thread must not merge"
        );
        let merged = merge_cross_scene(frames, &[thread("per_maya", &["s1", "s2"])]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].scene_ids, ["s1", "s2"]);
        assert_eq!(merged[0].evidence.len(), 2);
        assert_eq!(merged[0].source_fact_ids, ["fact_s1", "fact_s2"]);

        let threads = [
            thread("per_maya", &["s1", "s2"]),
            thread("per_other", &["s1", "s2"]),
        ];
        let conflicting_dates = merge_cross_scene(
            vec![
                event("s1", "2019", "per_maya"),
                event("s2", "2020", "per_maya"),
            ],
            &threads,
        );
        assert_eq!(conflicting_dates.len(), 2);
        let different_person = merge_cross_scene(
            vec![
                event("s1", "2019", "per_maya"),
                event("s2", "2019", "per_other"),
            ],
            &threads,
        );
        assert_eq!(different_person.len(), 2);
    }

    fn candidate(source: &str, text: &str, id: &str, kind: &str) -> Candidate {
        let start_byte = source.find(text).unwrap();
        Candidate {
            id: id.to_string(),
            text: text.to_string(),
            kind: kind.to_string(),
            start_byte,
            end_byte: start_byte + text.len(),
            sentence_index: 0,
            evidence: text.to_string(),
            score: 0.9,
            provenance: "test".to_string(),
        }
    }

    fn extract_with_model(
        scene_index: usize,
        scene_id: &str,
        source: &str,
        language: &str,
        candidates: &[Candidate],
    ) -> Vec<EventFrame> {
        let analysis = visualner::analyze(
            source,
            &visualner::ExtractOptions {
                language: language.to_string(),
                entity_count: 50,
            },
        )
        .expect("tagged source analysis");
        let temporal = semantic_core::temporal_facts(source, language);
        extract_bounded_with_tokens(
            SceneEventInput {
                scene_index,
                scene_id,
                source,
                language,
                candidates,
                temporal_facts: &temporal,
                places: &[],
            },
            &analysis.tokens,
            MAX_EVENT_FRAMES,
        )
    }

    fn place(source: &str, text: &str) -> Place {
        let start_byte = source.find(text).unwrap();
        Place {
            id: "pl_1".to_string(),
            entity_local_id: "loc_1".to_string(),
            kb_id: Some("wd:Q90".to_string()),
            link_status: "linked".to_string(),
            kind: "city".to_string(),
            name: text.to_string(),
            lat: Some(48.8566),
            lon: Some(2.3522),
            parent_ids: Vec::new(),
            role: "setting".to_string(),
            start_byte,
            end_byte: start_byte + text.len(),
            sentence_index: 0,
            evidence: text.to_string(),
        }
    }

    #[test]
    fn passive_arrest_frame_attaches_patient_place_and_date() {
        let source = "Maya Chen was arrested in Paris in 2019.";
        let candidates = vec![candidate(source, "Maya Chen", "per_1", "PERSON")];
        let dates = semantic_core::temporal_facts(source, "en");
        let places = vec![place(source, "Paris")];
        let frames = extract(0, "s1", source, "en", &candidates, &dates, &places);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event_type, "arrest");
        assert_eq!(frames[0].participants[0].role, "patient");
        assert_eq!(frames[0].participants[0].entity_id, "per_1");
        assert_eq!(frames[0].place_id.as_deref(), Some("pl_1"));
        assert_eq!(
            frames[0]
                .time
                .as_ref()
                .and_then(|time| time.normalized_date.as_deref()),
            Some("2019")
        );
        assert_eq!(
            &source[frames[0].trigger_start_byte..frames[0].trigger_end_byte],
            "arrested"
        );
    }

    #[test]
    fn active_arrest_assigns_agent_and_patient_by_grounded_order() {
        let source = "Officer Lee arrested Maya Chen.";
        let candidates = vec![
            candidate(source, "Officer Lee", "per_1", "PERSON"),
            candidate(source, "Maya Chen", "per_2", "PERSON"),
        ];
        let frames = extract(0, "s1", source, "en", &candidates, &[], &[]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].participants.len(), 2);
        assert!(frames[0]
            .participants
            .iter()
            .any(|p| p.role == "agent" && p.entity_id == "per_1"));
        assert!(frames[0]
            .participants
            .iter()
            .any(|p| p.role == "patient" && p.entity_id == "per_2"));
    }

    #[test]
    fn trigger_without_another_grounded_slot_is_dropped() {
        let source = "The arrest was discussed.";
        assert!(extract(0, "s1", source, "en", &[], &[], &[]).is_empty());
    }

    #[test]
    fn negation_is_preserved_on_a_grounded_frame() {
        let source = "Maya Chen was not arrested.";
        let candidates = vec![candidate(source, "Maya Chen", "per_1", "PERSON")];
        let frames = extract(0, "s1", source, "en", &candidates, &[], &[]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].status, "negated");
    }

    #[test]
    fn italian_passive_trigger_uses_italian_voice_and_cues() {
        let source = "Anna Bianchi è stata arrestata a Parigi.";
        let candidates = vec![candidate(source, "Anna Bianchi", "per_1", "PERSON")];
        let frames = extract_with_model(0, "s1", source, "it", &candidates);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event_type, "arrest");
        assert_eq!(frames[0].participants[0].role, "patient");
    }

    #[test]
    fn italian_pro_drop_subject_uses_a_unique_recurrent_person_focus() {
        let source = "Corrado Augias lavorò a Roma. Nel 2006 pubblicò un libro.";
        let mut person = candidate(source, "Corrado Augias", "person_corrado", "PERSON");
        let second_mention = Candidate {
            start_byte: source.rfind("Corrado Augias").unwrap(),
            end_byte: source.rfind("Corrado Augias").unwrap() + "Corrado Augias".len(),
            ..person.clone()
        };
        person.sentence_index = 0;
        let frames = extract_with_model(0, "s1", source, "it", &[person, second_mention]);
        let publication = frames
            .iter()
            .find(|frame| frame.event_type == "launch_release")
            .expect("lemma-triggered publication frame");
        assert!(publication.participants.iter().any(|participant| {
            participant.role == "agent" && participant.entity_id == "person_corrado"
        }));
        assert!(publication
            .confidence_basis
            .iter()
            .any(|basis| basis == "pro_drop_coref"));
    }

    #[test]
    fn italian_passive_publication_uses_possessive_for_unique_author_coreference() {
        let source =
            "Corrado Augias fu autore. Tra i suoi successi troviamo Quel treno da Vienna, pubblicato nel 1981.";
        let person = candidate(source, "Corrado Augias", "person_corrado", "PERSON");
        let work = candidate(source, "Quel treno da Vienna", "work_train", "WORK");
        let trigger_start = source.find("pubblicato").unwrap();
        let head_start = source.find("treno").unwrap();
        let tokens = vec![
            visualner::AnalyzedToken {
                text: "treno".into(),
                lemma: Some("treno".into()),
                upos: Some("NOUN".into()),
                features: None,
                dependency: Some("nsubj:pass".into()),
                id: Some(6),
                head: Some(7),
                sentence_id: Some(1),
                start: head_start,
                end: head_start + "treno".len(),
            },
            visualner::AnalyzedToken {
                text: "pubblicato".into(),
                lemma: Some("pubblicare".into()),
                upos: Some("VERB".into()),
                features: Some("VerbForm=Part|Voice=Pass".into()),
                dependency: Some("acl".into()),
                id: Some(7),
                head: Some(6),
                sentence_id: Some(1),
                start: trigger_start,
                end: trigger_start + "pubblicato".len(),
            },
        ];
        let temporal = semantic_core::temporal_facts(source, "it");
        let frames = extract_bounded_with_tokens(
            SceneEventInput {
                scene_index: 0,
                scene_id: "s1",
                source,
                language: "it",
                candidates: &[person.clone(), work.clone()],
                temporal_facts: &temporal,
                places: &[],
            },
            &tokens,
            MAX_EVENT_FRAMES,
        );
        let publication = frames
            .iter()
            .find(|frame| frame.event_type == "launch_release")
            .expect("passive publication frame");
        assert!(publication.participants.iter().any(|participant| {
            participant.role == "agent" && participant.entity_id == "person_corrado"
        }));
        assert!(publication.participants.iter().any(|participant| {
            participant.role == "patient" && participant.entity_id == "work_train"
        }));

        let no_possessive = source.replace("i suoi successi", "i successi");
        let mut person_without_possessive = person;
        person_without_possessive.start_byte = no_possessive.find("Corrado Augias").unwrap();
        person_without_possessive.end_byte =
            person_without_possessive.start_byte + "Corrado Augias".len();
        let work_start = no_possessive.find("Quel treno da Vienna").unwrap();
        let work_without_possessive = Candidate {
            start_byte: work_start,
            end_byte: work_start + "Quel treno da Vienna".len(),
            text: "Quel treno da Vienna".into(),
            evidence: "Quel treno da Vienna".into(),
            ..work
        };
        let trigger_start = no_possessive.find("pubblicato").unwrap();
        let head_start = no_possessive.find("treno").unwrap();
        let no_possessive_tokens = [
            visualner::AnalyzedToken {
                start: head_start,
                end: head_start + "treno".len(),
                ..tokens[0].clone()
            },
            visualner::AnalyzedToken {
                start: trigger_start,
                end: trigger_start + "pubblicato".len(),
                ..tokens[1].clone()
            },
        ];
        let temporal = semantic_core::temporal_facts(&no_possessive, "it");
        let frames = extract_bounded_with_tokens(
            SceneEventInput {
                scene_index: 0,
                scene_id: "s2",
                source: &no_possessive,
                language: "it",
                candidates: &[person_without_possessive, work_without_possessive],
                temporal_facts: &temporal,
                places: &[],
            },
            &no_possessive_tokens,
            MAX_EVENT_FRAMES,
        );
        let publication = frames
            .iter()
            .find(|frame| frame.event_type == "launch_release")
            .expect("passive publication still has the grounded work and date");
        assert!(!publication
            .participants
            .iter()
            .any(|participant| participant.role == "agent"));
    }

    // A biography often describes the family between two publications, so the
    // sentence before the omitted subject names two people. The publication
    // history of the scene still identifies the author.
    #[test]
    fn ambiguous_omitted_subject_reuses_the_scene_established_publication_agent() {
        let source = "Corrado Augias nacque a Roma. Nel 2006 pubblicò Inchiesta su Gesù. Era sposato con Daniela Pasti e aveva una figlia, Natalia. Nel 2024 pubblicò La vita s'impara.";
        let candidates = vec![
            candidate(source, "Corrado Augias", "person_corrado", "PERSON"),
            candidate(source, "Daniela Pasti", "person_daniela", "PERSON"),
            candidate(source, "Natalia", "person_natalia", "PERSON"),
            candidate(source, "Inchiesta su Gesù", "work_gesu", "WORK"),
            candidate(source, "La vita s'impara", "work_vita", "WORK"),
        ];
        let frames = extract_with_model(0, "s1", source, "it", &candidates);
        let publication = frames
            .iter()
            .find(|frame| {
                frame.event_type == "launch_release"
                    && frame
                        .evidence
                        .iter()
                        .any(|evidence| evidence.text.contains("2024"))
            })
            .expect("2024 publication frame");
        assert!(publication.participants.iter().any(|participant| {
            participant.role == "agent" && participant.entity_id == "person_corrado"
        }));
        assert!(publication
            .confidence_basis
            .iter()
            .any(|basis| basis == "discourse_agent_coref"));
    }

    #[test]
    fn election_agreement_and_accident_types_have_grounded_slots() {
        let election = "Maya Chen was elected in 2019.";
        let election_candidates = vec![candidate(election, "Maya Chen", "per_1", "PERSON")];
        let election_frames = extract(
            0,
            "s1",
            election,
            "en",
            &election_candidates,
            &semantic_core::temporal_facts(election, "en"),
            &[],
        );
        assert_eq!(election_frames[0].event_type, "election");

        let agreement = "Maya Chen signed an agreement in 2019.";
        let agreement_candidates = vec![candidate(agreement, "Maya Chen", "per_1", "PERSON")];
        let agreement_frames = extract(
            0,
            "s1",
            agreement,
            "en",
            &agreement_candidates,
            &semantic_core::temporal_facts(agreement, "en"),
            &[],
        );
        assert_eq!(agreement_frames[0].event_type, "agreement");
        assert_eq!(agreement_frames[0].participants[0].role, "signatory");

        let accident = "A truck crashed in Paris in 2019.";
        let accident_candidates = vec![candidate(accident, "truck", "veh_1", "VEHICLE")];
        let accident_frames = extract(
            0,
            "s1",
            accident,
            "en",
            &accident_candidates,
            &semantic_core::temporal_facts(accident, "en"),
            &[],
        );
        assert_eq!(accident_frames[0].event_type, "accident_disaster");
        assert_eq!(accident_frames[0].participants[0].role, "vehicle");
    }

    #[test]
    fn all_core_event_types_have_grounded_english_and_italian_triggers() {
        let cases = [
            ("en", "Maya Chen was born.", "Maya Chen", "PERSON", "birth"),
            ("en", "Maya Chen died.", "Maya Chen", "PERSON", "death"),
            (
                "en",
                "Maya Chen was convicted.",
                "Maya Chen",
                "PERSON",
                "trial_verdict",
            ),
            (
                "en",
                "Maya Chen was appointed.",
                "Maya Chen",
                "PERSON",
                "appointment_resignation",
            ),
            (
                "en",
                "Acme Labs was founded.",
                "Acme Labs",
                "ORG",
                "founding",
            ),
            (
                "en",
                "Acme Labs acquired Beta Corp.",
                "Acme Labs",
                "ORG",
                "acquisition_merger",
            ),
            (
                "en",
                "Acme Labs launched Orion.",
                "Acme Labs",
                "ORG",
                "launch_release",
            ),
            (
                "en",
                "Maya Chen attacked Lee.",
                "Maya Chen",
                "PERSON",
                "attack_battle",
            ),
            (
                "en",
                "Maya Chen defeated Lee.",
                "Maya Chen",
                "PERSON",
                "match_competition_result",
            ),
            (
                "it",
                "Anna Bianchi è nata.",
                "Anna Bianchi",
                "PERSON",
                "birth",
            ),
            (
                "it",
                "Anna Bianchi nacque a Roma.",
                "Anna Bianchi",
                "PERSON",
                "birth",
            ),
            (
                "it",
                "Anna Bianchi è morta.",
                "Anna Bianchi",
                "PERSON",
                "death",
            ),
            (
                "it",
                "Anna Bianchi morì a Roma.",
                "Anna Bianchi",
                "PERSON",
                "death",
            ),
            (
                "it",
                "Anna Bianchi pubblicò un libro.",
                "Anna Bianchi",
                "PERSON",
                "launch_release",
            ),
            (
                "it",
                "Anna Bianchi restituì un premio.",
                "Anna Bianchi",
                "PERSON",
                "award_return",
            ),
            (
                "it",
                "Anna Bianchi è stata condannata.",
                "Anna Bianchi",
                "PERSON",
                "trial_verdict",
            ),
            (
                "it",
                "Anna Bianchi è stata nominata.",
                "Anna Bianchi",
                "PERSON",
                "appointment_resignation",
            ),
            (
                "it",
                "Acme Labs è stata fondata.",
                "Acme Labs",
                "ORG",
                "founding",
            ),
            (
                "it",
                "Acme Labs ha acquisito Beta Corp.",
                "Acme Labs",
                "ORG",
                "acquisition_merger",
            ),
            (
                "it",
                "Acme Labs ha lanciato Orion.",
                "Acme Labs",
                "ORG",
                "launch_release",
            ),
            (
                "it",
                "Anna Bianchi ha attaccato Marco.",
                "Anna Bianchi",
                "PERSON",
                "attack_battle",
            ),
            (
                "it",
                "Milan ha sconfitto Roma.",
                "Milan",
                "ORG",
                "match_competition_result",
            ),
        ];

        for (index, (language, source, participant, kind, expected_type)) in
            cases.into_iter().enumerate()
        {
            let candidate = candidate(source, participant, "entity_1", kind);
            let frames = extract_with_model(index, "event", source, language, &[candidate]);
            assert!(
                frames.iter().any(|frame| frame.event_type == expected_type),
                "missing {expected_type} frame for {language}: {source}; got {frames:?}"
            );
        }
    }

    #[test]
    fn all_core_event_types_have_grounded_spanish_triggers() {
        let cases = [
            ("Maya Chen nació.", "Maya Chen", "PERSON", "birth"),
            ("Maya Chen murió.", "Maya Chen", "PERSON", "death"),
            ("Maya Chen fue arrestada.", "Maya Chen", "PERSON", "arrest"),
            (
                "Maya Chen fue condenada.",
                "Maya Chen",
                "PERSON",
                "trial_verdict",
            ),
            ("Maya Chen fue elegida.", "Maya Chen", "PERSON", "election"),
            (
                "Maya Chen fue nombrada.",
                "Maya Chen",
                "PERSON",
                "appointment_resignation",
            ),
            ("Acme Labs fue fundada.", "Acme Labs", "ORG", "founding"),
            (
                "Acme Labs adquirió Beta Corp.",
                "Acme Labs",
                "ORG",
                "acquisition_merger",
            ),
            (
                "Acme Labs lanzó Orion.",
                "Acme Labs",
                "ORG",
                "launch_release",
            ),
            (
                "El camión se estrelló.",
                "camión",
                "VEHICLE",
                "accident_disaster",
            ),
            (
                "Maya Chen atacó a Lee.",
                "Maya Chen",
                "PERSON",
                "attack_battle",
            ),
            (
                "Maya Chen derrotó a Lee.",
                "Maya Chen",
                "PERSON",
                "match_competition_result",
            ),
        ];

        for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
            let candidate = candidate(source, participant, "entity_1", kind);
            let frames = extract_with_model(index, "event", source, "es", &[candidate]);
            assert!(
                frames.iter().any(|frame| frame.event_type == expected_type),
                "missing {expected_type} frame for es: {source}; got {frames:?}"
            );
        }
    }

    #[test]
    fn all_core_event_types_have_grounded_french_triggers() {
        let cases = [
            ("Maya Chen est née à Paris.", "Maya Chen", "PERSON", "birth"),
            (
                "Maya Chen est morte à Paris.",
                "Maya Chen",
                "PERSON",
                "death",
            ),
            ("Maya Chen a été arrêtée.", "Maya Chen", "PERSON", "arrest"),
            (
                "Maya Chen a été condamnée.",
                "Maya Chen",
                "PERSON",
                "trial_verdict",
            ),
            ("Maya Chen a été élue.", "Maya Chen", "PERSON", "election"),
            (
                "Maya Chen a été nommée.",
                "Maya Chen",
                "PERSON",
                "appointment_resignation",
            ),
            ("Acme Labs a été fondée.", "Acme Labs", "ORG", "founding"),
            (
                "Acme Labs a acquis Beta Corp.",
                "Acme Labs",
                "ORG",
                "acquisition_merger",
            ),
            (
                "Acme Labs a lancé Orion.",
                "Acme Labs",
                "ORG",
                "launch_release",
            ),
            (
                "Avion Orion s'est écrasé.",
                "Avion Orion",
                "VEHICLE",
                "accident_disaster",
            ),
            (
                "Maya Chen a attaqué Lee.",
                "Maya Chen",
                "PERSON",
                "attack_battle",
            ),
            (
                "Milan a battu Roma.",
                "Milan",
                "ORG",
                "match_competition_result",
            ),
        ];

        for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
            let candidate = candidate(source, participant, "entity_fr", kind);
            let frames = extract(index, "event-fr", source, "fr", &[candidate], &[], &[]);
            assert!(
                frames.iter().any(|frame| frame.event_type == expected_type),
                "missing {expected_type} frame for fr: {source}; got {frames:?}"
            );
        }
    }

    #[test]
    fn french_negation_is_preserved_on_event_frames() {
        let source = "Maya Chen n'a pas été arrêtée.";
        let candidates = vec![candidate(source, "Maya Chen", "per_fr", "PERSON")];
        let frames = extract(0, "s-fr", source, "fr", &candidates, &[], &[]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].status, "negated");
    }

    #[test]
    fn french_uncertainty_is_preserved_on_event_frames() {
        let source = "Maya Chen aurait été arrêtée.";
        let candidates = vec![candidate(source, "Maya Chen", "per_fr", "PERSON")];
        let frames = extract(0, "s-fr", source, "fr", &candidates, &[], &[]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].status, "uncertain");
    }

    #[test]
    fn all_core_event_types_have_grounded_portuguese_triggers() {
        let cases = [
            (
                "Maya Chen nasceu em Lisboa.",
                "Maya Chen",
                "PERSON",
                "birth",
            ),
            (
                "Maya Chen morreu em Lisboa.",
                "Maya Chen",
                "PERSON",
                "death",
            ),
            ("Maya Chen foi presa.", "Maya Chen", "PERSON", "arrest"),
            (
                "Maya Chen foi condenada.",
                "Maya Chen",
                "PERSON",
                "trial_verdict",
            ),
            ("Maya Chen foi eleita.", "Maya Chen", "PERSON", "election"),
            (
                "Maya Chen foi nomeada.",
                "Maya Chen",
                "PERSON",
                "appointment_resignation",
            ),
            ("Acme Labs foi fundada.", "Acme Labs", "ORG", "founding"),
            (
                "Acme Labs adquiriu Beta Corp.",
                "Acme Labs",
                "ORG",
                "acquisition_merger",
            ),
            (
                "Acme Labs lançou Orion.",
                "Acme Labs",
                "ORG",
                "launch_release",
            ),
            (
                "Avião Orion sofreu um acidente.",
                "Avião Orion",
                "VEHICLE",
                "accident_disaster",
            ),
            (
                "Maya Chen atacou Lee.",
                "Maya Chen",
                "PERSON",
                "attack_battle",
            ),
            (
                "Milan venceu Roma.",
                "Milan",
                "ORG",
                "match_competition_result",
            ),
        ];

        for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
            let candidate = candidate(source, participant, "entity_pt", kind);
            let frames = extract(index, "event-pt", source, "pt", &[candidate], &[], &[]);
            assert!(
                frames.iter().any(|frame| frame.event_type == expected_type),
                "missing {expected_type} frame for pt: {source}; got {frames:?}"
            );
        }
    }

    #[test]
    fn portuguese_negation_and_uncertainty_are_preserved_on_event_frames() {
        for (source, expected) in [
            ("Maya Chen não foi presa.", "negated"),
            ("Maya Chen foi presa, supostamente.", "uncertain"),
        ] {
            let candidates = vec![candidate(source, "Maya Chen", "per_pt", "PERSON")];
            let frames = extract(0, "s-pt", source, "pt", &candidates, &[], &[]);
            assert_eq!(frames.len(), 1, "{source}: {frames:?}");
            assert_eq!(frames[0].status, expected, "{source}: {frames:?}");
        }
    }

    #[test]
    fn all_core_event_types_have_grounded_german_triggers() {
        let cases = [
            (
                "Maya Chen wurde in Berlin geboren.",
                "Maya Chen",
                "PERSON",
                "birth",
            ),
            ("Maya Chen starb in Berlin.", "Maya Chen", "PERSON", "death"),
            (
                "Maya Chen wurde festgenommen.",
                "Maya Chen",
                "PERSON",
                "arrest",
            ),
            (
                "Maya Chen wurde verurteilt.",
                "Maya Chen",
                "PERSON",
                "trial_verdict",
            ),
            (
                "Maya Chen wurde gewählt.",
                "Maya Chen",
                "PERSON",
                "election",
            ),
            (
                "Maya Chen wurde ernannt.",
                "Maya Chen",
                "PERSON",
                "appointment_resignation",
            ),
            ("Acme Labs wurde gegründet.", "Acme Labs", "ORG", "founding"),
            (
                "Acme Labs übernahm Beta Corp.",
                "Acme Labs",
                "ORG",
                "acquisition_merger",
            ),
            (
                "Acme Labs veröffentlichte Orion.",
                "Acme Labs",
                "ORG",
                "launch_release",
            ),
            (
                "Flugzeug Orion stürzte ab.",
                "Flugzeug Orion",
                "VEHICLE",
                "accident_disaster",
            ),
            (
                "Maya Chen attackierte Lee.",
                "Maya Chen",
                "PERSON",
                "attack_battle",
            ),
            (
                "Milan besiegte Roma.",
                "Milan",
                "ORG",
                "match_competition_result",
            ),
        ];

        for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
            let candidate = candidate(source, participant, "entity_de", kind);
            let frames = extract(index, "event-de", source, "de", &[candidate], &[], &[]);
            assert!(
                frames.iter().any(|frame| frame.event_type == expected_type),
                "missing {expected_type} frame for de: {source}; got {frames:?}"
            );
        }
    }

    #[test]
    fn german_negation_and_uncertainty_are_preserved_on_event_frames() {
        for (source, expected) in [
            ("Maya Chen wurde nicht festgenommen.", "negated"),
            ("Maya Chen wurde angeblich festgenommen.", "uncertain"),
        ] {
            let candidates = vec![candidate(source, "Maya Chen", "per_de", "PERSON")];
            let frames = extract(0, "s-de", source, "de", &candidates, &[], &[]);
            assert_eq!(frames.len(), 1, "{source}: {frames:?}");
            assert_eq!(frames[0].status, expected, "{source}: {frames:?}");
        }
    }

    #[test]
    fn all_core_event_types_have_grounded_dutch_triggers() {
        let cases = [
            (
                "Maya Chen werd in Amsterdam geboren.",
                "Maya Chen",
                "PERSON",
                "birth",
            ),
            (
                "Maya Chen overleed in Amsterdam.",
                "Maya Chen",
                "PERSON",
                "death",
            ),
            (
                "Maya Chen werd gearresteerd.",
                "Maya Chen",
                "PERSON",
                "arrest",
            ),
            (
                "Maya Chen werd veroordeeld.",
                "Maya Chen",
                "PERSON",
                "trial_verdict",
            ),
            ("Maya Chen werd gekozen.", "Maya Chen", "PERSON", "election"),
            (
                "Maya Chen werd benoemd.",
                "Maya Chen",
                "PERSON",
                "appointment_resignation",
            ),
            ("Acme Labs werd opgericht.", "Acme Labs", "ORG", "founding"),
            (
                "Acme Labs verwierf Beta Corp.",
                "Acme Labs",
                "ORG",
                "acquisition_merger",
            ),
            (
                "Acme Labs publiceerde Orion.",
                "Acme Labs",
                "ORG",
                "launch_release",
            ),
            (
                "Vliegtuig Orion stortte neer.",
                "Vliegtuig Orion",
                "VEHICLE",
                "accident_disaster",
            ),
            (
                "Maya Chen bestormde Lee.",
                "Maya Chen",
                "PERSON",
                "attack_battle",
            ),
            (
                "Milan versloeg Roma.",
                "Milan",
                "ORG",
                "match_competition_result",
            ),
        ];

        for (index, (source, participant, kind, expected_type)) in cases.into_iter().enumerate() {
            let candidate = candidate(source, participant, "entity_nl", kind);
            let frames = extract(index, "event-nl", source, "nl", &[candidate], &[], &[]);
            assert!(
                frames.iter().any(|frame| frame.event_type == expected_type),
                "missing {expected_type} frame for nl: {source}; got {frames:?}"
            );
        }
    }

    #[test]
    fn dutch_negation_and_uncertainty_are_preserved_on_event_frames() {
        for (source, expected) in [
            ("Maya Chen werd niet gearresteerd.", "negated"),
            ("Maya Chen werd vermoedelijk gearresteerd.", "uncertain"),
        ] {
            let candidates = vec![candidate(source, "Maya Chen", "per_nl", "PERSON")];
            let frames = extract(0, "s-nl", source, "nl", &candidates, &[], &[]);
            assert_eq!(frames.len(), 1, "{source}: {frames:?}");
            assert_eq!(frames[0].status, expected, "{source}: {frames:?}");
        }
    }

    #[test]
    fn all_core_event_types_have_grounded_indonesian_polish_russian_and_turkish_triggers() {
        for language in ["id", "pl", "ru", "tr"] {
            let file = lexicon(language).expect("event lexicon should be compiled");
            for (event_type, triggers) in file.event_types {
                let first = &triggers[0];
                let trigger = first.surface.as_deref().unwrap_or(&first.lemma);
                let source = format!("Maya Chen {trigger}.");
                let candidate = candidate(&source, "Maya Chen", "per_test", "PERSON");
                let frames = extract(0, "event-test", &source, language, &[candidate], &[], &[]);
                assert!(
                    frames.iter().any(|frame| frame.event_type == event_type),
                    "missing {event_type} frame for {language}: {source}; got {frames:?}"
                );
            }
        }
    }

    #[test]
    fn negation_and_uncertainty_are_preserved_for_new_event_languages() {
        for (language, source, expected) in [
            ("id", "Maya Chen tidak ditangkap.", "negated"),
            ("id", "Maya Chen diduga ditangkap.", "uncertain"),
            ("pl", "Maya Chen nie został aresztowany.", "negated"),
            (
                "pl",
                "Maya Chen został prawdopodobnie aresztowany.",
                "uncertain",
            ),
            ("ru", "Maya Chen не арестован.", "negated"),
            ("ru", "Maya Chen якобы арестован.", "uncertain"),
            ("tr", "Maya Chen tutuklu değil.", "negated"),
            ("tr", "Maya Chen muhtemelen tutuklandı.", "uncertain"),
        ] {
            let candidates = vec![candidate(source, "Maya Chen", "per_test", "PERSON")];
            let frames = extract(0, "event-status", source, language, &candidates, &[], &[]);
            assert_eq!(frames.len(), 1, "{language} {source}: {frames:?}");
            assert_eq!(frames[0].status, expected, "{language} {source}");
        }
    }

    #[test]
    fn spanish_negation_is_preserved_on_event_frames() {
        let source = "Maya Chen no fue arrestada.";
        let candidates = vec![candidate(source, "Maya Chen", "per_1", "PERSON")];
        let frames = extract_with_model(0, "s1", source, "es", &candidates);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].status, "negated");
    }

    #[test]
    fn frame_cap_keeps_the_earliest_source_events() {
        let source = "Agreement in 2019. Agreement in 2020. Agreement in 2021.";
        let frames = extract_bounded(
            SceneEventInput {
                scene_index: 0,
                scene_id: "s1",
                source,
                language: "en",
                candidates: &[],
                temporal_facts: &semantic_core::temporal_facts(source, "en"),
                places: &[],
            },
            2,
        );
        assert_eq!(frames.len(), 2);
        assert!(frames[0].trigger_start_byte < frames[1].trigger_start_byte);
        assert_eq!(frames[0].event_type, "agreement");
    }
}
