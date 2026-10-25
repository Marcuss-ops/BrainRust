//! Explicit numbered enumerations and rankings with Top 10 completeness.

use super::super::*;

/// Recognizes only explicit, consecutively numbered source lines. A declared
/// Top 10 is complete only when all ten distinct ranks are present; no items
/// are generated to fill a gap.
pub(crate) fn numbered_groups(
    scene_id: &str,
    source: &str,
    sentences: &[semantic_core::SentenceSpan],
) -> Vec<SemanticGroup> {
    let mut target_count =
        if source.to_lowercase().contains("top 10") || source.to_lowercase().contains("top ten") {
            Some(10usize)
        } else {
            None
        };
    // Items may be one per line OR an inline list in a single narration
    // paragraph ("1. ... 2. ..."). The scanner therefore walks the whole
    // source instead of splitting on newlines only, and each item runs from
    // its own marker to the next marker.
    let mut markers = Vec::<(usize, usize, usize)>::new();
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let at_boundary = index == 0
            || bytes[index - 1].is_ascii_whitespace()
            || matches!(bytes[index - 1], b'>' | b'(' | b'\n');
        if !at_boundary {
            index += 1;
            continue;
        }
        let mut digits_end = index;
        while digits_end < bytes.len()
            && bytes[digits_end].is_ascii_digit()
            && digits_end - index < 2
        {
            digits_end += 1;
        }
        let delimiter = bytes.get(digits_end).copied();
        let after = digits_end + 1;
        let valid = digits_end > index
            && matches!(delimiter, Some(b'.' | b')'))
            && bytes.get(after).is_some_and(u8::is_ascii_whitespace)
            && source
                .get(after..)
                .and_then(|tail| tail.trim_start().chars().next())
                .is_some_and(|first| !first.is_whitespace());
        if !valid {
            index = after.max(index + 1);
            continue;
        }
        if let Ok(rank) = source[index..digits_end].parse::<usize>() {
            let text_start = source[after..]
                .find(|ch: char| !ch.is_whitespace())
                .map(|offset| after + offset)
                .unwrap_or(after);
            if rank > 0 {
                markers.push((rank, index, text_start));
            }
        }
        index = after;
    }

    let mut candidates = Vec::<(usize, usize, usize, String)>::new();
    for (position, (rank, start, _)) in markers.iter().enumerate() {
        let end = markers
            .get(position + 1)
            .map(|(_, next_start, _)| *next_start)
            .unwrap_or_else(|| {
                // The last item ends with its own sentence, so trailing
                // narration is not swallowed into the list.
                sentences
                    .iter()
                    .find(|sentence| *start >= sentence.start_byte && *start < sentence.end_byte)
                    .map(|sentence| sentence.end_byte)
                    .unwrap_or(bytes.len())
            });
        let evidence = source[*start..end].trim_end().to_string();
        if evidence.is_empty() {
            continue;
        }
        candidates.push((*rank, *start, *start + evidence.len(), evidence));
    }
    if candidates.len() < 2 {
        return Vec::new();
    }
    let ascending = candidates.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1);
    let descending = candidates.windows(2).all(|pair| pair[0].0 == pair[1].0 + 1);
    if !ascending && !descending {
        return Vec::new();
    }
    if target_count.is_some_and(|target| candidates.len() > target) {
        return Vec::new();
    }
    // Ten consecutive explicit ranks are a complete Top 10 even when the
    // narration never spells out the words "Top 10". Fewer items stay an
    // enumeration: completeness is unknown, so nothing is invented.
    if target_count.is_none() && candidates.len() == 10 {
        target_count = Some(10);
    }
    let mut members = Vec::with_capacity(candidates.len());
    for (rank, start, end, evidence) in &candidates {
        if check_span(source, *start, *end, evidence) != Grounding::Grounded {
            return Vec::new();
        }
        let sentence_index = sentences
            .iter()
            .position(|sentence| *start >= sentence.start_byte && *start < sentence.end_byte)
            .unwrap_or(members.len());
        members.push(SemanticGroupMember {
            candidate_id: format!("ranked_item_{rank:02}"),
            scene_id: scene_id.to_string(),
            rank: Some(*rank),
            sentence_index,
            text: evidence.clone(),
            evidence: evidence.clone(),
            start_byte: *start,
            end_byte: *end,
            temporal_surface: None,
            normalized_date: None,
        });
    }
    let complete = target_count.map(|target| {
        candidates.len() == target
            && (1..=target).all(|rank| candidates.iter().any(|item| item.0 == rank))
    });
    vec![SemanticGroup {
        id: "numbered_list_01".to_string(),
        group_type: if target_count.is_some() {
            "ranking"
        } else {
            "enumeration"
        }
        .to_string(),
        title: if target_count.is_some() {
            "Top 10"
        } else {
            "Elenco numerato"
        }
        .to_string(),
        ranking_basis: Some("explicit_source_order".to_string()),
        target_count,
        complete,
        members,
    }]
}
