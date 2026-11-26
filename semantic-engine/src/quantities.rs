//! Conservative attachment of unit metadata to grounded numeric facts.

use semantic_core::discourse::word_spans;
use semantic_core::units::{language_units, UnitDefinition};
use semantic_core::{
    parse_unambiguous_value, scale_word_factor, NumericFact, Quantity, QuantityComparison,
    QuantityPeriod, TemporalFact,
};

pub(crate) fn extract(
    source: &str,
    language: &str,
    numeric_facts: &[NumericFact],
    temporal_facts: &[TemporalFact],
) -> Vec<Quantity> {
    let Some(language_data) = language_units(language) else {
        return Vec::new();
    };
    let words = word_spans(source);
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut quantities = Vec::new();

    for fact in numeric_facts {
        let Some(sentence_index) = sentences.iter().position(|sentence| {
            sentence.start_byte <= fact.start_byte && fact.end_byte <= sentence.end_byte
        }) else {
            continue;
        };
        let Some(sentence) = sentences.get(sentence_index) else {
            continue;
        };
        let Some(number_text) = source.get(fact.start_byte..fact.end_byte) else {
            continue;
        };
        if number_text != fact.surface || fact.end_byte <= fact.start_byte {
            continue;
        }
        let next_number_start = numeric_facts
            .iter()
            .filter(|other| other.start_byte > fact.start_byte)
            .filter(|other| {
                sentences.iter().any(|other_sentence| {
                    other_sentence.start_byte == sentence.start_byte
                        && other_sentence.start_byte <= other.start_byte
                        && other.end_byte <= other_sentence.end_byte
                })
            })
            .map(|other| other.start_byte)
            .min();
        let shared_range_unit = next_number_start.is_some_and(|next_start| {
            is_explicit_range(
                source,
                fact.start_byte,
                fact.end_byte,
                next_start,
                language_data,
            )
        });
        let unit_search_end = if shared_range_unit {
            sentence.end_byte
        } else {
            next_number_start.unwrap_or(sentence.end_byte)
        };
        let unit_match = find_unit(
            source,
            fact,
            &words,
            language,
            &language_data.units,
            unit_search_end,
            shared_range_unit,
        );
        let Some((unit, unit_surface)) = unit_match else {
            continue;
        };
        // "3 volte al giorno" is a frequency idiom, not a ratio. The period
        // cue table is language data; its presence demotes the ratio.
        if unit.dimension == "ratio"
            && is_frequency_ratio(
                source,
                sentence.start_byte,
                sentence.end_byte,
                language_data,
            )
        {
            continue;
        }
        // "2 kali 5%" / "twice 5%": the ratio word multiplies the next
        // quantity, so it is a comparison cue, not a measured ratio of its
        // own. Keeping it as a fact made the following percent values
        // incompatible and hid the ratio comparison.
        if unit.dimension == "ratio"
            && next_number_start.is_some_and(|next| {
                next > fact.end_byte
                    && next <= sentence.end_byte
                    && source
                        .get(fact.end_byte..next)
                        .is_some_and(|gap| gap.split_whitespace().count() <= 2)
            })
            && has_ratio_signal(sentence.text(source), language)
        {
            continue;
        }
        let scale_in_surface = word_spans(number_text)
            .iter()
            .find_map(|word| scale_word_factor(&word.lowered));
        let scale_end = next_number_start.unwrap_or(sentence.end_byte);
        let scale_after_surface = words
            .iter()
            .filter(|word| word.start >= fact.end_byte && word.end <= scale_end)
            .find_map(|word| scale_word_factor(&word.lowered));
        let scale = scale_in_surface.or(scale_after_surface);
        let raw_value = match (fact.value, scale_in_surface, scale) {
            (Some(value), Some(scale), _) => Some(value / scale),
            (Some(value), None, Some(_)) => Some(value),
            (Some(value), None, None) => Some(value),
            (None, _, _) => number_text
                .split_whitespace()
                .find_map(|part| parse_unambiguous_value(part, language)),
        };
        let value = raw_value.map(|raw| raw * scale.unwrap_or(1.0) * unit.factor_to_base);
        let qualifier = qualifier_for(source, fact.start_byte, language_data);
        let status =
            match semantic_core::relations::clause_status(sentence.text(source), &[language]) {
                semantic_core::RelationStatus::Asserted => "asserted",
                semantic_core::RelationStatus::Negated => "negated",
                semantic_core::RelationStatus::Uncertain => "uncertain",
            };
        let period = bind_period(
            source,
            fact.start_byte,
            fact.end_byte,
            sentence_index,
            &sentences,
            temporal_facts,
        );
        quantities.push(Quantity {
            id: format!("q_{:02}", quantities.len() + 1),
            value,
            raw_value,
            scale,
            dimension: unit.dimension.clone(),
            unit: unit.code.clone(),
            unit_surface,
            qualifier,
            range_id: None,
            period,
            subject_entity_id: None,
            predicate: predicate_for(source, sentence_index, &sentences, language_data),
            start_byte: fact.start_byte,
            end_byte: fact.end_byte,
            evidence: number_text.to_string(),
            sentence_index,
            status: status.to_string(),
        });
    }
    assign_range_pairs(source, language, &mut quantities);
    quantities
}

