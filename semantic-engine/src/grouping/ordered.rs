//! Explicit four-step ordinal sequences inside one source sentence.
//!
//! The markers live in `lexicon/ordered_steps.json`. A group is emitted only
//! when all four positions occur once, in source order, and every member has a
//! non-empty verbatim span. Three-step prose, unordered bullets and tied
//! rankings therefore never become a sequence.

use super::super::*;

pub(crate) fn ordered_sequence_groups(
    scene_id: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
    language: &str,
) -> Vec<SemanticGroup> {
    let key = language.split(['-', '_']).next().unwrap_or("");
    let Some(markers) = lexicon::lexicon().ordered_steps.languages.get(key) else {
        return Vec::new();
    };
    if markers.len() != 4 {
        return Vec::new();
    }
    let mut groups = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let Some(text) = source.get(sentence.start_byte..sentence.end_byte) else {
            continue;
        };
        let words = semantic_core::discourse::word_spans(text);
        let mut positions = Vec::with_capacity(4);
        let mut search_from = 0usize;
        let mut complete = true;
        for alternatives in markers {
            let mut found: Option<(usize, usize)> = None;
            for marker in alternatives {
                if marker.is_empty() {
                    continue;
                }
                let mut index = search_from;
                while index + marker.len() <= words.len() {
                    if marker
                        .iter()
                        .enumerate()
                        .all(|(offset, word)| words[index + offset].lowered == *word)
                    {
                        // Alternatives may appear in either order in the
                        // source ("potem ... następnie" vs "następnie ...").
                        // The earliest match wins, not the first variant listed.
                        let candidate = (index, index + marker.len());
                        if found.is_none_or(|(start, _)| candidate.0 < start) {
                            found = Some(candidate);
                        }
                        break;
                    }
                    index += 1;
                }
            }
            let Some((start_word, end_word)) = found else {
                complete = false;
                break;
            };
            positions.push((start_word, end_word));
            search_from = end_word;
        }
        if !complete || positions.len() != 4 {
            continue;
        }
        // English may put the first marker after its step ("seal the roof
        // first, then ..."). Treat it as a postfix marker only when the words
        // between the clause boundary and the marker are substantive; a
        // lead-in such as "the order stayed fixed: first ..." keeps the normal
        // prefix reading.
        let first_postfix = if positions[0].0 == 0 {
            false
        } else {
            let marker_byte = words[positions[0].0].start;
            let clause_start = text[..marker_byte]
                .rfind([':', ';'])
                .map_or(0, |separator| separator + 1);
            text[clause_start..marker_byte]
                .chars()
                .any(|character| character.is_alphanumeric())
        };
        let mut members = Vec::with_capacity(4);
        for (position, (_, marker_end)) in positions.iter().enumerate() {
            let (raw_start, raw_end) = if position == 0 && first_postfix {
                let marker_byte = words[positions[0].0].start;
                let clause_start = text[..marker_byte]
                    .rfind([':', ';'])
                    .map_or(0, |separator| separator + 1);
                (
                    sentence.start_byte + clause_start,
                    sentence.start_byte + marker_byte,
                )
            } else {
                (
                    sentence.start_byte + words[*marker_end - 1].end,
                    positions
                        .get(position + 1)
                        .map(|(next_start, _)| sentence.start_byte + words[*next_start].start)
                        .unwrap_or(sentence.end_byte),
                )
            };
            let (start_byte, end_byte) = trim_member(source, raw_start, raw_end);
            let Some(member_text) = source.get(start_byte..end_byte) else {
                complete = false;
                break;
            };
            if start_byte >= end_byte
                || !member_text
                    .chars()
                    .any(|character| character.is_alphanumeric())
            {
                complete = false;
                break;
            }
            let rank = position + 1;
            members.push(SemanticGroupMember {
                candidate_id: format!("ordered_step_{rank:02}"),
                scene_id: scene_id.to_string(),
                rank: Some(rank),
                sentence_index,
                text: member_text.to_string(),
                evidence: member_text.to_string(),
                start_byte,
                end_byte,
                temporal_surface: None,
                normalized_date: None,
            });
        }
        if !complete || members.len() != 4 {
            continue;
        }
        groups.push(SemanticGroup {
            id: format!("ordered_sequence_{:02}", groups.len() + 1),
            group_type: "step_sequence".to_string(),
            title: "Sequenza ordinata".to_string(),
            ranking_basis: Some("explicit_source_order".to_string()),
            target_count: Some(4),
            complete: Some(true),
            members,
        });
    }
    groups
}

/// Removes the separators around a member without changing the source bytes
/// the span points at: the evidence stays a verbatim slice.
fn trim_member(source: &str, mut start: usize, mut end: usize) -> (usize, usize) {
    while start < end {
        let Some(character) = source[start..end].chars().next() else {
            break;
        };
        if character.is_whitespace() || matches!(character, ',' | ';' | ':' | '—' | '-') {
            start += character.len_utf8();
        } else {
            break;
        }
    }
    while end > start {
        let Some(character) = source[start..end].chars().next_back() else {
            break;
        };
        if character.is_whitespace() || matches!(character, ',' | ';' | ':' | '.' | '!' | '?') {
            end -= character.len_utf8();
        } else {
            break;
        }
    }
    (start, end)
}
