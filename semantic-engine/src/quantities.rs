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
        let Some((local_start, local_end)) =
            semantic_core::discourse::comparison_cue(sentence.text(source))
        else {
            continue;
        };
        let cue_start = sentence.start_byte + local_start;
        let cue_end = sentence.start_byte + local_end;
        let [left, right, ..] = sentence_quantities.as_slice() else {
            continue;
        };
        if cue_start < left.end_byte || cue_end > right.start_byte {
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
        let cue = sentence.text(source).to_lowercase();
        let ratio_cue = ["doppio", "triplic", "double", "triple", "twice"]
            .iter()
            .any(|term| cue.contains(term));
        out.push(QuantityComparison {
            kind: if ratio_cue { "ratio" } else { "difference" }.to_string(),
            left: left.id.clone(),
            right: right.id.clone(),
            dimension: left.dimension.clone(),
            ratio: (ratio_cue && right_value != 0.0).then_some(left_value / right_value),
            difference: (!ratio_cue).then_some(left_value - right_value),
            evidence_start_byte: cue_start,
            evidence_end_byte: cue_end,
        });
    }
    out
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
        && [" and ", " e ", " a ", " to ", " y ", " et ", " und ", " i "]
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
            if has_boundary {
                return if kind == "range" {
                    "range_start".to_string()
                } else {
                    kind.clone()
                };
            }
        }
    }
    "exact".to_string()
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
mod tests {
    use super::*;
    use serde_json::json;

    fn fact(source: &str, surface: &str, kind: &str, value: Option<f64>) -> NumericFact {
        let start = source.find(surface).unwrap();
        NumericFact {
            surface: surface.to_string(),
            kind: kind.to_string(),
            value,
            start_byte: start,
            end_byte: start + surface.len(),
            evidence: surface.to_string(),
            form: None,
        }
    }

    #[test]
    fn attaches_currency_scale_predicate_and_same_sentence_year() {
        let source = "Il fatturato è 3,2 miliardi di euro nel 2019.";
        let number = fact(source, "3,2 miliardi", "MONEY", Some(3_200_000_000.0));
        let date = semantic_core::temporal_facts(source, "it");
        let quantities = extract(source, "it", &[number], &date);
        assert_eq!(quantities.len(), 1);
        let quantity = &quantities[0];
        assert_eq!(quantity.dimension, "currency");
        assert_eq!(quantity.unit, "EUR");
        assert_eq!(quantity.scale, Some(1_000_000_000.0));
        assert_eq!(quantity.value, Some(3_200_000_000.0));
        assert_eq!(quantity.predicate.as_deref(), Some("revenue"));
        assert_eq!(
            quantity.period.as_ref().map(|period| period.year),
            Some(2019)
        );
        assert_eq!(
            &source[quantity.start_byte..quantity.end_byte],
            quantity.evidence
        );
    }

    #[test]
    fn ignores_years_without_units_and_does_not_cross_sentence_boundaries() {
        let source = "Nel 2019 la popolazione è cresciuta. Il fatturato era 3 milioni di euro.";
        let number = fact(source, "3", "NUMBER", Some(3.0));
        let dates = semantic_core::temporal_facts(source, "it");
        let quantities = extract(source, "it", &[number], &dates);
        assert_eq!(quantities.len(), 1);
        assert_eq!(quantities[0].dimension, "currency");
        assert!(quantities[0].period.is_none());
    }

    #[test]
    fn a_later_quantity_cannot_supply_the_previous_number_scale() {
        let source = "Il gruppo ha 2 euro contro 3 milioni di euro.";
        let facts = [
            fact(source, "2", "NUMBER", Some(2.0)),
            fact(source, "3", "NUMBER", Some(3.0)),
        ];
        let quantities = extract(source, "it", &facts, &[]);
        assert_eq!(quantities.len(), 2);
        assert_eq!(quantities[0].scale, None);
        assert_eq!(quantities[0].value, Some(2.0));
        assert_eq!(quantities[1].scale, Some(1_000_000.0));
        assert_eq!(quantities[1].value, Some(3_000_000.0));
    }

    #[test]
    fn uses_the_language_unit_table_for_population_and_percent() {
        let population = "La città ha 3 milioni di abitanti.";
        let q = extract(
            population,
            "it",
            &[fact(population, "3", "NUMBER", Some(3.0))],
            &[],
        );
        assert_eq!(q[0].dimension, "population");
        assert_eq!(q[0].unit, "person");
        let percentage = "Revenue grew by 12%.";
        let q = extract(
            percentage,
            "en",
            &[fact(percentage, "12%", "PERCENT", Some(12.0))],
            &[],
        );
        assert_eq!(q[0].dimension, "percent");
    }

    #[test]
    fn unknown_languages_do_not_borrow_another_language_lexicon() {
        let source = json!("The city has 3 million inhabitants.")
            .as_str()
            .unwrap()
            .to_string();
        let number = fact(&source, "3", "NUMBER", Some(3.0));
        assert!(extract(&source, "ru", &[number], &[]).is_empty());
    }

