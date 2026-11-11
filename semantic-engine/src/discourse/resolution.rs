//! Whole-script coreference resolution.
//!
//! The resolver only consumes already-grounded identities. Explicit mentions
//! are resolved by construction; pronouns and role nominals are resolved only
//! when one compatible antecedent remains after the active-window and global
//! uniqueness rules. Two plausible antecedents produce `ambiguous`, never a
//! silent pick. Quoted speech is never attributed to the narrator.

use std::collections::{BTreeMap, BTreeSet};

use semantic_core::narrative::DiscourseReference;
use semantic_core::{SceneInput, SceneManifest, SentenceSpan};

use super::references::{detect, in_quotes, DetectedReference};

#[derive(Clone)]
struct NamedMention {
    identity_id: String,
    scene_index: usize,
    scene_id: String,
    sentence_index: usize,
    start: usize,
    end: usize,
    surface: String,
}

struct IdentityInfo {
    canonical_surface: String,
    kind: String,
    mentions: Vec<NamedMention>,
}

struct Context<'a> {
    inputs: &'a [SceneInput],
    sentences: Vec<Vec<SentenceSpan>>,
    identities: BTreeMap<String, IdentityInfo>,
    scene_mentions: Vec<Vec<NamedMention>>,
}

enum RawReference {
    Explicit(NamedMention),
    Detected(DetectedReference),
}

impl RawReference {
    fn start(&self) -> usize {
        match self {
            Self::Explicit(mention) => mention.start,
            Self::Detected(reference) => reference.start,
        }
    }

    fn explicit(&self) -> bool {
        matches!(self, Self::Explicit(_))
    }
}

/// Resolves every scene into the additive `discourse.v1` reference list.
pub(super) fn resolve_references(
    language: &str,
    inputs: &[SceneInput],
    scenes: &[SceneManifest],
) -> Vec<DiscourseReference> {
    let context = Context::build(language, inputs, scenes);
    let mut output = Vec::new();
    for scene_index in 0..scenes.len() {
        let text = &context.inputs[scene_index].text;
        let named = &context.scene_mentions[scene_index];
        let mut raw: Vec<RawReference> = named
            .iter()
            .filter(|mention| context.is_referenceable(&mention.identity_id))
            .cloned()
            .map(RawReference::Explicit)
            .collect();
        for detected in detect(text, language) {
            // A pronoun or role may be extracted as a generic concept. It must
            // reach the resolver, not be frozen as an explicit name. Only an
            // overlap with a referenceable identity (a real name/org/place)
            // suppresses detection.
            if named.iter().any(|mention| {
                context.is_referenceable(&mention.identity_id)
                    && detected.start < mention.end
                    && mention.start < detected.end
            }) {
                continue;
            }
            raw.push(RawReference::Detected(detected));
        }
        raw.sort_by_key(|reference| (reference.start(), !reference.explicit()));

        for reference in raw {
            let resolved = match reference {
                RawReference::Explicit(mention) => explicit_reference(text, &context, &mention),
                RawReference::Detected(detected) => {
                    detected_reference(text, scene_index, &context, &detected, named)
                }
            };
            output.push(resolved);
        }
    }
    for (index, reference) in output.iter_mut().enumerate() {
        reference.id = format!("dref_{:04}", index + 1);
    }
    output
}