pub(crate) fn comparisons(
    source: &str,
    language: &str,
    quantities: &[Quantity],
) -> Vec<QuantityComparison> {
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut out = Vec::new();
    for (sentence_index, sentence) in sentences.iter().enumerate() {
        let sentence_quantities: Vec<_> = quantities
            .iter()
            .filter(|quantity| quantity.sentence_index == sentence_index)
            .collect();
        if sentence_quantities.len() < 2 {
            continue;
        }
        let sentence_text = sentence.text(source);
        let language_key = language.split(['-', '_']).next().unwrap_or(language);
        let change_span = semantic_core::lexicon::lexicon()
            .discourse
            .change_signals
            .get(language_key)
            .and_then(|signals| find_cue_span(sentence_text, signals));
        let cue_span =
            change_span.or_else(|| semantic_core::discourse::comparison_cue(sentence_text));
        let Some((local_start, local_end)) = cue_span else {
            continue;
        };
        let cue_start = sentence.start_byte + local_start;
        let cue_end = sentence.start_byte + local_end;
        // A multiplier may have contributed a non-percent fact before the
        // pair ("10% compared with twice 5%"). Compare the first two facts
        // that actually share a dimension and unit instead of the first two
        // facts in source order.
        let Some((left, right)) = sentence_quantities
            .windows(2)
            .find(|pair| pair[0].dimension == pair[1].dimension && pair[0].unit == pair[1].unit)
            .map(|pair| (pair[0], pair[1]))
        else {
            continue;
        };
        let is_change = change_span.is_some();
        if cue_start < left.end_byte
            || (!is_change && cue_end > right.start_byte)
            || (is_change && cue_start > right.end_byte.saturating_add(24))
        {
            continue;
        }
        if left.dimension != right.dimension || left.unit != right.unit {
            continue;
        }
        if left.dimension == "currency"
            && (left.period.as_ref().map(|period| period.year)
                != right.period.as_ref().map(|period| period.year))
        {
            continue;
        }
        let (Some(left_value), Some(right_value)) = (left.value, right.value) else {
            continue;
        };
        let ratio_cue = semantic_core::lexicon::lexicon()
            .discourse
            .ratio_signals
            .get(language_key)
            .is_some_and(|signals| contains_cue_phrase(sentence.text(source), signals));
        let kind = if is_change {
            "change"
        } else if ratio_cue {
            "ratio"
        } else {
            "difference"
        };
        out.push(QuantityComparison {
            kind: kind.to_string(),
            left: left.id.clone(),
            right: right.id.clone(),
            dimension: left.dimension.clone(),
            ratio: (ratio_cue && right_value != 0.0).then_some(left_value / right_value),
            difference: (!ratio_cue).then_some(if is_change {
                right_value - left_value
            } else {
                left_value - right_value
            }),
            evidence_start_byte: cue_start,
            evidence_end_byte: cue_end,
        });
    }
    out
}

fn find_cue_span(text: &str, cues: &[String]) -> Option<(usize, usize)> {
    let words = semantic_core::discourse::word_spans(text);
    let mut best = None;
    for cue in cues {
        let expected = semantic_core::discourse::word_spans(cue);
        if expected.is_empty() {
            continue;
        }
        for window in words.windows(expected.len()) {
            if window
                .iter()
                .zip(&expected)
                .all(|(actual, expected)| actual.lowered == expected.lowered)
            {
                let span = (window[0].start, window[window.len() - 1].end);
                if best.is_none_or(|current: (usize, usize)| span.0 < current.0) {
                    best = Some(span);
                }
            }
        }
    }
    best
}

