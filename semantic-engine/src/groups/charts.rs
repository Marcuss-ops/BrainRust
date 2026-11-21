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
        out.push(VisualCandidate {
            kind: "chart",
            sentence_index: sentence,
            entity_ids: ids_of(identities, &indices),
            entity_texts: texts_of(entities, &indices),
            entity_indices: indices,
            chart_items: items,
            temporal_surfaces: Vec::new(),
            suggested_layout: Some("bar_comparison"),
            requires_verified_images: false,
            evidence: chart_claim_evidence(source, bullet, &points),
            start: chart_claim_bounds(bullet, &points).0,
            end: chart_claim_bounds(bullet, &points).1,
        });
    }
    cross_sentence_series(
        source,
        entities,
        bullets,
        chart_points,
        quantities,
        identities,
        out,
    );
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
/// across sentences (for example, annual revenue or output). The emitted
/// evidence is one exact source slice spanning all included sentences.
fn cross_sentence_series(
    source: &str,
    entities: &[VisualEntity],
    bullets: &[Bullet],
    chart_points: &[ChartPoint],
    quantities: &[Quantity],
    identities: &IdentityLookup,
    out: &mut Vec<VisualCandidate>,
) {
    let mut ordered: Vec<&ChartPoint> = chart_points.iter().collect();
    ordered.sort_by_key(|point| (point.start, point.end));
    for (left_index, left) in ordered.iter().enumerate() {
        for right in ordered.iter().skip(left_index + 1) {
            if left.sentence_index == right.sentence_index {
                continue;
            }
            let Some(left_bullet) = bullets
                .iter()
                .find(|b| b.sentence_index == left.sentence_index)
            else {
                continue;
            };
            let Some(right_bullet) = bullets
                .iter()
                .find(|b| b.sentence_index == right.sentence_index)
            else {
                continue;
            };
            if right.sentence_index.saturating_sub(left.sentence_index) > 3
                || !crate::chart_rules::chart_point_is_eligible(
                    &left_bullet.text,
                    left.start.saturating_sub(left_bullet.start),
                    left.end.saturating_sub(left_bullet.start),
                )
                || !crate::chart_rules::chart_point_is_eligible(
                    &right_bullet.text,
                    right.start.saturating_sub(right_bullet.start),
                    right.end.saturating_sub(right_bullet.start),
                )
                || !same_series_metric(&left_bullet.text, &right_bullet.text)
            {
                continue;
            }
            let has_time_anchor = has_four_digit_year(&left_bullet.text)
                && (has_four_digit_year(&right_bullet.text)
                    || right_bullet.text.to_lowercase().contains("seguente")
                    || right_bullet.text.to_lowercase().contains("following year"));
            if !has_time_anchor {
                continue;
            }
            let (left_claim_start, _) = chart_claim_bounds(left_bullet, &[*left]);
            let (right_claim_start, right_claim_end) = chart_claim_bounds(right_bullet, &[*right]);
            let evidence_start = left_claim_start.min(right_claim_start);
            let evidence_end = right_claim_end;
            let Some(evidence) = source.get(evidence_start..evidence_end) else {
                continue;
            };
            let Some(left_start) = left.start.checked_sub(left_bullet.start) else {
                continue;
            };
            let Some(left_end) = left.end.checked_sub(left_bullet.start) else {
                continue;
            };
            let Some(right_start) = right.start.checked_sub(right_bullet.start) else {
                continue;
            };
            let Some(right_end) = right.end.checked_sub(right_bullet.start) else {
                continue;
            };
            let mut measures = [
                chart_measure(&left_bullet.text, left_start, left_end, left, quantities),
                chart_measure(
                    &right_bullet.text,
                    right_start,
                    right_end,
                    right,
                    quantities,
                ),
            ];
            if measures[0].signature == measures[0].kind
                && measures[1].signature != measures[1].kind
            {
                measures[0].signature = measures[1].signature.clone();
                measures[0].kind = measures[1].kind.clone();
            } else if measures[1].signature == measures[1].kind
                && measures[0].signature != measures[0].kind
            {
                measures[1].signature = measures[0].signature.clone();
                measures[1].kind = measures[0].kind.clone();
            }
            if measures[0].signature != measures[1].signature
                || measures[0].kind != measures[1].kind
            {
                continue;
            }
            let (left_display_start, left_display_end) =
                crate::chart_rules::qualified_numeric_span(&left_bullet.text, left_start, left_end);
            let (right_display_start, right_display_end) =
                crate::chart_rules::qualified_numeric_span(
                    &right_bullet.text,
                    right_start,
                    right_end,
                );
            let (Some(left_text), Some(right_text)) = (
                left_bullet.text.get(left_display_start..left_display_end),
                right_bullet
                    .text
                    .get(right_display_start..right_display_end),
            ) else {
                continue;
            };
            let items = vec![
                ChartItem {
                    label: chart_subject_label(
                        source,
                        entities,
                        left,
                        left_bullet,
                        left_bullet.start,
                        left_bullet.end,
                    ),
                    text: left_text.to_string(),
                    kind: measures[0].kind.clone(),
                    value: measures[0].value,
                    start_byte: left_bullet.start + left_display_start,
                    end_byte: left_bullet.start + left_display_end,
                },
                ChartItem {
                    label: chart_subject_label(
                        source,
                        entities,
                        right,
                        right_bullet,
                        right_bullet.start,
                        right_bullet.end,
                    ),
                    text: right_text.to_string(),
                    kind: measures[1].kind.clone(),
                    value: measures[1].value,
                    start_byte: right_bullet.start + right_display_start,
                    end_byte: right_bullet.start + right_display_end,
                },
            ];
            if out.iter().any(|candidate| {
                candidate.kind == "chart"
                    && candidate
                        .chart_items
                        .iter()
                        .any(|item| item.start_byte == left.start)
                    && candidate
                        .chart_items
                        .iter()
                        .any(|item| item.start_byte == right.start)
            }) {
                continue;
            }
            let mut indices: Vec<usize> =
                [left.entity, right.entity].into_iter().flatten().collect();
            indices.sort_unstable();
            indices.dedup();
            out.push(VisualCandidate {
                kind: "chart",
                sentence_index: left.sentence_index.min(right.sentence_index),
                entity_ids: ids_of(identities, &indices),
                entity_texts: texts_of(entities, &indices),
                entity_indices: indices,
                chart_items: items,
                temporal_surfaces: Vec::new(),
                suggested_layout: Some("bar_comparison"),
                requires_verified_images: false,
                evidence: evidence.to_string(),
                start: evidence_start,
                end: evidence_end,
            });
        }
    }
}

