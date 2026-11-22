//! Chart proposal construction and source-grounded chart helpers.

use super::*;

pub(super) fn build(
    source: &str,
    language: &str,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    chart_input: ChartInput<'_>,
    identities: &IdentityLookup,
    out: &mut Vec<VisualCandidate>,
) {
    let chart_points = chart_input.points;
    let quantities = chart_input.quantities;
    let mut sentences: Vec<usize> = chart_points.iter().map(|p| p.sentence_index).collect();
    sentences.sort_unstable();
    sentences.dedup();
    for sentence in sentences {
        let Some(bullet) = bullets.iter().find(|b| b.sentence_index == sentence) else {
            continue;
        };
        let points: Vec<&ChartPoint> = chart_points
            .iter()
            .filter(|p| p.sentence_index == sentence)
            .filter(|p| {
                let start = p.start.saturating_sub(bullet.start);
                let end = p.end.saturating_sub(bullet.start);
                crate::chart_rules::chart_point_is_eligible(&bullet.text, start, end)
            })
            .collect();
        if points.len() < 2 {
            continue;
        }
        let mut measures: Vec<crate::chart_rules::ChartUnit> = points
            .iter()
            .filter_map(|point| {
                let relative_start = point.start.checked_sub(bullet.start)?;
                let relative_end = point.end.checked_sub(bullet.start)?;
                Some(chart_measure(
                    &bullet.text,
                    relative_start,
                    relative_end,
                    point,
                    quantities,
                ))
            })
            .collect();
        let explicit_units: std::collections::BTreeSet<(String, String)> = measures
            .iter()
            .filter(|measure| measure.signature != measure.kind)
            .map(|measure| (measure.signature.clone(), measure.kind.clone()))
            .collect();
        if explicit_units.len() == 1 {
            let (signature, kind) = explicit_units.iter().next().unwrap();
            for measure in &mut measures {
                if measure.signature == measure.kind {
                    measure.signature = signature.clone();
                    measure.kind = kind.clone();
                }
            }
        }
        if measures.len() != points.len()
            || measures
                .windows(2)
                .any(|pair| pair[0].signature != pair[1].signature || pair[0].kind != pair[1].kind)
        {
            continue;
        }
        let items: Vec<ChartItem> = points
            .iter()
            .zip(&measures)
            .enumerate()
            .filter_map(|(index, (p, measure))| {
                let relative_start = p.start.checked_sub(bullet.start)?;
                let relative_end = p.end.checked_sub(bullet.start)?;
                let (qualified_start, qualified_end) = crate::chart_rules::qualified_numeric_span(
                    &bullet.text,
                    relative_start,
                    relative_end,
                );
                let text = bullet.text.get(qualified_start..qualified_end)?;
                let subject_start = if index == 0 {
                    bullet.start
                } else {
                    points[index - 1].end + (p.start.saturating_sub(points[index - 1].end) / 2)
                };
                let subject_end = points.get(index + 1).map_or(bullet.end, |next| {
                    p.end + (next.start.saturating_sub(p.end) / 2)
                });
                Some(ChartItem {
                    label: chart_subject_label(
                        source,
                        entities,
                        p,
                        bullet,
                        subject_start,
                        subject_end,
                    ),
                    text: text.to_string(),
                    kind: measure.kind.clone(),
                    value: measure.value,
                    start_byte: bullet.start + qualified_start,
                    end_byte: bullet.start + qualified_end,
                })
            })
            .collect();
        // A chart needs at least two comparable values; one number is not a
        // comparison, and mixed kinds (a year beside a percentage) are not
        // comparable either.
        if items.len() < 2 || !all_same_kind(&items) {
            continue;
        }
        // Equal units and a shared sentence are not enough to prove that two
        // values belong in one chart. Require either an explicit comparison
        // cue or the conservative repeated-series context checked below.
        if !comparison_cue_binds_values(bullet, chart_points, sentence, language)
            && !crate::chart_rules::has_series_context(&bullet.text, items.len(), language)
        {
            continue;
        }
        let mut indices: Vec<usize> = chart_points
            .iter()
            .filter(|p| p.sentence_index == sentence)
            .filter_map(|p| p.entity)
            .collect();
        indices.sort_unstable();
        indices.dedup();
        let chart_meta = chart_metadata(&items, &measures, quantities, &points);
        let evidence = chart_claim_evidence(source, bullet, &points);
        let suggested_layout = select_layout(&items, &evidence, language);
        let (start, end) = chart_claim_bounds(bullet, &points);
        out.push(VisualCandidate {
            kind: "chart",
            sentence_index: sentence,
            entity_ids: ids_of(identities, &indices),
            entity_texts: texts_of(entities, &indices),
            entity_indices: indices,
            chart_items: items,
            chart_meta,
            temporal_surfaces: Vec::new(),
            suggested_layout: Some(suggested_layout),
            requires_verified_images: false,
            evidence,
            start,
            end,
        });
    }
    comparison_deltas(source, quantities, chart_input.comparisons, out);
    cross_sentence_series(
        source,
        entities,
        bullets,
        chart_points,
        quantities,
        identities,
        language,
        out,
    );
}

