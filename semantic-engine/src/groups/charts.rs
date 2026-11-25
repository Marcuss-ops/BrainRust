//! Chart proposal construction and source-grounded chart helpers.

use super::*;

/// Finds the typed quantity backing one chart point. The point and the
/// quantity normally share the exact numeric span; a qualified span can be
/// wider, so the containing quantity is the fallback.
fn chart_point_quantity(
    quantities: &[Quantity],
    sentence_index: usize,
    start: usize,
    end: usize,
) -> Option<&Quantity> {
    quantities
        .iter()
        .find(|quantity| quantity.start_byte == start && quantity.end_byte == end)
        .or_else(|| {
            quantities.iter().find(|quantity| {
                quantity.sentence_index == sentence_index
                    && quantity.start_byte <= start
                    && quantity.end_byte >= end
            })
        })
}

/// A bare number beside one explicit unit ("120 cars" then "180") inherits
/// that unit. The sentence-local builder and the cross-sentence series builder
/// carried this block verbatim; one helper keeps the rule from drifting.
fn inherit_explicit_units(measures: &mut [crate::chart_rules::ChartUnit]) {
    let explicit: std::collections::BTreeSet<(String, String)> = measures
        .iter()
        .filter(|measure| measure.signature != measure.kind)
        .map(|measure| (measure.signature.clone(), measure.kind.clone()))
        .collect();
    if explicit.len() != 1 {
        return;
    }
    let (signature, kind) = explicit.into_iter().next().expect("length checked");
    for measure in measures {
        if measure.signature == measure.kind {
            measure.signature = signature.clone();
            measure.kind = kind.clone();
        }
    }
}

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
        let relative_spans: Vec<(usize, usize)> = points
            .iter()
            .filter_map(|point| {
                Some((
                    point.start.checked_sub(bullet.start)?,
                    point.end.checked_sub(bullet.start)?,
                ))
            })
            .collect();
        if crate::chart_rules::mixed_forecast_claims(&bullet.text, &relative_spans) {
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
        inherit_explicit_units(&mut measures);
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
                let quantity = chart_point_quantity(quantities, p.sentence_index, p.start, p.end);
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
                        language,
                    ),
                    text: text.to_string(),
                    kind: measure.kind.clone(),
                    value: measure.value,
                    start_byte: bullet.start + qualified_start,
                    end_byte: bullet.start + qualified_end,
                    qualifier: quantity
                        .map(|quantity| quantity.qualifier.clone())
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            crate::chart_rules::numeric_qualifier(
                                &bullet.text,
                                relative_start,
                                relative_end,
                            )
                        }),
                    period: quantity
                        .and_then(|quantity| quantity.period.as_ref())
                        .map(|period| period.year.to_string()),
                    lower: None,
                    upper: None,
                    range_id: quantity.and_then(|quantity| quantity.range_id.clone()),
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
        let lower_bullet = bullet.text.to_lowercase();
        let ranked_context = semantic_core::chart_cues::global()
            .rank_veto_cues
            .iter()
            .any(|cue| lower_bullet.contains(&cue.to_lowercase()));
        let subject_bound =
            !ranked_context && subject_comparison_binds_values(entities, &points, bullet);
        let percentage_distribution = !ranked_context && percentage_distribution_points(&points);
        if !comparison_cue_binds_values(bullet, chart_points, sentence, language)
            && !crate::chart_rules::has_series_context(&bullet.text, items.len(), language)
            && !subject_bound
            && !percentage_distribution
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
        SeriesContext {
            source,
            entities,
            bullets,
            chart_points,
            quantities,
            identities,
            language,
        },
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
            qualifier: Some(quantity.qualifier.clone()).filter(|value| !value.is_empty()),
            period: quantity
                .period
                .as_ref()
                .map(|period| period.year.to_string()),
            lower: None,
            upper: None,
            range_id: quantity.range_id.clone(),
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
                chart_meta.period = Some(format!("{}→{}", left_period.year, right_period.year));
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
            suggested_layout: Some("before_after_metric"),
            requires_verified_images: false,
            evidence: evidence.to_string(),
            start,
            end,
        });
    }
}

#[path = "charts/series.rs"]
mod series;
use series::*;