fn has_four_digit_year(text: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_digit()).any(|token| {
        token.len() == 4
            && token
                .parse::<u16>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
    })
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
            let stopwords = [
                "nel", "nella", "il", "la", "lo", "un", "una", "the", "a", "in", "on", "der",
                "die", "das", "el", "los", "las", "le", "les", "no", "en", "em", "im", "w", "в",
                "pada", "dans", "sur", "du", "au", "auf", "am", "zum",
            ];
            if end > scope_end
                || !first.is_uppercase()
                || end > point_start
                || stopwords.contains(&word.lowered.as_str())
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
                ![
                    "ha",
                    "hanno",
                    "è",
                    "sono",
                    "era",
                    "erano",
                    "was",
                    "were",
                    "is",
                    "are",
                    "has",
                    "have",
                    "had",
                    "sold",
                    "recorded",
                    "reported",
                    "reached",
                    "produced",
                    "venduto",
                    "venduta",
                    "registrato",
                    "registrata",
                    "raggiunto",
                    "raggiunta",
                    "prodotto",
                    "prodotta",
                    "aumentate",
                    "aumentato",
                    "passed",
                    "rose",
                    "increased",
                ]
                .contains(&word.as_str())
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

fn same_series_metric(left: &str, right: &str) -> bool {
    const METRICS: &[&str] = &[
        "ricav",
        "fatturat",
        "vendu",
        "vendit",
        "prodott",
        "produzion",
        "incident",
        "sales",
        "revenue",
        "produced",
        "production",
        "accident",
        "export",
    ];
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    METRICS
        .iter()
        .any(|metric| left.contains(metric) && right.contains(metric))
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
