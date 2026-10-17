//! Grounded analysis over extracted entities.
//!
//! Every output here is derived from [`extract`] and carries byte offsets into
//! the source text, so the same `NO EVIDENCE → NO ENTITY` rule applies to the
//! derived structures: a relation, lower-third, bullet or chart point is only
//! emitted for a sentence that exists verbatim in the source.
//!
//! Entities are referenced by their index in `Analysis::entities`.

use crate::extractor::{extract, numeric_value_spans};
use crate::temporal::temporal_facts;
use crate::types::{ExtractOptions, VisualEntity};
use serde::Serialize;

/// Full analysis of one segment of source text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Analysis {
    pub entities: Vec<VisualEntity>,
    /// Pairs of entities that occur in the same sentence.
    pub relations: Vec<EntityRelation>,
    /// One lower-third spec per entity.
    pub lower_thirds: Vec<LowerThird>,
    /// One bullet per sentence that mentions at least one entity.
    pub bullets: Vec<Bullet>,
    /// Numeric entities whose value is unambiguous (e.g. `25%`, `$1200`).
    pub chart_points: Vec<ChartPoint>,
    /// Every source-grounded date occurrence, independent of the visual
    /// top-N budget, with calendar normalization and precision. A scene with
    /// five dates must not lose facts before PipelineGen can build a
    /// timeline: this projection feeds the composed date-timeline groups.
    pub temporal_facts: Vec<crate::temporal::TemporalFact>,
    /// Every numeric-value fact (NUMBER/PERCENT/MONEY with its parse when
    /// unambiguous), independent of the visual top-N budget. Charts and
    /// compared statistics consume this list so a scene with more numbers
    /// than the entity budget loses none of them.
    pub numeric_facts: Vec<NumericFact>,
}

