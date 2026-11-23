//! Participant proximity and event status helpers.

use semantic_core::{Candidate, EventFrame, EventTime, SentenceSpan, TemporalFact};

pub(super) fn whole_surface_spans(text: &str, surface: &str) -> Vec<(usize, usize)> {
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

pub(super) fn passive_context(sentence: &str, trigger_start: usize, language: &str) -> bool {
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

pub(super) fn active_perfect_context(sentence: &str, trigger_start: usize, language: &str) -> bool {
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

pub(super) fn nearest_candidate<'a>(
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

pub(super) fn preceding_sentence_person(
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
pub(super) fn established_event_agent(
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

pub(super) fn has_possessive_cue(sentence: &str, language: &str) -> bool {
    let Some(possessives) = semantic_core::language_profiles::get(language)
        .and_then(|profile| profile.word_sets.get("possessives"))
    else {
        return false;
    };
    semantic_core::discourse::word_spans(sentence)
        .iter()
        .any(|word| possessives.iter().any(|cue| cue == &word.lowered))
}

pub(super) fn status(sentence: &str, language: &str) -> &'static str {
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

pub(super) fn temporal_slot(
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