fn has_ratio_signal(text: &str, language: &str) -> bool {
    let key = language.split(['-', '_']).next().unwrap_or(language);
    semantic_core::lexicon::lexicon()
        .discourse
        .ratio_signals
        .get(key)
        .is_some_and(|cues| contains_cue_phrase(text, cues))
}

fn contains_cue_phrase(text: &str, cues: &[String]) -> bool {
    let words = word_spans(text);
    cues.iter().any(|cue| {
        let expected = word_spans(cue);
        !expected.is_empty()
            && words.windows(expected.len()).any(|window| {
                window
                    .iter()
                    .zip(&expected)
                    .all(|(actual, expected)| actual.lowered == expected.lowered)
            })
    })
}

pub(crate) fn attach_grounded_facts(
    quantities: &mut [Quantity],
    facts: &[semantic_core::EntityFact],
) {
    for quantity in quantities {
        if let Some(fact) = facts.iter().find(|fact| {
            fact.start_byte == quantity.start_byte && fact.end_byte == quantity.end_byte
        }) {
            quantity.subject_entity_id = Some(fact.subject_entity_id.clone());
            quantity.predicate = Some(fact.predicate.clone());
            quantity.status = fact.status.clone();
        }
    }
}

fn is_explicit_range(
    source: &str,
    first_start: usize,
    first_end: usize,
    second_start: usize,
    data: &semantic_core::units::LanguageUnits,
) -> bool {
    let prefix = source[..first_start].to_lowercase();
    let connector = source
        .get(first_end..second_start)
        .unwrap_or("")
        .to_lowercase();
    let range_open = data.qualifiers.get("range").is_some_and(|cues| {
        cues.iter()
            .any(|cue| prefix.trim_end().ends_with(&cue.to_lowercase()))
    });
    range_open
        && [
            " and ", " e ", " a ", " to ", " y ", " et ", " und ", " i ", " sampai ", " en ",
            " и ", " ile ",
        ]
        .iter()
        .any(|cue| connector.contains(cue))
}

fn assign_range_pairs(source: &str, language: &str, quantities: &mut [Quantity]) {
    let Some(data) = language_units(language) else {
        return;
    };
    let sentences = semantic_core::split_sentences(source, language, None);
    let mut range_number = 0;
    for (sentence_index, _) in sentences.iter().enumerate() {
        let indices: Vec<usize> = quantities
            .iter()
            .enumerate()
            .filter_map(|(index, quantity)| {
                (quantity.sentence_index == sentence_index).then_some(index)
            })
            .collect();
        for pair in indices.windows(2) {
            let first = &quantities[pair[0]];
            let second = &quantities[pair[1]];
            if first.dimension != second.dimension || first.unit != second.unit {
                continue;
            }
            if !is_explicit_range(
                source,
                first.start_byte,
                first.end_byte,
                second.start_byte,
                data,
            ) {
                continue;
            }
            range_number += 1;
            let id = format!("range_{range_number:02}");
            quantities[pair[0]].range_id = Some(id.clone());
            quantities[pair[0]].qualifier = "range_start".to_string();
            quantities[pair[1]].range_id = Some(id);
            quantities[pair[1]].qualifier = "range_end".to_string();
            if quantities[pair[0]].scale.is_none() {
                if let Some(scale) = quantities[pair[1]].scale {
                    let unit_factor = data
                        .units
                        .iter()
                        .find(|unit| {
                            unit.dimension == quantities[pair[0]].dimension
                                && unit.code == quantities[pair[0]].unit
                        })
                        .map_or(1.0, |unit| unit.factor_to_base);
                    quantities[pair[0]].scale = Some(scale);
                    quantities[pair[0]].value = quantities[pair[0]]
                        .raw_value
                        .map(|raw| raw * scale * unit_factor);
                }
            }
        }
    }
}

