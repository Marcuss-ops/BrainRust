//! Trigger classification and event frame extraction.

use std::collections::{HashMap, HashSet};

use semantic_core::{Candidate, EventEvidence, EventFrame, EventParticipant, SentenceSpan};

use super::lexicon::{lexicon, TriggerEntry};
use super::proximity::{
    active_perfect_context, established_event_agent, has_possessive_cue, nearest_candidate,
    passive_context, preceding_sentence_person, status, temporal_slot, whole_surface_spans,
};
use super::SceneEventInput;
#[cfg(test)]
use super::MAX_EVENT_FRAMES;
#[cfg(test)]
use semantic_core::{Place, TemporalFact};

#[cfg(test)]
pub(super) fn extract(
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
pub(super) fn extract_bounded(input: SceneEventInput<'_>, max_frames: usize) -> Vec<EventFrame> {
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

pub(super) fn trigger_spans(
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