/// Bridges grounded `QuantityComparison` records into delta/before-after
/// proposals. A comparison is emitted only when both linked quantities carry
/// values, dimensions and units that already agree; the evidence span is the
/// verbatim source slice covering both values and the comparison cue.
fn comparison_deltas(
    source: &str,
    quantities: &[Quantity],
    comparisons: &[semantic_core::QuantityComparison],
    out: &mut Vec<VisualCandidate>,
) {
    let mut seen = std::collections::BTreeSet::new();
    for comparison in comparisons {
        let Some(left) = quantities
            .iter()
            .find(|quantity| quantity.id == comparison.left)
        else {
            continue;
        };
        let Some(right) = quantities
            .iter()
            .find(|quantity| quantity.id == comparison.right)
        else {
            continue;
        };
        let (Some(left_value), Some(right_value)) = (left.value, right.value) else {
            continue;
        };
        if left.dimension != right.dimension || left.unit != right.unit {
            continue;
        }
        let start = left.start_byte.min(comparison.evidence_start_byte);
        let end = right
            .end_byte
            .max(comparison.evidence_end_byte)
            .max(left.end_byte);
        if start >= end || end > source.len() {
            continue;
        }
        let key = format!("{}:{}:{}:{}", comparison.kind, start, end, left.id);
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(evidence) = source.get(start..end) else {
            continue;
        };
        let item = |quantity: &Quantity, value: f64| ChartItem {
            label: quantity
                .predicate
                .clone()
                .unwrap_or_else(|| quantity.evidence.clone()),
            text: quantity.evidence.clone(),
            kind: match quantity.dimension.as_str() {
                "percent" | "growth_rate" => "PERCENT",
                "currency" => "MONEY",
                _ => "NUMBER",
            }
            .to_string(),
            value,
            start_byte: quantity.start_byte,
            end_byte: quantity.end_byte,
        };
        let mut entity_ids: Vec<String> = [&left.subject_entity_id, &right.subject_entity_id]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        entity_ids.sort();
        entity_ids.dedup();
        let mut chart_meta = semantic_core::ChartMetadata {
            metric: left.predicate.clone().filter(|metric| {
                right
                    .predicate
                    .as_deref()
                    .is_some_and(|right_metric| right_metric == metric)
            }),
            unit: Some(left.unit.clone()).filter(|unit| !unit.is_empty()),
            dimension: Some(left.dimension.clone()),
            period: None,
            series_type: Some("category".to_string()),
        };
        if comparison.kind == "change" {
            if let (Some(left_period), Some(right_period)) = (&left.period, &right.period) {
                chart_meta.period =
                    Some(format!("{}→{}", left_period.year, right_period.year));
            }
        }
        out.push(VisualCandidate {
            kind: "delta_comparison",
            sentence_index: left.sentence_index.min(right.sentence_index),
            entity_indices: Vec::new(),
            entity_ids,
            entity_texts: Vec::new(),
            chart_items: vec![item(left, left_value), item(right, right_value)],
            chart_meta: Some(chart_meta),
            temporal_surfaces: Vec::new(),
            suggested_layout: Some(if comparison.kind == "change" {
                "before_after"
            } else {
                "delta_comparison"
            }),
            requires_verified_images: false,
            evidence: evidence.to_string(),
            start,
            end,
        });
    }
}