fn find_unit<'a>(
    source: &str,
    fact: &NumericFact,
    words: &[semantic_core::discourse::WordSpan],
    language: &str,
    units: &'a [UnitDefinition],
    unit_search_end: usize,
    shared_range_unit: bool,
) -> Option<(&'a UnitDefinition, String)> {
    let surface = source.get(fact.start_byte..fact.end_byte)?;
    if fact.kind == "PERCENT" {
        // A signed/period-bound percentage change ("+12% YoY", "-4,5% annuo")
        // is a growth rate, not a level percentage. The growth cue table is
        // chart data shared with the chart lane.
        if growth_cue_present(source, fact.start_byte, unit_search_end, language) {
            if let Some(unit) = units.iter().find(|unit| unit.dimension == "growth_rate") {
                return Some((unit, surface.to_string()));
            }
        }
        return semantic_core::units::find_unit(language, "%")
            .or_else(|| semantic_core::units::find_unit(language, "percent"))
            .map(|unit| (unit, surface.to_string()));
    }
    // Compound slash units (notably km/h) must win over a prefix unit such as
    // km. Otherwise a speed measure is silently typed as length before the
    // more specific unit row is considered.
    for unit in units {
        for candidate in &unit.surfaces {
            if !candidate.contains('/') {
                continue;
            }
            let Some(tail) = source.get(fact.end_byte..unit_search_end) else {
                continue;
            };
            let Some(offset) = tail.to_lowercase().find(&candidate.to_lowercase()) else {
                continue;
            };
            let start = fact.end_byte + offset;
            if source[fact.end_byte..start].split_whitespace().count() > 3 {
                continue;
            }
            let end = start + candidate.len();
            if let Some(surface) = source.get(start..end) {
                return Some((unit, surface.to_string()));
            }
        }
    }
    for unit in units {
        for candidate in &unit.surfaces {
            if candidate.chars().all(|ch| !ch.is_alphanumeric()) {
                if surface.to_lowercase().contains(&candidate.to_lowercase()) {
                    return Some((unit, surface.to_string()));
                }
                if let Some((start, end)) = adjacent_symbol(source, fact, candidate) {
                    return Some((unit, source[start..end].to_string()));
                }
                continue;
            }
            let candidate_words = word_spans(candidate);
            if candidate_words.is_empty() {
                continue;
            }
            let surface_words = word_spans(surface);
            for start in 0..surface_words.len() {
                let end_index = start + candidate_words.len();
                if end_index <= surface_words.len()
                    && candidate_words
                        .iter()
                        .enumerate()
                        .all(|(offset, word)| surface_words[start + offset].lowered == word.lowered)
                {
                    let start_byte = fact.start_byte + surface_words[start].start;
                    let end_byte = fact.start_byte + surface_words[end_index - 1].end;
                    return Some((unit, source[start_byte..end_byte].to_string()));
                }
            }
            for start in 0..words.len() {
                if words[start].start < fact.end_byte {
                    continue;
                }
                if words[start].start > unit_search_end {
                    break;
                }
                let end_index = start + candidate_words.len();
                if end_index > words.len()
                    || !candidate_words
                        .iter()
                        .enumerate()
                        .all(|(offset, word)| words[start + offset].lowered == word.lowered)
                {
                    continue;
                }
                let end = words[end_index - 1].end;
                let start_byte = words[start].start;
                let max_gap_words = if shared_range_unit { 8 } else { 3 };
                if source[fact.end_byte..start_byte].split_whitespace().count() > max_gap_words {
                    continue;
                }
                return Some((unit, source[start_byte..end].to_string()));
            }
        }
    }
    None
}

fn is_frequency_ratio(
    source: &str,
    sentence_start: usize,
    sentence_end: usize,
    data: &semantic_core::units::LanguageUnits,
) -> bool {
    // The period cue can precede the number (Turkish "günde 3 kez") or follow
    // it ("3 volte al giorno"), so the whole sentence is the frequency scope.
    let Some(scope) = source.get(sentence_start..sentence_end.min(source.len())) else {
        return false;
    };
    let tail = scope.to_lowercase();
    data.qualifiers
        .get("frequency_periods")
        .is_some_and(|cues| cues.iter().any(|cue| tail.contains(&cue.to_lowercase())))
}

fn growth_cue_present(
    source: &str,
    number_start: usize,
    search_end: usize,
    language: &str,
) -> bool {
    let start = number_start.saturating_sub(24);
    let start = source
        .char_indices()
        .find(|(index, _)| *index >= start)
        .map_or(0, |(index, _)| index);
    let end = search_end.min(source.len());
    let Some(window) = source.get(start..end) else {
        return false;
    };
    let window = window.to_lowercase();
    semantic_core::chart_cues::language_cues(language)
        .growth_cues
        .iter()
        .any(|cue| window.contains(&cue.to_lowercase()))
}

