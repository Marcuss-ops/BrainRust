use super::*;

/// Uses the semantic quantity attached to this exact numeric span when one is
/// available. The legacy chart parser remains a compatibility fallback for
/// chart points that have no recognized typed unit.
pub(super) fn chart_measure(
    sentence: &str,
    start: usize,
    end: usize,
    point: &ChartPoint,
    quantities: &[Quantity],
) -> crate::chart_rules::ChartUnit {
    if let Some(quantity) = quantities
        .iter()
        .find(|quantity| quantity.start_byte == point.start && quantity.end_byte == point.end)
    {
        if let (Some(raw_value), Some(_)) = (quantity.raw_value, quantity.value) {
            let scaled_value = raw_value * quantity.scale.unwrap_or(1.0);
            let kind = match quantity.dimension.as_str() {
                "percent" => "PERCENT",
                "currency" => "MONEY",
                _ => point.kind.as_str(),
            };
            let display_measure =
                crate::chart_rules::chart_unit(sentence, start, end, scaled_value, kind);
            let signature = match quantity.dimension.as_str() {
                "length" => "length:m".to_string(),
                "currency" => {
                    let period = display_measure
                        .signature
                        .rsplit_once('/')
                        .map_or("unit", |(_, period)| period);
                    format!("currency:{}/{period}", quantity.unit)
                }
                _ => format!("{}:{}", quantity.dimension, quantity.unit),
            };
            return crate::chart_rules::ChartUnit {
                kind: kind.to_string(),
                signature,
                value: display_measure.value,
            };
        }
    }
    crate::chart_rules::chart_unit(sentence, start, end, point.value, &point.kind)
}

/// Joins adjacent source-grounded measures when the script repeats a metric
/// across sentences (for example, annual revenue or output). Consecutive
/// points with the same metric and unit form ONE ordered series of 2..=6
/// items; the emitted evidence is one exact source slice spanning all included
/// sentences. A grounded `Quantity` predicate and unit take precedence over the
/// lexical stem fallback.
#[allow(clippy::too_many_arguments)]
pub(super) fn cross_sentence_series(
    source: &str,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    chart_points: &[ChartPoint],
    quantities: &[Quantity],
    identities: &IdentityLookup,
    language: &str,
    out: &mut Vec<VisualCandidate>,
) {
    let mut ordered: Vec<&ChartPoint> = chart_points.iter().collect();
    ordered.sort_by_key(|point| (point.start, point.end));
    let mut used = vec![false; ordered.len()];
    for start_index in 0..ordered.len() {
        if used[start_index] {
            continue;
        }
        let mut chain = vec![start_index];
        let mut cursor = start_index;
        while chain.len() < 6 {
            let Some(next) =
                next_series_point(&ordered, &used, cursor, bullets, quantities, language)
            else {
                break;
            };
            chain.push(next);
            cursor = next;
        }
        if chain.len() < 2 {
            continue;
        }
        let Some(candidate) = build_series_candidate(
            source, entities, bullets, &ordered, &chain, quantities, identities, language,
        ) else {
            continue;
        };
        // A single sentence-local chart already covering these exact values
        // wins; do not emit a duplicate scenario-spanning copy.
        if out.iter().any(|existing| {
            existing.kind == "chart"
                && chain.iter().all(|index| {
                    let point = ordered[*index];
                    existing
                        .chart_items
                        .iter()
                        .any(|item| item.start_byte == point.start && item.end_byte == point.end)
                })
        }) {
            continue;
        }
        for index in &chain {
            used[*index] = true;
        }
        out.push(candidate);
    }
}