/// Uses the semantic quantity attached to this exact numeric span when one is
/// available. The legacy chart parser remains a compatibility fallback for
/// chart points that have no recognized typed unit.
fn chart_measure(
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
fn cross_sentence_series(
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
            source,
            entities,
            bullets,
            &ordered,
            &chain,
            quantities,
            identities,
            language,
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
fn next_series_point(
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
        let right_measure =
            chart_measure(&right_bullet.text, right_start, right_end, right, quantities);
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

fn compatible_measures(
    left: &crate::chart_rules::ChartUnit,
    right: &crate::chart_rules::ChartUnit,
) -> bool {
    if left.signature == right.signature && left.kind == right.kind {
        return true;
    }
    let fallback = |bare: &crate::chart_rules::ChartUnit, explicit: &crate::chart_rules::ChartUnit| {
        bare.signature == bare.kind && explicit.signature != explicit.kind
    };
    (fallback(left, right) || fallback(right, left))
        && (left.kind == right.kind
            || left.signature == right.signature
            || right.signature == right.kind
            || left.signature == left.kind)
}

fn series_time_anchor(left: &str, right: &str) -> bool {
    if !has_four_digit_year(left) {
        return false;
    }
    let lower = right.to_lowercase();
    has_four_digit_year(right)
        || lower.contains("seguente")
        || lower.contains("following year")
}

/// Resolves the repeated metric for a series. Exact grounded quantities win:
/// two values with the same dimension/unit and the same explicit predicate are
/// the same metric even when the surface wording differs. When the predicate is
/// missing, the language stem table is the conservative fallback.
fn series_metric_match(
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
fn build_series_candidate(
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
    let explicit_units: std::collections::BTreeSet<(String, String)> = measures
        .iter()
        .filter(|measure| measure.signature != measure.kind)
        .map(|measure| (measure.signature.clone(), measure.kind.clone()))
        .collect();
    if explicit_units.len() == 1 {
        let (signature, kind) = explicit_units.iter().next().unwrap();
        for measure in &mut measures {
            if measure.signature == measure.kind {
                measure.signature = signature.clone();
                measure.kind = kind.clone();
            }
        }
    }
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
        let label = single_year_label(&bullet.text)
            .unwrap_or_else(|| chart_subject_label(source, entities, point, bullet, bullet.start, bullet.end));
        items.push(ChartItem {
            label,
            text: text.to_string(),
            kind: measure.kind.clone(),
            value: measure.value,
            start_byte: bullet.start + display_start,
            end_byte: bullet.start + display_end,
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

/// Picks the layout the grounded values support. The default stays
/// `bar_comparison`; `line_series` needs ordered year labels; `donut_share`
/// needs percentage parts whose sum is a complete share (or an explicit share
/// cue in the evidence).
fn select_layout(items: &[ChartItem], evidence: &str, language: &str) -> &'static str {
    if labels_are_ordered_years(items) {
        return "line_series";
    }
    if (2..=6).contains(&items.len()) && items.iter().all(|item| item.kind == "PERCENT") {
        let sum: f64 = items.iter().map(|item| item.value).sum();
        let share_cue = {
            let lower = evidence.to_lowercase();
            semantic_core::chart_cues::language_cues(language)
                .share_context_cues
                .iter()
                .any(|cue| lower.contains(&cue.to_lowercase()))
        };
        if (95.0..=100.5).contains(&sum) || (sum > 0.0 && sum <= 100.0 && share_cue) {
            return "donut_share";
        }
    }
    "bar_comparison"
}

fn labels_are_ordered_years(items: &[ChartItem]) -> bool {
    if items.len() < 2 {
        return false;
    }
    let years: Option<Vec<i32>> = items
        .iter()
        .map(|item| {
            let label = item.label.trim();
            (label.len() == 4)
                .then(|| label.parse::<i32>().ok())
                .flatten()
                .filter(|year| (1900..=2099).contains(year))
        })
        .collect();
    let Some(years) = years else {
        return false;
    };
    years.windows(2).all(|pair| pair[1] > pair[0])
        || years.windows(2).all(|pair| pair[1] < pair[0])
}

fn chart_metadata(
    items: &[ChartItem],
    measures: &[crate::chart_rules::ChartUnit],
    quantities: &[Quantity],
    points: &[&ChartPoint],
) -> Option<semantic_core::ChartMetadata> {
    if items.len() < 2 {
        return None;
    }
    let matched: Vec<&Quantity> = points
        .iter()
        .filter_map(|point| {
            quantities
                .iter()
                .find(|quantity| {
                    quantity.start_byte == point.start && quantity.end_byte == point.end
                })
        })
        .collect();
    let shared = |values: Vec<Option<String>>| {
        let mut iterator = values.into_iter();
        let first = iterator.next().flatten()?;
        iterator
            .all(|value| value.as_deref() == Some(first.as_str()))
            .then_some(first)
    };
    let metric = shared(
        points
            .iter()
            .map(|point| {
                quantities
                    .iter()
                    .find(|quantity| {
                        quantity.start_byte == point.start && quantity.end_byte == point.end
                    })
                    .and_then(|quantity| quantity.predicate.clone())
            })
            .collect(),
    );
    let unit = shared(
        matched
            .iter()
            .map(|quantity| Some(quantity.unit.clone()))
            .chain(std::iter::repeat(None))
            .take(points.len())
            .collect(),
    )
    .or_else(|| {
        let signatures: Vec<String> = measures
            .iter()
            .map(|measure| measure.signature.clone())
            .collect();
        let first = signatures.first()?;
        (signatures.iter().all(|signature| signature == first)
            && measures.iter().all(|measure| measure.signature != measure.kind))
        .then(|| first.clone())
    });
    let dimension = shared(
        matched
            .iter()
            .map(|quantity| Some(quantity.dimension.clone()))
            .chain(std::iter::repeat(None))
            .take(points.len())
            .collect(),
    )
    .or_else(|| {
        let kind = &items[0].kind;
        if !items.iter().all(|item| &item.kind == kind) {
            return None;
        }
        match kind.as_str() {
            "PERCENT" => Some("percent".to_string()),
            "MONEY" => Some("currency".to_string()),
            _ => None,
        }
    });
    let period = shared(
        points
            .iter()
            .map(|point| {
                quantities
                    .iter()
                    .find(|quantity| {
                        quantity.start_byte == point.start && quantity.end_byte == point.end
                    })
                    .and_then(|quantity| quantity.period.as_ref())
                    .map(|period| period.year.to_string())
            })
            .collect(),
    );
    let series_type = if labels_are_ordered_years(items) {
        Some("time".to_string())
    } else {
        Some("category".to_string())
    };
    Some(semantic_core::ChartMetadata {
        metric,
        unit,
        dimension,
        period,
        series_type,
    })
}

fn has_four_digit_year(text: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_digit()).any(|token| {
        token.len() == 4
            && token
                .parse::<u16>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
    })
}

fn single_year_label(text: &str) -> Option<String> {
    let years: Vec<String> = text
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|token| {
            token.len() == 4
                && token
                    .parse::<u16>()
                    .is_ok_and(|year| (1900..=2099).contains(&year))
        })
        .map(str::to_string)
        .collect();
    (years.len() == 1).then(|| years[0].clone())
}

fn chart_claim_bounds(bullet: &Bullet, points: &[&ChartPoint]) -> (usize, usize) {
    if crate::chart_rules::chart_claim_is_eligible(&bullet.text) {
        return (bullet.start, bullet.end);
    }
    let Some(first) = points.iter().map(|point| point.start).min() else {
        return (bullet.start, bullet.end);
    };
    let Some(last) = points.iter().map(|point| point.end).max() else {
        return (bullet.start, bullet.end);
    };
    let local_first = first.saturating_sub(bullet.start);
    let local_last = last.saturating_sub(bullet.start);
    let left = bullet.text[..local_first]
        .char_indices()
        .filter(|(_, ch)| matches!(ch, ':' | ';' | '.' | '!' | '?' | '\n'))
        .map(|(index, ch)| index + ch.len_utf8())
        .next_back()
        .unwrap_or(0);
    let right = bullet.text[local_last..]
        .char_indices()
        .find(|(_, ch)| matches!(ch, ':' | ';' | '.' | '!' | '?' | '\n'))
        .map(|(index, _)| local_last + index)
        .unwrap_or(bullet.text.len());
    (bullet.start + left, bullet.start + right)
}

fn chart_claim_evidence(source: &str, bullet: &Bullet, points: &[&ChartPoint]) -> String {
    let (start, end) = chart_claim_bounds(bullet, points);
    source
        .get(start..end)
        .unwrap_or(&bullet.text)
        .trim()
        .to_string()
}

/// Uses the nearest source-grounded nonnumeric entity as the chart category.
/// When the value is a time-series point, its nearby year is the category.
/// The full sentence is never used as a chart label.
fn chart_subject_label(
    source: &str,
    entities: &[VisualEntity],
    point: &ChartPoint,
    bullet: &Bullet,
    scope_start: usize,
    scope_end: usize,
) -> String {
    let point_start = point.start;
    let point_end = point.end;
    let years = year_spans(&bullet.text, bullet.start);
    if years.len() >= 2 {
        if let Some((_, _, year)) = years
            .iter()
            .filter(|(start, end, _)| {
                *end <= point_start && *start >= scope_start.saturating_sub(80)
            })
            .max_by_key(|(_, end, _)| *end)
        {
            return year.clone();
        }
    }
    let entity_candidate = entities
        .iter()
        .filter(|entity| {
            entity.start >= scope_start
                && entity.end <= scope_end
                && entity.end <= point_start.saturating_add(80)
                && entity.start.saturating_add(80) >= point_end
                && !matches!(
                    entity.r#type.as_str(),
                    "NUMBER" | "PERCENT" | "MONEY" | "DATE"
                )
        })
        .min_by_key(|entity| span_distance(entity.start, entity.end, point_start, point_end))
        .and_then(|entity| source.get(entity.start..entity.end).map(str::to_string));
    // Percentages normally describe the immediately following population
    // ("42% dei tecnici", "18% del gruppo di controllo"). Keep that complete
    // grounded noun phrase as the category instead of dropping its relation
    // marker and displaying only the head noun.
    let percentage_population = (point.kind == "PERCENT")
        .then(|| {
            entities
                .iter()
                .filter(|entity| {
                    entity.start >= point_end
                        && entity.start < scope_end
                        && !matches!(
                            entity.r#type.as_str(),
                            "NUMBER" | "PERCENT" | "MONEY" | "DATE"
                        )
                        && entity.start.saturating_sub(point_end) <= 24
                })
                .min_by_key(|entity| entity.start)
                .and_then(|entity| {
                    let mut end = entity.end;
                    let tail = source.get(end..scope_end)?;
                    let mut words = tail.split_whitespace();
                    let connector = words.next()?;
                    if matches!(
                        connector.to_lowercase().as_str(),
                        "di" | "de" | "of" | "van" | "von"
                    ) {
                        let next = words.next()?;
                        let next_len = next.trim_end_matches([',', ';', '.', ':']).len();
                        end += tail.find(next)? + next_len;
                    }
                    source.get(point_end..end)
                })
                .map(str::trim)
                .filter(|label| {
                    let words = label.split_whitespace().count();
                    (2..=5).contains(&words)
                        && !label.chars().any(|ch| matches!(ch, ',' | ';' | '.' | ':'))
                })
                .map(str::to_string)
        })
        .flatten();
    let proper_candidate = semantic_core::discourse::word_spans(&bullet.text)
        .into_iter()
        .filter_map(|word| {
            let start = bullet.start + word.start;
            let end = bullet.start + word.end;
            let surface = source.get(start..end)?;
            let first = surface.chars().next()?;
            if end > scope_end
                || !first.is_uppercase()
                || end > point_start
                || semantic_core::chart_cues::global()
                    .subject_stopwords
                    .iter()
                    .any(|stopword| stopword == &word.lowered)
            {
                return None;
            }
            let distance = span_distance(start, end, point_start, point_end);
            (distance <= 80).then_some((distance, surface.to_string()))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, surface)| surface);
    if let Some(proper) = proper_candidate {
        return proper;
    }
    if let Some(population) = percentage_population {
        return population;
    }
    if let Some(entity) = entity_candidate {
        return entity;
    }
    let local_start = point_start.saturating_sub(40).max(bullet.start);
    let local_end = point_end.saturating_add(40).min(bullet.end);
    if let Some(context) = source.get(local_start..local_end) {
        if let Some(year) = context.split(|ch: char| !ch.is_ascii_digit()).find(|word| {
            word.len() == 4
                && word
                    .parse::<u16>()
                    .is_ok_and(|year| (1900..=2099).contains(&year))
        }) {
            return year.to_string();
        }
    }
    if let Some(after_value) = source.get(point_end..scope_end.min(bullet.end)) {
        let descriptor: Vec<&str> = after_value
            .split_whitespace()
            .take_while(|word| {
                let word = word
                    .trim_matches(|ch: char| !ch.is_alphanumeric())
                    .to_lowercase();
                !semantic_core::chart_cues::global()
                    .subject_verb_cutoffs
                    .iter()
                    .any(|cutoff| cutoff == &word)
            })
            .take(6)
            .collect();
        let descriptor = descriptor.join(" ");
        let descriptor = descriptor.trim_matches(|ch: char| !ch.is_alphanumeric());
        if !descriptor.is_empty() {
            return descriptor.to_string();
        }
    }
    format!("valore {}", point.text)
}

fn year_spans(text: &str, base: usize) -> Vec<(usize, usize, String)> {
    let mut years = Vec::new();
    let mut token_start = None;
    for (index, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            token_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = token_start.take() {
            let token = &text[start..index];
            if token.len() == 4
                && token
                    .parse::<u16>()
                    .is_ok_and(|year| (1900..=2099).contains(&year))
            {
                years.push((base + start, base + index, token.to_string()));
            }
        }
    }
    if let Some(start) = token_start {
        let token = &text[start..];
        if token.len() == 4
            && token
                .parse::<u16>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
        {
            years.push((base + start, base + text.len(), token.to_string()));
        }
    }
    years
}

fn span_distance(start: usize, end: usize, point_start: usize, point_end: usize) -> usize {
    if end <= point_start {
        point_start - end
    } else {
        start.saturating_sub(point_end)
    }
}


/// A comparison word elsewhere in a long enumerated bullet must not turn the
/// list's rank numbers into chart values. Bind the cue to at least two
/// comparable values on the same source line. Ordinary prose comparisons
/// remain valid because their values and cue share a line.
fn comparison_cue_binds_values(
    bullet: &Bullet,
    chart_points: &[ChartPoint],
    sentence_index: usize,
    language: &str,
) -> bool {
    let explicit = semantic_core::discourse::comparison_cue(&bullet.text);
    let (cue_start, bare) = match explicit {
        Some((start, _)) => (start, false),
        None => match bare_conjunction_cue(bullet, chart_points, sentence_index, language) {
            Some(start) => (start, true),
            None => return false,
        },
    };
    let cue_line = bullet.text[..cue_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    let points_on_line: Vec<&ChartPoint> = chart_points
        .iter()
        .filter(|point| point.sentence_index == sentence_index)
        .filter(|point| point.start >= bullet.start && point.end <= bullet.end)
        .filter(|point| {
            let relative = point.start - bullet.start;
            bullet.text[..relative]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                == cue_line
        })
        .collect();
    if points_on_line.len() < 2 {
        return false;
    }
    // A bare conjunction is a contrast only when the grounded values straddle
    // it. "28% ... a ... 13%" qualifies; "a" before or after both numbers does
    // not. The language guard is explicit: Italian/Spanish/Portuguese "a" is
    // never passed here.
    if bare {
        let absolute = bullet.start + cue_start;
        let has_before = points_on_line.iter().any(|point| point.end <= absolute);
        let has_after = points_on_line.iter().any(|point| point.start > absolute);
        if !has_before || !has_after {
            return false;
        }
    }
    true
}

/// Locale-scoped fallback for Polish `a` and Russian `а`, used only when the
/// token sits between two grounded values and both sides carry a word. The cue
/// is never added to the global comparison vocabulary: in Italian, Spanish and
/// Portuguese the same letter is an article or preposition.
fn bare_conjunction_cue(
    bullet: &Bullet,
    chart_points: &[ChartPoint],
    sentence_index: usize,
    language: &str,
) -> Option<usize> {
    let cue = match language.split(['-', '_']).next().unwrap_or("") {
        "pl" => "a",
        "ru" => "а",
        _ => return None,
    };
    let words = semantic_core::discourse::word_spans(&bullet.text);
    let position = words.iter().position(|word| word.lowered == cue)?;
    let word = &words[position];
    let absolute_start = bullet.start + word.start;
    let absolute_end = bullet.start + word.end;
    let points: Vec<&ChartPoint> = chart_points
        .iter()
        .filter(|point| point.sentence_index == sentence_index)
        .filter(|point| point.start >= bullet.start && point.end <= bullet.end)
        .collect();
    if !points.iter().any(|point| point.end <= absolute_start)
        || !points.iter().any(|point| point.start >= absolute_end)
    {
        return None;
    }
    let substantive = |spans: &[semantic_core::discourse::WordSpan]| {
        spans.iter().any(|span| {
            span.lowered
                .chars()
                .filter(|character| character.is_alphabetic())
                .count()
                >= 2
        })
    };
    if !substantive(&words[..position]) || !substantive(&words[position + 1..]) {
        return None;
    }
    Some(word.start)
}