fn adjacent_symbol(source: &str, fact: &NumericFact, symbol: &str) -> Option<(usize, usize)> {
    let before = source.get(..fact.start_byte)?;
    let after = source.get(fact.end_byte..)?;
    let trimmed_before = before.trim_end();
    if trimmed_before.ends_with(symbol) {
        let gap = &before[trimmed_before.len()..];
        if gap.chars().all(char::is_whitespace) && !gap.contains('\n') {
            let start = trimmed_before.len() - symbol.len();
            if before[start..trimmed_before.len()].chars().count() == symbol.chars().count() {
                return Some((start, trimmed_before.len()));
            }
        }
    }
    let trimmed_after = after.trim_start();
    if trimmed_after.starts_with(symbol) {
        let gap = &after[..after.len() - trimmed_after.len()];
        if gap.chars().all(char::is_whitespace) && !gap.contains('\n') {
            let start = fact.end_byte + gap.len();
            let end = start + symbol.len();
            return Some((start, end));
        }
    }
    None
}

fn qualifier_for(source: &str, start: usize, data: &semantic_core::units::LanguageUnits) -> String {
    let prefix = &source[..start];
    let normalized_prefix = prefix.to_lowercase();
    let mut best_match: Option<(&str, usize)> = None;
    for (kind, cues) in &data.qualifiers {
        for cue in cues {
            let normalized_cue = cue.to_lowercase();
            let trimmed = normalized_prefix.trim_end();
            if !trimmed.ends_with(&normalized_cue) {
                continue;
            }
            let cue_start = trimmed.len() - normalized_cue.len();
            let has_boundary = cue_start == 0
                || !trimmed[..cue_start]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
            if has_boundary && best_match.is_none_or(|(_, length)| normalized_cue.len() > length) {
                best_match = Some((kind, normalized_cue.len()));
            }
        }
    }
    match best_match {
        Some(("range", _)) => "range_start".to_string(),
        Some((kind, _)) => kind.to_string(),
        None => "exact".to_string(),
    }
}

fn predicate_for(
    source: &str,
    sentence_index: usize,
    sentences: &[semantic_core::SentenceSpan],
    data: &semantic_core::units::LanguageUnits,
) -> Option<String> {
    let sentence = sentences.get(sentence_index)?.text(source);
    let sentence_words = word_spans(sentence);
    data.predicates.iter().find_map(|(predicate, cues)| {
        cues.iter()
            .any(|cue| {
                let cue_words = word_spans(cue);
                !cue_words.is_empty()
                    && sentence_words.windows(cue_words.len()).any(|window| {
                        window
                            .iter()
                            .zip(&cue_words)
                            .all(|(word, cue_word)| word.lowered == cue_word.lowered)
                    })
            })
            .then_some(predicate.clone())
    })
}

fn bind_period(
    source: &str,
    number_start: usize,
    number_end: usize,
    sentence_index: usize,
    sentences: &[semantic_core::SentenceSpan],
    dates: &[TemporalFact],
) -> Option<QuantityPeriod> {
    let sentence = sentences.get(sentence_index)?;
    dates
        .iter()
        .enumerate()
        .filter(|(_, date)| {
            sentence.start_byte <= date.start_byte && date.end_byte <= sentence.end_byte
        })
        .filter_map(|(fact_index, date)| {
            let gap = if date.start_byte >= number_end {
                source.get(number_end..date.start_byte)?
            } else if number_start >= date.end_byte {
                source.get(date.end_byte..number_start)?
            } else {
                return None;
            };
            let gap = gap.to_lowercase();
            let connected = gap.contains('(')
                || [" nel ", " nell'", " in ", " en ", " im ", " w ", " в "]
                    .iter()
                    .any(|cue| gap.contains(cue));
            if !connected {
                return None;
            }
            let year = date.normalized.as_deref()?.get(..4)?.parse().ok()?;
            Some((
                number_start.abs_diff(date.start_byte),
                QuantityPeriod { year, fact_index },
            ))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, period)| period)
}

#[cfg(test)]
#[path = "quantities_tests.rs"]
mod tests;
