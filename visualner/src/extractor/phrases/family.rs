//! Family-relation lexicon support for relation-introduced people.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use super::super::tokenizer::Token;
use super::super::values::Candidate;

const FAMILY_RELATIONS: &str = include_str!("../../../../lexicon/family_relations.json");
static FAMILY_CUE_HEADS: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();

#[derive(serde::Deserialize)]
struct FamilyLexicon {
    languages: BTreeMap<String, Vec<FamilyCue>>,
}

#[derive(serde::Deserialize)]
struct FamilyCue {
    surface: String,
}

fn family_cue_heads() -> &'static BTreeMap<String, BTreeSet<String>> {
    FAMILY_CUE_HEADS.get_or_init(|| {
        let parsed: FamilyLexicon = serde_json::from_str(FAMILY_RELATIONS)
            .expect("family relation lexicon must be valid JSON");
        parsed
            .languages
            .into_iter()
            .map(|(language, cues)| {
                let heads = cues
                    .into_iter()
                    .filter_map(|cue| cue.surface.split_whitespace().next().map(str::to_lowercase))
                    .collect();
                (language, heads)
            })
            .collect()
    })
}

/// Type a single proper name as PERSON when a family-relation cue introduces
/// it. This relies on the relation lexicon rather than a list of given names.
pub(crate) fn type_relation_introduced_people(
    candidates: &mut [Candidate],
    tokens: &[Token],
    source: &str,
    language: &str,
) {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let Some(cues) = family_cue_heads().get(base) else {
        return;
    };
    for candidate in candidates.iter_mut().filter(|candidate| {
        candidate.entity_type.is_none() && candidate.text.split_whitespace().count() == 1
    }) {
        let Some(index) = tokens
            .iter()
            .position(|token| token.start == candidate.start && token.end == candidate.end)
        else {
            continue;
        };
        if tokens[index].upos.as_deref() != Some("PROPN") || index == 0 {
            continue;
        }
        let sentence_id = tokens[index].sentence_id;
        let from = index.saturating_sub(4);
        let introduced = (from..index).rev().any(|cue_index| {
            if tokens[cue_index].sentence_id != sentence_id {
                return false;
            }
            let gap = &source[tokens[cue_index].end..tokens[index].start];
            if gap
                .chars()
                .any(|ch| matches!(ch, '.' | ';' | ':' | '!' | '?'))
            {
                return false;
            }
            cues.contains(&tokens[cue_index].text.to_lowercase())
        });
        if introduced {
            candidate.entity_type = Some("PERSON".to_string());
        }
    }
}