/// One grounded numeric-value fact. `value` is parsed only when the surface
/// is unambiguous for the scene language (`25%`, `$1,200`, `12,5% euro`); a
/// scaled quantity such as `2 million` carries no raw value instead of a
/// guessed one. The killer rule is inherited: surface == evidence == source
/// slice at `[start_byte, end_byte)`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NumericFact {
    /// Exact source form (e.g. "25%").
    pub surface: String,
    /// One of `NUMBER`, `PERCENT`, `MONEY`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Parsed numeric value when unambiguous; `None` otherwise.
    pub value: Option<f64>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityRelation {
    pub source: usize,
    pub target: usize,
    pub sentence_index: usize,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LowerThird {
    pub entity: usize,
    pub title: String,
    pub kind: String,
    pub sentence_index: usize,
    pub evidence: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bullet {
    pub sentence_index: usize,
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub entities: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartPoint {
    pub entity: usize,
    pub kind: String,
    pub value: f64,
    pub label: String,
    pub sentence_index: usize,
}

/// A sentence span `[start, end)` of the source text, already trimmed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sentence {
    start: usize,
    end: usize,
}

/// Extracts entities and derives the analysis structures from them.
pub fn analyze(source_text: &str, options: &ExtractOptions) -> Result<Analysis, String> {
    let entities = extract(source_text, options)?;
    let sentences = split_sentences(source_text);
    let entity_sentence: Vec<Option<usize>> = entities
        .iter()
        .map(|entity| sentence_index_of(&sentences, entity.start))
        .collect();

    let relations = relations(source_text, &entities, &sentences, &entity_sentence);
    let lower_thirds = entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| {
            let sentence_index = entity_sentence[index]?;
            Some(LowerThird {
                entity: index,
                title: entity.text.clone(),
                kind: entity.r#type.clone(),
                sentence_index,
                evidence: entity.evidence.clone(),
                start: entity.start,
                end: entity.end,
            })
        })
        .collect();
    let bullets = bullets(source_text, &sentences, &entity_sentence);
    let chart_points = chart_points(
        source_text,
        &entities,
        &sentences,
        &entity_sentence,
        &options.language,
    );
    let temporal_facts = temporal_facts(source_text, &options.language);
    let numeric_facts = numeric_facts(source_text, &options.language);

    Ok(Analysis {
        entities,
        relations,
        lower_thirds,
        bullets,
        chart_points,
        temporal_facts,
        numeric_facts,
    })
}

fn numeric_facts(source_text: &str, language: &str) -> Vec<NumericFact> {
    numeric_value_spans(source_text)
        .into_iter()
        .filter_map(|span| {
            let surface = source_text.get(span.start..span.end)?;
            Some(NumericFact {
                surface: surface.to_string(),
                kind: span.entity_type,
                value: parse_unambiguous_value(surface, language),
                start_byte: span.start,
                end_byte: span.end,
                evidence: surface.to_string(),
            })
        })
        .collect()
}

fn relations(
    source_text: &str,
    entities: &[VisualEntity],
    sentences: &[Sentence],
    entity_sentence: &[Option<usize>],
) -> Vec<EntityRelation> {
    let mut out = Vec::new();
    for source in 0..entities.len() {
        for target in source + 1..entities.len() {
            let (Some(a), Some(b)) = (entity_sentence[source], entity_sentence[target]) else {
                continue;
            };
            if a != b || same_surface(&entities[source], &entities[target]) {
                continue;
            }
            let sentence = sentences[a];
            out.push(EntityRelation {
                source,
                target,
                sentence_index: a,
                evidence: source_text[sentence.start..sentence.end].to_string(),
            });
        }
    }
    out
}

fn bullets(
    source_text: &str,
    sentences: &[Sentence],
    entity_sentence: &[Option<usize>],
) -> Vec<Bullet> {
    sentences
        .iter()
        .enumerate()
        .filter_map(|(index, sentence)| {
            let entities: Vec<usize> = entity_sentence
                .iter()
                .enumerate()
                .filter(|(_, s)| **s == Some(index))
                .map(|(entity, _)| entity)
                .collect();
            if entities.is_empty() {
                return None;
            }
            Some(Bullet {
                sentence_index: index,
                text: source_text[sentence.start..sentence.end].to_string(),
                start: sentence.start,
                end: sentence.end,
                entities,
            })
        })
        .collect()
}

fn chart_points(
    source_text: &str,
    entities: &[VisualEntity],
    sentences: &[Sentence],
    entity_sentence: &[Option<usize>],
    language: &str,
) -> Vec<ChartPoint> {
    let numeric = |entity: &VisualEntity| {
        matches!(entity.r#type.as_str(), "NUMBER" | "PERCENT" | "MONEY")
    };
    entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| numeric(entity))
        // A bare NUMBER inside a larger numeric span ("25" in "25%", "1,200"
        // in "$1,200") is the same quantity, so keep only the outer entity.
        // Only drop it when the outer span actually yields a value; otherwise
        // ("3.141" inside "3.141 units") the inner number is the value.
        .filter(|(_, entity)| {
            !(entity.r#type == "NUMBER"
                && entities.iter().any(|other| {
                    numeric(other)
                        && other.start <= entity.start
                        && entity.end <= other.end
                        && (other.start, other.end) != (entity.start, entity.end)
                        && parse_unambiguous_value(&other.text, language).is_some()
                }))
        })
        .filter_map(|(index, entity)| {
            let sentence_index = entity_sentence[index]?;
            let value = parse_unambiguous_value(&entity.text, language)?;
            let sentence = sentences[sentence_index];
            Some(ChartPoint {
                entity: index,
                kind: entity.r#type.clone(),
                value,
                label: source_text[sentence.start..sentence.end].to_string(),
                sentence_index,
            })
        })
        .collect()
}

/// Parses a numeric surface such as `25%`, `$1,200`, `1.200 euro`, `12,5%`,
/// `3.5` or `25 percent`, using the separator convention of `language`:
/// English groups thousands with `,` and uses `.` as decimal point; the
/// other supported languages (it, de, fr, es, pt, pl, ru, tr, id) use `.` for
/// thousands and `,` as decimal point. Returns `None` for scale words
/// (`million`) and for any grouping that does not match the convention, so a
/// chart never plots a guessed value.
fn parse_unambiguous_value(text: &str, language: &str) -> Option<f64> {
    let mut rest = text.trim().to_lowercase();
    for suffix in ["percent", "dollars", "euros", "euro", "pounds", "yen"] {
        if let Some(stripped) = rest.strip_suffix(suffix) {
            rest = stripped.trim_end().to_string();
            break;
        }
    }
    let rest = rest
        .trim_matches(|c: char| matches!(c, '%' | '$' | '£' | '€' | '¥') || c.is_whitespace());
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',') {
        return None;
    }
    let (thousands, decimal) = if language.eq_ignore_ascii_case("en") {
        (',', '.')
    } else {
        ('.', ',')
    };
    let (int_part, frac_part) = match rest.matches(decimal).count() {
        0 => (rest, None),
        1 => {
            let (int_part, frac) = rest.split_once(decimal)?;
            (int_part, Some(frac))
        }
        _ => return None,
    };
    if matches!(frac_part, Some(frac) if frac.is_empty() || frac.contains(thousands)) {
        return None;
    }
    // A lone decimal-comma followed by exactly three digits ("1,200" in an
    // Italian sentence) reads as 1.2 by convention, but is the usual way an
    // English thousands group is written, so it is refused rather than guessed.
    if decimal == ',' && !rest.contains(thousands) && frac_part.is_some_and(|frac| frac.len() == 3) {
        return None;
    }
    let int_digits = if int_part.contains(thousands) {
        let groups: Vec<&str> = int_part.split(thousands).collect();
        let (first, rest_groups) = groups.split_first()?;
        let valid = (1..=3).contains(&first.len())
            && rest_groups.iter().all(|group| group.len() == 3);
        if !valid {
            return None;
        }
        groups.concat()
    } else {
        int_part.to_string()
    };
    if int_digits.is_empty() {
        return None;
    }
    let number = match frac_part {
        Some(frac) => format!("{int_digits}.{frac}"),
        None => int_digits,
    };
    number.parse::<f64>().ok()
}