/// Finds the next ordered point that extends the current series. Gap, metric,
/// unit and time-anchor checks are identical to the historical pair rule; the
/// difference is that the chain is kept together instead of fragmenting into
/// overlapping pairs.
pub(super) fn next_series_point(
    ordered: &[&ChartPoint],
    used: &[bool],
    from: usize,
    bullets: &[Bullet],
    quantities: &[Quantity],
    language: &str,
) -> Option<usize> {
    let left = ordered[from];
    let left_bullet = bullets
        .iter()
        .find(|bullet| bullet.sentence_index == left.sentence_index)?;
    for index in from + 1..ordered.len() {
        if used[index] {
            continue;
        }
        let right = ordered[index];
        if right.sentence_index <= left.sentence_index {
            continue;
        }
        if right.sentence_index.saturating_sub(left.sentence_index) > 3 {
            break;
        }
        let Some(right_bullet) = bullets
            .iter()
            .find(|bullet| bullet.sentence_index == right.sentence_index)
        else {
            continue;
        };
        if !crate::chart_rules::chart_point_is_eligible(
            &left_bullet.text,
            left.start.saturating_sub(left_bullet.start),
            left.end.saturating_sub(left_bullet.start),
        ) || !crate::chart_rules::chart_point_is_eligible(
            &right_bullet.text,
            right.start.saturating_sub(right_bullet.start),
            right.end.saturating_sub(right_bullet.start),
        ) {
            continue;
        }
        if !series_time_anchor(&left_bullet.text, &right_bullet.text) {
            continue;
        }
        let (Some(left_start), Some(left_end), Some(right_start), Some(right_end)) = (
            left.start.checked_sub(left_bullet.start),
            left.end.checked_sub(left_bullet.start),
            right.start.checked_sub(right_bullet.start),
            right.end.checked_sub(right_bullet.start),
        ) else {
            continue;
        };
        let left_measure = chart_measure(&left_bullet.text, left_start, left_end, left, quantities);
        let right_measure = chart_measure(
            &right_bullet.text,
            right_start,
            right_end,
            right,
            quantities,
        );
        if !compatible_measures(&left_measure, &right_measure) {
            continue;
        }
        if series_metric_match(
            &left_bullet.text,
            &right_bullet.text,
            quantities,
            left,
            right,
            language,
        )
        .is_none()
        {
            continue;
        }
        return Some(index);
    }
    None
}

pub(super) fn compatible_measures(
    left: &crate::chart_rules::ChartUnit,
    right: &crate::chart_rules::ChartUnit,
) -> bool {
    if left.signature == right.signature && left.kind == right.kind {
        return true;
    }
    let fallback = |bare: &crate::chart_rules::ChartUnit,
                    explicit: &crate::chart_rules::ChartUnit| {
        bare.signature == bare.kind && explicit.signature != explicit.kind
    };
    (fallback(left, right) || fallback(right, left))
        && (left.kind == right.kind
            || left.signature == right.signature
            || right.signature == right.kind
            || left.signature == left.kind)
}

pub(super) fn series_time_anchor(left: &str, right: &str) -> bool {
    if !has_four_digit_year(left) {
        return false;
    }
    let lower = right.to_lowercase();
    has_four_digit_year(right) || lower.contains("seguente") || lower.contains("following year")
}