impl<'a> Context<'a> {
    fn build(language: &str, inputs: &'a [SceneInput], scenes: &[SceneManifest]) -> Self {
        let sentences: Vec<Vec<SentenceSpan>> = inputs
            .iter()
            .map(|input| semantic_core::split_sentences(&input.text, language, None))
            .collect();
        let mut identities: BTreeMap<String, IdentityInfo> = BTreeMap::new();
        let mut scene_mentions: Vec<Vec<NamedMention>> = vec![Vec::new(); scenes.len()];
        let mut seen: BTreeSet<(String, String, usize, usize)> = BTreeSet::new();
        for (scene_index, scene) in scenes.iter().enumerate() {
            for identity in &scene.identities {
                let info = identities
                    .entry(identity.id.clone())
                    .or_insert_with(|| IdentityInfo {
                        canonical_surface: identity.canonical_surface.clone(),
                        kind: identity.kind.clone(),
                        mentions: Vec::new(),
                    });
                if info.canonical_surface.is_empty() {
                    info.canonical_surface = identity.canonical_surface.clone();
                }
                for mention in &identity.mentions {
                    if mention.scene_id != scene.scene_id {
                        continue;
                    }
                    let key = (
                        identity.id.clone(),
                        mention.scene_id.clone(),
                        mention.start_byte,
                        mention.end_byte,
                    );
                    if !seen.insert(key) {
                        continue;
                    }
                    let sentence_index =
                        sentence_index_at(&sentences[scene_index], mention.start_byte);
                    let named = NamedMention {
                        identity_id: identity.id.clone(),
                        scene_index,
                        scene_id: scene.scene_id.clone(),
                        sentence_index,
                        start: mention.start_byte,
                        end: mention.end_byte,
                        surface: mention.surface.clone(),
                    };
                    info.mentions.push(named.clone());
                    scene_mentions[scene_index].push(named);
                }
            }
        }
        for mentions in &mut scene_mentions {
            mentions
                .sort_by_key(|mention| (mention.start, mention.end, mention.identity_id.clone()));
            mentions.dedup_by(|left, right| {
                left.identity_id == right.identity_id
                    && left.start == right.start
                    && left.end == right.end
            });
        }
        Self {
            inputs,
            sentences,
            identities,
            scene_mentions,
        }
    }

    fn is_referenceable(&self, identity_id: &str) -> bool {
        self.identities
            .get(identity_id)
            .is_some_and(|identity| is_referenceable_kind(&identity.kind))
    }

    fn compatible(&self, identity_id: &str, kind_hint: &str) -> bool {
        let Some(identity) = self.identities.get(identity_id) else {
            return false;
        };
        if !is_referenceable_kind(&identity.kind) {
            return false;
        }
        match kind_hint {
            "PERSON" => identity.kind == "PERSON",
            "ORG" => matches!(identity.kind.as_str(), "ORG" | "ORGANIZATION"),
            _ => true,
        }
    }

    fn unique_globally(&self, kind_hint: &str) -> Option<String> {
        let candidates: Vec<(&String, &IdentityInfo)> = self
            .identities
            .iter()
            .filter(|(_, identity)| !identity.mentions.is_empty())
            .filter(|(_, identity)| {
                is_referenceable_kind(&identity.kind)
                    && match kind_hint {
                        "PERSON" => identity.kind == "PERSON",
                        "ORG" => matches!(identity.kind.as_str(), "ORG" | "ORGANIZATION"),
                        _ => true,
                    }
            })
            .collect();
        (candidates.len() == 1).then(|| candidates[0].0.clone())
    }

    fn sentence(&self, scene_index: usize, sentence_index: usize) -> (String, usize, usize) {
        let text = &self.inputs[scene_index].text;
        let Some(span) = self
            .sentences
            .get(scene_index)
            .and_then(|spans| spans.get(sentence_index))
        else {
            return (text.clone(), 0, text.len());
        };
        (
            text.get(span.start_byte..span.end_byte)
                .unwrap_or(text)
                .to_string(),
            span.start_byte,
            span.end_byte,
        )
    }
}

