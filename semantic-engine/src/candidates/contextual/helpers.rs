use super::*;

pub(super) fn visual_slot_rank(entity_type: &str) -> u8 {
    match entity_type {
        "PRODUCT" => 0,
        "OBJECT" => 1,
        "FACILITY" | "STRUCTURE" => 2,
        "BRAND" | "ORG" | "ORGANIZATION" => 3,
        "PERSON" => 4,
        "LOCATION" => 5,
        _ => 6,
    }
}

pub(super) fn visual_component_kind(entity_type: &str) -> &'static str {
    match entity_type {
        "PERSON" => "entity",
        "LOCATION" => "location",
        "ORG" | "ORGANIZATION" | "BRAND" => "related_entity",
        "FACILITY" | "STRUCTURE" => "facility",
        _ => "object",
    }
}

pub(super) fn closest_related_brand<'a>(
    object: &VisualEntity,
    brands: &[&'a VisualEntity],
) -> Option<&'a VisualEntity> {
    brands
        .iter()
        .copied()
        .min_by_key(|brand| object.start.abs_diff(brand.start))
}

pub(super) fn is_passive_visual_owner(clause: &str, entity: &VisualEntity) -> bool {
    let Some(relative) = clause.find(entity.text.as_str()) else {
        return false;
    };
    let prefix = clause[..relative].to_lowercase();
    crate::consolidate::image_query_policy()
        .passive_owner_cues
        .iter()
        .any(|cue| prefix.contains(cue.to_lowercase().as_str()))
}

pub(super) fn explicit_location_link(
    source: &str,
    subject: &VisualEntity,
    location: &VisualEntity,
) -> bool {
    let policy = crate::consolidate::image_query_policy();
    if subject.end <= location.start {
        let Some(gap) = source.get(subject.end..location.start) else {
            return false;
        };
        let lower = gap.to_lowercase();
        let padded_gap = format!(" {} ", lower.trim());
        return policy
            .location_anchor_cues
            .iter()
            .any(|cue| padded_gap.contains(&format!(" {} ", cue.trim().to_lowercase())));
    }
    if location.end <= subject.start {
        let Some(gap) = source.get(location.end..subject.start) else {
            return false;
        };
        let lower = gap.to_lowercase();
        let padded_gap = format!(" {} ", lower.trim());
        return policy
            .location_preceding_subject_cues
            .iter()
            .any(|cue| padded_gap.contains(&format!(" {} ", cue.trim().to_lowercase())));
    }
    false
}

pub(super) fn is_explicit_visual_comparison(clause: &str, _language: &str) -> bool {
    let lower = clause.to_lowercase();
    crate::consolidate::image_query_policy()
        .comparison_cues
        .iter()
        .any(|cue| lower.contains(cue.to_lowercase().as_str()))
}

pub(super) fn has_explicit_link_disclaimer(clause: &str, _language: &str) -> bool {
    let lower = clause.to_lowercase();
    crate::consolidate::image_query_policy()
        .link_disclaimer_cues
        .iter()
        .any(|cue| lower.contains(cue.to_lowercase().as_str()))
}

pub(super) fn visual_clause_spans(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_lowercase();
    let separators = &crate::consolidate::image_query_policy().visual_clause_separators;
    let mut cuts = Vec::new();
    for separator in separators {
        for (start, matched) in lower.match_indices(separator.as_str()) {
            let end = start + matched.len();
            let left_boundary = start == 0
                || lower[..start]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
            let right_boundary =
                end == lower.len() || lower[end..].chars().next().is_some_and(char::is_whitespace);
            if left_boundary && right_boundary {
                cuts.push((start, end));
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut spans = Vec::new();
    let mut start = 0;
    for (cut_start, cut_end) in cuts {
        if start < cut_start {
            let raw = &text[start..cut_start];
            let left = raw.len()
                - raw
                    .trim_start_matches(|ch: char| ch.is_whitespace() || ch == ',')
                    .len();
            let right = raw
                .trim_end_matches(|ch: char| ch.is_whitespace() || ch == ',')
                .len();
            if left < right {
                spans.push((start + left, start + right));
            }
        }
        start = cut_end;
    }
    if start < text.len() {
        let raw = &text[start..];
        let left = raw.len()
            - raw
                .trim_start_matches(|ch: char| ch.is_whitespace() || ch == ',')
                .len();
        let right = raw
            .trim_end_matches(|ch: char| ch.is_whitespace() || ch == ',')
            .len();
        if left < right {
            spans.push((start + left, start + right));
        }
    }
    if spans.is_empty() {
        spans.push((0, text.len()));
    }
    spans
}

pub(super) fn visual_action_component(
    source: &str,
    start: usize,
    end: usize,
    clause: &str,
) -> Option<semantic_core::QueryComponent> {
    let lower = clause.to_lowercase();
    let policy = crate::consolidate::image_query_policy();
    // A lexical cue can describe a document action rather than an imageable
    // event (for example, "ricorso presentato in Cassazione"). In that case
    // retain the entities as independent candidates without adding a visual
    // action to their composed query.
    if policy
        .nonvisual_action_context_phrases
        .iter()
        .any(|phrase| lower.contains(&phrase.to_lowercase()))
    {
        return None;
    }
    // Policy order ranks useful visual frames above incidental verbs. If two
    // cues start at the same position, keep the longest configured phrase so
    // "annullando con rinvio" stays intact in both query and evidence.
    let mut selected_cue: Option<(&str, usize)> = None;
    for cue in &policy.visual_action_cues {
        let Some(offset) = lower.find(&cue.to_lowercase()) else {
            continue;
        };
        match selected_cue {
            None => selected_cue = Some((cue, offset)),
            Some((selected, selected_offset))
                if offset == selected_offset && cue.len() > selected.len() =>
            {
                selected_cue = Some((cue, offset));
            }
            _ => {}
        }
    }
    let cue = selected_cue?;
    let absolute_start = start + cue.1;
    let absolute_end = absolute_start + cue.0.len();
    if absolute_end > end {
        return None;
    }
    let text = source.get(absolute_start..absolute_end)?.to_string();
    Some(semantic_core::QueryComponent {
        kind: "action".to_string(),
        component_type: "ACTION".to_string(),
        text,
        start_byte: absolute_start,
        end_byte: absolute_end,
    })
}

pub(crate) fn is_visual_query_entity(entity_type: &str) -> bool {
    matches!(
        entity_type,
        "PERSON"
            | "LOCATION"
            | "ORG"
            | "ORGANIZATION"
            | "FACILITY"
            | "BRAND"
            | "PRODUCT"
            | "OBJECT"
            | "WORK"
            | "STRUCTURE"
    )
}

pub(crate) fn is_image_component_type(entity_type: &str) -> bool {
    is_visual_query_entity(entity_type)
        || matches!(entity_type, "DATE" | "NUMBER" | "PERCENT" | "MONEY")
}