/// Resolves the repeated metric for a series. Exact grounded quantities win:
/// two values with the same dimension/unit and the same explicit predicate are
/// the same metric even when the surface wording differs. When the predicate is
/// missing, the language stem table is the conservative fallback.
pub(super) fn series_metric_match(
    left_text: &str,
    right_text: &str,
    quantities: &[Quantity],
    left: &ChartPoint,
    right: &ChartPoint,
    language: &str,
) -> Option<String> {
    let exact = |point: &ChartPoint| {
        quantities
            .iter()
            .find(|quantity| quantity.start_byte == point.start && quantity.end_byte == point.end)
    };
    if let (Some(left_quantity), Some(right_quantity)) = (exact(left), exact(right)) {
        if left_quantity.dimension == right_quantity.dimension
            && left_quantity.unit == right_quantity.unit
        {
            match (&left_quantity.predicate, &right_quantity.predicate) {
                (Some(a), Some(b)) if a == b => return Some(a.clone()),
                (Some(_), Some(_)) => return None,
                _ => {}
            }
        }
    }
    let left_lower = left_text.to_lowercase();
    let right_lower = right_text.to_lowercase();
    semantic_core::chart_cues::language_cues(language)
        .series_metric_stems
        .iter()
        .find(|stem| left_lower.contains(stem.as_str()) && right_lower.contains(stem.as_str()))
        .cloned()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_series_candidate(
    source: &str,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    ordered: &[&ChartPoint],
    chain: &[usize],
    quantities: &[Quantity],
    identities: &IdentityLookup,
    language: &str,
) -> Option<VisualCandidate> {
    let points: Vec<&ChartPoint> = chain.iter().map(|index| ordered[*index]).collect();
    let first = *points.first()?;
    let last = *points.last()?;
    let first_bullet = bullets
        .iter()
        .find(|bullet| bullet.sentence_index == first.sentence_index)?;
    let last_bullet = bullets
        .iter()
        .find(|bullet| bullet.sentence_index == last.sentence_index)?;
    let (evidence_start, _) = chart_claim_bounds(first_bullet, &[first]);
    let (_, evidence_end) = chart_claim_bounds(last_bullet, &[last]);
    let evidence = source.get(evidence_start..evidence_end)?;

    let mut measures: Vec<crate::chart_rules::ChartUnit> = Vec::with_capacity(points.len());
    for point in &points {
        let bullet = bullets
            .iter()
            .find(|bullet| bullet.sentence_index == point.sentence_index)?;
        let start = point.start.checked_sub(bullet.start)?;
        let end = point.end.checked_sub(bullet.start)?;
        measures.push(chart_measure(&bullet.text, start, end, point, quantities));
    }
    // A bare number beside an explicit unit ("120 cars" then "180") inherits
    // the explicit unit, exactly like the sentence-local chart path.
    inherit_explicit_units(&mut measures);
    if measures
        .windows(2)
        .any(|pair| !compatible_measures(&pair[0], &pair[1]))
    {
        return None;
    }
    let mut items = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let bullet = bullets
            .iter()
            .find(|bullet| bullet.sentence_index == point.sentence_index)?;
        let start = point.start.checked_sub(bullet.start)?;
        let end = point.end.checked_sub(bullet.start)?;
        let measure = &measures[index];
        let (display_start, display_end) =
            crate::chart_rules::qualified_numeric_span(&bullet.text, start, end);
        let text = bullet.text.get(display_start..display_end)?;
        // A cross-sentence annual series names each year once, so the single
        // year in the sentence is the correct category for that point.
        let label = single_year_label(&bullet.text).unwrap_or_else(|| {
            chart_subject_label(source, entities, point, bullet, bullet.start, bullet.end)
        });
        let quantity =
            chart_point_quantity(quantities, point.sentence_index, point.start, point.end);
        items.push(ChartItem {
            label,
            text: text.to_string(),
            kind: measure.kind.clone(),
            value: measure.value,
            start_byte: bullet.start + display_start,
            end_byte: bullet.start + display_end,
            qualifier: quantity
                .map(|quantity| quantity.qualifier.clone())
                .filter(|value| !value.is_empty()),
            period: quantity
                .and_then(|quantity| quantity.period.as_ref())
                .map(|period| period.year.to_string()),
            lower: None,
            upper: None,
            range_id: quantity.and_then(|quantity| quantity.range_id.clone()),
        });
    }
    if items.len() < 2 {
        return None;
    }
    let mut indices: Vec<usize> = points.iter().filter_map(|point| point.entity).collect();
    indices.sort_unstable();
    indices.dedup();
    let chart_meta = chart_metadata(&items, &measures, quantities, &points);
    let suggested_layout = select_layout(&items, evidence, language);
    Some(VisualCandidate {
        kind: "chart",
        sentence_index: first.sentence_index.min(last.sentence_index),
        entity_ids: ids_of(identities, &indices),
        entity_texts: texts_of(entities, &indices),
        entity_indices: indices,
        chart_items: items,
        chart_meta,
        temporal_surfaces: Vec::new(),
        suggested_layout: Some(suggested_layout),
        requires_verified_images: false,
        evidence: evidence.to_string(),
        start: evidence_start,
        end: evidence_end,
    })
}