fn explicit_reference(
    text: &str,
    context: &Context<'_>,
    mention: &NamedMention,
) -> DiscourseReference {
    let identity = context.identities.get(&mention.identity_id);
    let canonical = identity.map(|identity| identity.canonical_surface.as_str());
    let reference_type = if canonical.is_some_and(|canonical| {
        canonical.split_whitespace().count() > mention.surface.split_whitespace().count()
            && canonical
                .to_lowercase()
                .ends_with(&mention.surface.to_lowercase())
    }) {
        "surname"
    } else {
        "explicit_name"
    };
    let (evidence, sentence_start, sentence_end) =
        context.sentence(mention.scene_index, mention.sentence_index);
    DiscourseReference {
        id: String::new(),
        scene_id: mention.scene_id.clone(),
        sentence_index: mention.sentence_index,
        mention: text
            .get(mention.start..mention.end)
            .unwrap_or(&mention.surface)
            .to_string(),
        reference_type: reference_type.to_string(),
        status: "resolved".to_string(),
        target_entity_ids: vec![mention.identity_id.clone()],
        candidate_entity_ids: Vec::new(),
        resolution_basis: Some(reference_type.to_string()),
        reason: None,
        evidence,
        start_byte: mention.start,
        end_byte: mention.end,
        sentence_start_byte: sentence_start,
        sentence_end_byte: sentence_end,
    }
}

fn detected_reference(
    text: &str,
    scene_index: usize,
    context: &Context<'_>,
    detected: &DetectedReference,
    named: &[NamedMention],
) -> DiscourseReference {
    let sentence_index = sentence_index_at(&context.sentences[scene_index], detected.start);
    let (evidence, sentence_start, sentence_end) = context.sentence(scene_index, sentence_index);
    let active = active_mentions(context, scene_index, sentence_index, detected.start);
    let active_ids = distinct_ids(&active, detected.kind_hint, context);
    let scene_ids = distinct_ids(
        &context.scene_mentions[scene_index],
        detected.kind_hint,
        context,
    );
    let same_sentence = same_sentence_before(named, sentence_index, detected.start);
    let same_sentence_ids = distinct_ids(&same_sentence, detected.kind_hint, context);
    let nearest = nearest_sentence(&active);
    let nearest_ids = nearest
        .as_ref()
        .map(|(_, mentions)| distinct_ids(mentions, detected.kind_hint, context))
        .unwrap_or_default();

    let mut reference = DiscourseReference {
        id: String::new(),
        scene_id: context.inputs[scene_index].scene_id.clone(),
        sentence_index,
        mention: text
            .get(detected.start..detected.end)
            .unwrap_or(&detected.surface)
            .to_string(),
        reference_type: detected.reference_type.to_string(),
        status: "unresolved".to_string(),
        target_entity_ids: Vec::new(),
        candidate_entity_ids: Vec::new(),
        resolution_basis: None,
        reason: None,
        evidence,
        start_byte: detected.start,
        end_byte: detected.end,
        sentence_start_byte: sentence_start,
        sentence_end_byte: sentence_end,
    };

    if in_quotes(text, detected.start) {
        reference.reason = Some("quoted_speech".to_string());
        return reference;
    }

    if detected.reference_type == "pronoun" || detected.reference_type == "nominal_reference" {
        if detected.number == "plural" {
            if nearest_ids.len() == 2 {
                reference.status = "resolved".to_string();
                reference.target_entity_ids = nearest_ids;
                reference.resolution_basis = Some("plural_pair_antecedent".to_string());
            } else if nearest_ids.len() > 2 {
                reference.status = "ambiguous".to_string();
                reference.candidate_entity_ids = nearest_ids;
                reference.reason = Some("multiple_compatible_antecedents".to_string());
            }
        } else if detected.reference_type == "nominal_reference" {
            let role_ids = if scene_ids.is_empty() {
                &active_ids
            } else {
                &scene_ids
            };
            if role_ids.len() == 1 {
                reference.status = "resolved".to_string();
                reference.target_entity_ids = role_ids.clone();
                reference.resolution_basis = Some("unique_active_role_antecedent".to_string());
            } else if role_ids.len() > 1 {
                reference.status = "ambiguous".to_string();
                reference.candidate_entity_ids = role_ids.clone();
                reference.reason = Some("multiple_compatible_antecedents".to_string());
            }
        } else if same_sentence_ids.len() == 1 {
            reference.status = "resolved".to_string();
            reference.target_entity_ids = same_sentence_ids;
            reference.resolution_basis = Some("same_sentence_antecedent".to_string());
        } else if same_sentence_ids.len() > 1 {
            reference.status = "ambiguous".to_string();
            reference.candidate_entity_ids = same_sentence_ids;
            reference.reason = Some("multiple_compatible_antecedents".to_string());
        } else if nearest_ids.len() == 1 {
            reference.status = "resolved".to_string();
            reference.target_entity_ids = nearest_ids;
            reference.resolution_basis = Some("nearest_sentence_antecedent".to_string());
        } else if nearest_ids.len() > 1 {
            reference.status = "ambiguous".to_string();
            reference.candidate_entity_ids = nearest_ids;
            reference.reason = Some("multiple_compatible_antecedents".to_string());
        }
    }

    if reference.status == "unresolved" && reference.reason.is_none() {
        if let Some(identity_id) = context.unique_globally(detected.kind_hint) {
            reference.status = "resolved".to_string();
            reference.target_entity_ids = vec![identity_id];
            reference.resolution_basis = Some("globally_unique_compatible_entity".to_string());
        } else if active_ids.len() > 1 {
            reference.status = "ambiguous".to_string();
            reference.candidate_entity_ids = active_ids;
            reference.reason = Some("multiple_compatible_antecedents".to_string());
        } else {
            reference.reason = Some("no_compatible_antecedent".to_string());
        }
    }
    reference
}