fn same_surface(a: &VisualEntity, b: &VisualEntity) -> bool {
    a.text.to_lowercase() == b.text.to_lowercase()
}

fn sentence_index_of(sentences: &[Sentence], offset: usize) -> Option<usize> {
    sentences
        .iter()
        .position(|sentence| sentence.start <= offset && offset < sentence.end)
}

/// Splits on `.`, `!`, `?` and newlines. A `.` between two digits (e.g. `3.5`)
/// does not end a sentence.
fn split_sentences(source_text: &str) -> Vec<Sentence> {
    let bytes = source_text.as_bytes();
    let mut sentences = Vec::new();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let is_break = match b {
            b'!' | b'?' | b'\n' => true,
            b'.' => {
                let digit_before = i > 0 && bytes[i - 1].is_ascii_digit();
                let digit_after = bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit());
                !(digit_before && digit_after)
            }
            _ => false,
        };
        if is_break {
            push_trimmed(source_text, start, i + 1, &mut sentences);
            start = i + 1;
        }
    }
    push_trimmed(source_text, start, bytes.len(), &mut sentences);
    sentences
}

fn push_trimmed(source_text: &str, start: usize, end: usize, out: &mut Vec<Sentence>) {
    let raw = &source_text[start..end];
    let trimmed_start = start + (raw.len() - raw.trim_start().len());
    let trimmed_end = trimmed_start + raw.trim().len();
    if trimmed_end > trimmed_start {
        out.push(Sentence {
            start: trimmed_start,
            end: trimmed_end,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "Sales rose 25% in Lisbon. Fado singers perform in Alfama.";

    fn options() -> ExtractOptions {
        ExtractOptions {
            language: "en".to_string(),
            entity_count: 10,
        }
    }

    #[test]
    fn every_derived_span_is_verbatim_source() {
        let analysis = analyze(SOURCE, &options()).unwrap();
        for lt in &analysis.lower_thirds {
            assert_eq!(&SOURCE[lt.start..lt.end], lt.evidence);
        }
        for bullet in &analysis.bullets {
            assert_eq!(&SOURCE[bullet.start..bullet.end], bullet.text);
        }
    }

    #[test]
    fn numeric_facts_survive_a_low_entity_budget() {
        // The plan's second killer scenario: more numbers than the visual
        // top-N budget. NumericFacts must carry every numeric occurrence.
        let text = "Il team contava 15 persone, il budget copriva il 42%, i costi passarono da 3 ad 8, e il 2024 chiuse con 1,2 milioni di euro.";
        let analysis = analyze(
            text,
            &ExtractOptions {
                language: "it".to_string(),
                entity_count: 3,
            },
        )
        .unwrap();
        for fact in &analysis.numeric_facts {
            assert_eq!(
                text.get(fact.start_byte..fact.end_byte),
                Some(fact.surface.as_str()),
                "surface must be verbatim at its offsets"
            );
            assert_eq!(fact.evidence, fact.surface);
            assert!(matches!(fact.kind.as_str(), "NUMBER" | "PERCENT" | "MONEY"));
        }
        let surfaces: Vec<&str> = analysis
            .numeric_facts
            .iter()
            .map(|fact| fact.surface.as_str())
            .collect();
        assert!(surfaces.contains(&"42%"), "facts: {surfaces:?}");
        assert!(surfaces.contains(&"15"), "facts: {surfaces:?}");
        // A 4-digit year classifies as DATE, not NUMBER: the temporal
        // projection owns it, so the numeric projection must not duplicate
        // it (one fact, one projection).
        assert!(!surfaces.contains(&"2024"), "facts: {surfaces:?}");
    }

    #[test]
    fn numeric_facts_parse_units_by_language_convention() {
        let en = analyze(
            "Revenue hit $1,200 and 25% growth.",
            &ExtractOptions {
                language: "en".to_string(),
                entity_count: 3,
            },
        )
        .unwrap();
        let money = en
            .numeric_facts
            .iter()
            .find(|fact| fact.kind == "MONEY")
            .expect("$1,200 must produce a MONEY fact");
        assert_eq!(money.value, Some(1200.0));
        let percent = en
            .numeric_facts
            .iter()
            .find(|fact| fact.kind == "PERCENT" && fact.value == Some(25.0));
        assert!(percent.is_some(), "facts: {:?}", en.numeric_facts);

        // Italian decimal comma: 12,5% parses to 12.5.
        let it = analyze(
            "Le vendite crebbero del 12,5%.",
            &ExtractOptions {
                language: "it".to_string(),
                entity_count: 3,
            },
        )
        .unwrap();
        let percent = it
            .numeric_facts
            .iter()
            .find(|fact| fact.kind == "PERCENT")
            .expect("12,5% must produce a PERCENT fact");
        assert_eq!(percent.value, Some(12.5));
    }

    #[test]
    fn relations_stay_inside_one_sentence() {
        let analysis = analyze(SOURCE, &options()).unwrap();
        for rel in &analysis.relations {
            let a = &analysis.entities[rel.source];
            let b = &analysis.entities[rel.target];
            assert!(rel.evidence.contains(&a.text));
            assert!(rel.evidence.contains(&b.text));
        }
    }

    #[test]
    fn one_lower_third_per_entity_and_chart_value_is_plain() {
        let analysis = analyze(SOURCE, &options()).unwrap();
        assert_eq!(analysis.lower_thirds.len(), analysis.entities.len());
        let percent = analysis
            .chart_points
            .iter()
            .find(|point| point.kind == "PERCENT")
            .expect("25% must be a chart point");
        assert_eq!(percent.value, 25.0);
    }

    #[test]
    fn parses_only_plain_numeric_surfaces() {
        assert_eq!(parse_unambiguous_value("25%", "en"), Some(25.0));
        assert_eq!(parse_unambiguous_value("$1,200", "en"), Some(1200.0));
        assert_eq!(parse_unambiguous_value("3.5", "en"), Some(3.5));
        assert_eq!(parse_unambiguous_value("25 percent", "en"), Some(25.0));
        assert_eq!(parse_unambiguous_value("1.5 million", "en"), None);
        assert_eq!(parse_unambiguous_value("March 2025", "en"), None);
    }

    #[test]
    fn decimal_comma_languages_use_dot_thousands() {
        assert_eq!(parse_unambiguous_value("1.200 euro", "it"), Some(1200.0));
        assert_eq!(parse_unambiguous_value("1.200.000", "de"), Some(1_200_000.0));
        assert_eq!(parse_unambiguous_value("12,5%", "it"), Some(12.5));
        // A lone comma followed by three digits is ambiguous in Italian: refuse.
        assert_eq!(parse_unambiguous_value("1,200", "it"), None);
        assert_eq!(parse_unambiguous_value("$1,200", "it"), None);
        assert_eq!(parse_unambiguous_value("1,25", "it"), Some(1.25));
        // Not a valid grouping in either convention: refuse rather than guess.
        assert_eq!(parse_unambiguous_value("3.5", "it"), None);
        assert_eq!(parse_unambiguous_value("1.2.3", "it"), None);
        assert_eq!(parse_unambiguous_value("1,2,3", "it"), None);
    }

    #[test]
    fn overlapping_numeric_spans_yield_one_chart_point() {
        let source = "Costs rose 25% to $1,200 in 2025.";
        let entities = vec![
            VisualEntity { text: "25".into(), r#type: "NUMBER".into(), start: 12, end: 14, score: 1.0, evidence: String::new() },
            VisualEntity { text: "25%".into(), r#type: "PERCENT".into(), start: 12, end: 15, score: 1.0, evidence: String::new() },
            VisualEntity { text: "$1,200".into(), r#type: "MONEY".into(), start: 19, end: 25, score: 1.0, evidence: String::new() },
            VisualEntity { text: "1,200".into(), r#type: "NUMBER".into(), start: 20, end: 25, score: 1.0, evidence: String::new() },
        ];
        let sentences = split_sentences(source);
        let owners = vec![Some(0); entities.len()];
        let points = chart_points(source, &entities, &sentences, &owners, "en");
        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        assert_eq!(values, vec![25.0, 1200.0]);
    }

    #[test]
    fn inner_number_survives_when_outer_span_has_no_value() {
        let source = "Units: 3.25 units sold.";
        let entities = vec![
            VisualEntity { text: "3.25 units".into(), r#type: "NUMBER".into(), start: 7, end: 17, score: 1.0, evidence: String::new() },
            VisualEntity { text: "3.25".into(), r#type: "NUMBER".into(), start: 7, end: 11, score: 1.0, evidence: String::new() },
        ];
        let sentences = split_sentences(source);
        let owners = vec![Some(0); entities.len()];
        let points = chart_points(source, &entities, &sentences, &owners, "en");
        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        assert_eq!(values, vec![3.25]);
    }

    #[test]
    fn decimal_point_does_not_split_sentences() {
        let sentences = split_sentences("Pi is 3.14. Done");
        assert_eq!(sentences.len(), 2);
        assert_eq!(&"Pi is 3.14. Done"[sentences[0].start..sentences[0].end], "Pi is 3.14.");
    }
}