    #[test]
    fn comparisons_require_the_same_dimension_and_currency_year() {
        let source = "Il gruppo A ha 42% contro il 18% del gruppo B.";
        let facts = [
            fact(source, "42%", "PERCENT", Some(42.0)),
            fact(source, "18%", "PERCENT", Some(18.0)),
        ];
        let quantities = extract(source, "it", &facts, &[]);
        let found_comparisons = comparisons(source, "it", &quantities);
        assert_eq!(found_comparisons.len(), 1);
        assert_eq!(found_comparisons[0].kind, "difference");
        assert_eq!(found_comparisons[0].difference, Some(24.0));

        let mixed = "Il gruppo A ha 42% contro 18 euro.";
        let facts = [
            fact(mixed, "42%", "PERCENT", Some(42.0)),
            fact(mixed, "18", "MONEY", Some(18.0)),
        ];
        let quantities = extract(mixed, "it", &facts, &[]);
        assert!(comparisons(mixed, "it", &quantities).is_empty());

        let cross_currency = "Il valore è 10 EUR contro 5 USD.";
        let facts = [
            fact(cross_currency, "10", "MONEY", Some(10.0)),
            fact(cross_currency, "5", "MONEY", Some(5.0)),
        ];
        let quantities = extract(cross_currency, "it", &facts, &[]);
        assert!(comparisons(cross_currency, "it", &quantities).is_empty());

        let ratio = "Il valore è 10% rispetto al doppio di 5%.";
        let facts = [
            fact(ratio, "10%", "PERCENT", Some(10.0)),
            fact(ratio, "5%", "PERCENT", Some(5.0)),
        ];
        let quantities = extract(ratio, "it", &facts, &[]);
        let found_comparisons = comparisons(ratio, "it", &quantities);
        assert_eq!(found_comparisons.len(), 1);
        assert_eq!(found_comparisons[0].kind, "ratio");
        assert_eq!(found_comparisons[0].ratio, Some(2.0));
    }

    #[test]
    fn common_currency_units_are_attached_in_every_supported_language() {
        let currencies = [
            ("de", "euro"),
            ("en", "euro"),
            ("es", "euro"),
            ("fr", "euro"),
            ("id", "euro"),
            ("it", "euro"),
            ("nl", "euro"),
            ("pl", "euro"),
            ("pt", "euro"),
            ("ru", "евро"),
            ("tr", "euro"),
        ];
        for (language, surface) in currencies {
            let source = format!("5 {surface}");
            let number = fact(&source, "5", "MONEY", Some(5.0));
            let quantities = extract(&source, language, &[number], &[]);
            assert_eq!(
                quantities.first().map(|quantity| quantity.unit.as_str()),
                Some("EUR"),
                "language={language}, source={source}"
            );
        }
    }

    #[test]
    fn compound_speed_units_win_over_the_length_prefix_in_every_language() {
        for language in [
            "de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr",
        ] {
            let unit = language_units(language)
                .unwrap()
                .units
                .iter()
                .find(|unit| unit.dimension == "speed")
                .unwrap();
            let surface = unit
                .surfaces
                .iter()
                .find(|surface| surface.contains('/'))
                .unwrap();
            let source = format!("5 {surface}");
            let quantities = extract(
                &source,
                language,
                &[fact(&source, "5", "NUMBER", Some(5.0))],
                &[],
            );
            assert_eq!(quantities.len(), 1, "language={language}, source={source}");
            assert_eq!(quantities[0].dimension, "speed", "{source}");
            assert_eq!(quantities[0].unit, "km/h", "{source}");
        }
    }

    #[test]
    fn years_and_frequency_counts_do_not_become_typed_quantities() {
        let year = "Nel 2019 la popolazione è cresciuta.";
        let year_fact = fact(year, "2019", "NUMBER", Some(2019.0));
        assert!(extract(year, "it", &[year_fact], &[]).is_empty());

        let frequency = "La squadra ha vinto 3 volte.";
        let count = fact(frequency, "3", "NUMBER", Some(3.0));
        assert!(extract(frequency, "it", &[count], &[]).is_empty());
    }

    #[test]
    fn a_range_can_share_one_unit_but_a_comparison_cannot_steal_the_later_unit() {
        let source = "Il percorso misura tra 3 e 5 km.";
        let facts = [
            fact(source, "3", "NUMBER", Some(3.0)),
            fact(source, "5", "NUMBER", Some(5.0)),
        ];
        let quantities = extract(source, "it", &facts, &[]);
        assert_eq!(quantities.len(), 2);
        assert_eq!(quantities[0].unit, "km");
        assert_eq!(quantities[1].unit, "km");
        assert_eq!(quantities[0].qualifier, "range_start");
        assert_eq!(quantities[1].qualifier, "range_end");
        assert_eq!(quantities[0].range_id, quantities[1].range_id);

        let scaled_range =
            "Il costo previsto è compreso tra 10 e 15 milioni di euro, rispetto agli 8 milioni di budget.";
        let facts = [
            fact(scaled_range, "10", "NUMBER", Some(10.0)),
            fact(scaled_range, "15 milioni", "NUMBER", Some(15_000_000.0)),
            fact(scaled_range, "8 milioni", "NUMBER", Some(8_000_000.0)),
        ];
        let quantities = extract(scaled_range, "it", &facts, &[]);
        assert_eq!(quantities.len(), 2, "quantities: {quantities:#?}");
        assert_eq!(quantities[0].qualifier, "range_start");
        assert_eq!(quantities[0].scale, Some(1_000_000.0));
        assert_eq!(quantities[0].value, Some(10_000_000.0));
        assert_eq!(quantities[1].value, Some(15_000_000.0));

        let comparison = "Il gruppo ha 3 contro 5 km.";
        let facts = [
            fact(comparison, "3", "NUMBER", Some(3.0)),
            fact(comparison, "5", "NUMBER", Some(5.0)),
        ];
        let quantities = extract(comparison, "it", &facts, &[]);
        assert_eq!(
            quantities.len(),
            1,
            "the first value must not steal the later unit"
        );
        assert_eq!(quantities[0].evidence, "5");
    }
}