fn active_mentions(
    context: &Context<'_>,
    scene_index: usize,
    sentence_index: usize,
    before: usize,
) -> Vec<NamedMention> {
    let mut output = Vec::new();
    for mention in &context.scene_mentions[scene_index] {
        let in_current = mention.sentence_index == sentence_index && mention.start < before;
        let in_recent = mention.sentence_index < sentence_index
            && sentence_index.saturating_sub(mention.sentence_index) <= 3;
        if in_current || in_recent {
            output.push(mention.clone());
        }
    }
    if output.is_empty() && sentence_index <= 1 && scene_index > 0 {
        let previous_len = context.sentences[scene_index - 1].len();
        for mention in &context.scene_mentions[scene_index - 1] {
            if previous_len <= 2 || mention.sentence_index + 2 >= previous_len {
                output.push(mention.clone());
            }
        }
    }
    output
}

fn same_sentence_before(
    named: &[NamedMention],
    sentence_index: usize,
    before: usize,
) -> Vec<NamedMention> {
    named
        .iter()
        .filter(|mention| mention.sentence_index == sentence_index && mention.start < before)
        .cloned()
        .collect()
}

fn nearest_sentence(active: &[NamedMention]) -> Option<(usize, Vec<NamedMention>)> {
    let latest = active.iter().map(|mention| mention.sentence_index).max()?;
    Some((
        latest,
        active
            .iter()
            .filter(|mention| mention.sentence_index == latest)
            .cloned()
            .collect(),
    ))
}

fn distinct_ids(mentions: &[NamedMention], kind_hint: &str, context: &Context<'_>) -> Vec<String> {
    let mut ids = Vec::new();
    for mention in mentions {
        if context.compatible(&mention.identity_id, kind_hint)
            && !ids.iter().any(|known| known == &mention.identity_id)
        {
            ids.push(mention.identity_id.clone());
        }
    }
    ids
}

fn is_referenceable_kind(kind: &str) -> bool {
    matches!(
        kind,
        "PERSON"
            | "ORG"
            | "ORGANIZATION"
            | "LOCATION"
            | "FACILITY"
            | "BRAND"
            | "PRODUCT"
            | "WORK"
            | "OBJECT"
            | "EVENT"
    )
}

fn sentence_index_at(sentences: &[SentenceSpan], offset: usize) -> usize {
    let index = sentences.partition_point(|sentence| sentence.start_byte <= offset);
    if let Some(sentence) = index
        .checked_sub(1)
        .and_then(|previous| sentences.get(previous))
    {
        if offset < sentence.end_byte {
            return index - 1;
        }
    }
    sentences
        .iter()
        .position(|sentence| offset >= sentence.start_byte && offset < sentence.end_byte)
        .unwrap_or(0)
}
