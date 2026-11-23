use super::*;

pub(super) fn chart_metadata(
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
            quantities.iter().find(|quantity| {
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
            && measures
                .iter()
                .all(|measure| measure.signature != measure.kind))
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

pub(super) fn has_four_digit_year(text: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_digit()).any(|token| {
        token.len() == 4
            && token
                .parse::<u16>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
    })
}

pub(super) fn single_year_label(text: &str) -> Option<String> {
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

pub(super) fn chart_claim_bounds(bullet: &Bullet, points: &[&ChartPoint]) -> (usize, usize) {
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

pub(super) fn chart_claim_evidence(
    source: &str,
    bullet: &Bullet,
    points: &[&ChartPoint],
) -> String {
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
pub(super) fn chart_subject_label(
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