/// True when at least two distinct subject entities are each bound to a
/// grounded value. This is the unit-independent comparison shape
/// ("Mario 1500, Luca 1800, Marco 1200") that no explicit comparison word
/// marks. Non-subject phrases ("codice", "password") never qualify.
fn subject_comparison_binds_values(
    entities: &[VisualEntity],
    points: &[&ChartPoint],
    bullet: &Bullet,
) -> bool {
    let mut subjects = std::collections::BTreeSet::new();
    for point in points {
        let nearest = entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| entity.start >= bullet.start && entity.end <= bullet.end)
            .filter(|(_, entity)| entity.end <= point.start && point.start - entity.end <= 80)
            .filter(|(_, entity)| {
                semantic_core::entity_capabilities::entity_capabilities(&entity.r#type)
                    .co_mention_subject
            })
            .max_by_key(|(_, entity)| entity.end);
        if let Some((index, _)) = nearest {
            subjects.insert(index);
        }
    }
    if subjects.len() >= 2 {
        return true;
    }
    // Some taggers do not emit a PERSON entity for a bare first name. A
    // capitalized subject token bound to each value is still grounded
    // evidence; stopwords and sentence-initial articles are excluded.
    let stopwords = &semantic_core::chart_cues::global().subject_stopwords;
    let mut proper = std::collections::BTreeSet::new();
    for point in points {
        let nearest = semantic_core::discourse::word_spans(&bullet.text)
            .into_iter()
            .filter(|word| {
                let end = bullet.start + word.end;
                end <= point.start && point.start - end <= 80
            })
            .filter(|word| {
                let surface = &bullet.text[word.start..word.end];
                surface.chars().next().is_some_and(char::is_uppercase)
                    && surface.chars().filter(|ch| ch.is_alphabetic()).count() >= 2
                    && !stopwords.iter().any(|stop| stop == &word.lowered)
            })
            .max_by_key(|word| word.end);
        if let Some(word) = nearest {
            proper.insert(word.lowered.clone());
        }
    }
    proper.len() >= 2
}

/// A three-or-more-part percentage breakdown that covers the whole (about
/// 100%) is a distribution even without an explicit share word.
fn percentage_distribution_points(points: &[&ChartPoint]) -> bool {
    points.len() >= 3
        && points.iter().all(|point| point.kind == "PERCENT")
        && (95.0..=100.5).contains(&points.iter().map(|point| point.value).sum::<f64>())
}

/// Picks the layout the grounded values support. The default stays
/// `bar_comparison`; `timeline_growth` needs ordered year labels;
/// `segmented_total_bar` needs percentage parts whose sum is a complete share
/// (or an explicit share cue in the evidence).
fn select_layout(items: &[ChartItem], evidence: &str, language: &str) -> &'static str {
    // An explicit range pair wins over the generic series reading: its two
    // endpoints are one observation, not a growth timeline.
    if let Some(range) = items.first().and_then(|item| item.range_id.as_deref()) {
        if items
            .iter()
            .all(|item| item.range_id.as_deref() == Some(range))
        {
            return "bounded_range_comparison";
        }
    }
    let scenario = {
        let lower = evidence.to_lowercase();
        semantic_core::chart_cues::language_cues(language)
            .scenario_cues
            .iter()
            .any(|cue| lower.contains(&cue.to_lowercase()))
    };
    if scenario {
        return "scenario_split";
    }
    if labels_are_ordered_years(items) {
        if items.iter().any(|item| {
            matches!(
                item.qualifier.as_deref(),
                Some("approximate" | "greater_than" | "range")
            )
        }) {
            return "qualified_growth";
        }
        if items
            .iter()
            .any(|item| item.qualifier.as_deref() == Some("missing"))
        {
            return "missing_data_timeline";
        }
        return "timeline_growth";
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
            return "segmented_total_bar";
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
    years.windows(2).all(|pair| pair[1] > pair[0]) || years.windows(2).all(|pair| pair[1] < pair[0])
}

#[path = "charts/metadata.rs"]
mod metadata;
use metadata::*;
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

#[cfg(test)]
#[path = "charts/unit_inheritance_tests.rs"]
mod unit_inheritance_tests;
