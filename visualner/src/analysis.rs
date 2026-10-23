//! Grounded analysis over extracted entities.
//!
//! Every output here is derived from [`extract`] and carries byte offsets into
//! the source text, so the same `NO EVIDENCE → NO ENTITY` rule applies to the
//! derived structures: a relation, lower-third, bullet or chart point is only
//! emitted for a sentence that exists verbatim in the source.
//!
//! Entities are referenced by their index in `Analysis::entities`.

use crate::extractor::{extract, numeric_value_spans};
use crate::temporal::{temporal_facts_with_words, SpelledNumber};
use crate::types::{ExtractOptions, VisualEntity};
use semantic_core::bullets::{generate_with_sentences, BulletSignal, SignalKind};
use semantic_core::numbers::parse_unambiguous_value;
use semantic_core::sentences::{split_sentences, SentenceSpan};
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
    /// Unambiguous numeric values (e.g. `25%`, `$1200`), projected from the
    /// single [`Analysis::numeric_facts`] collection. There is no second
    /// number scanner: a chart point is a typed view over a grounded fact.
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

/// One grounded numeric-value fact. The type is shared with `semantic-core`
/// so every extractor agrees on the parse: `value` is set only when the
/// surface is unambiguous for the scene language. Re-exported to keep the
/// `visualner::analysis::NumericFact` path stable for callers.
pub use semantic_core::numbers::NumericFact;

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
    /// Shared editorial salience from `semantic_core::bullets`. It ranks
    /// candidates only; it is not a confidence and never admits a new span.
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartPoint {
    /// Exact numeric surface (e.g. `"25%"`).
    pub text: String,
    pub kind: String,
    pub value: f64,
    /// The sentence the value appears in (verbatim).
    pub label: String,
    pub sentence_index: usize,
    pub start: usize,
    pub end: usize,
    /// Index of the matching NER entity, when one survived the visual top-N.
    /// A chart is admitted from the numeric fact, never from this link.
    pub entity: Option<usize>,
}

/// Extracts entities and derives the analysis structures from them.
pub fn analyze(source_text: &str, options: &ExtractOptions) -> Result<Analysis, String> {
    let entities = extract(source_text, options)?;
    // One canonical segmenter for every extractor (semantic-core).
    let sentences = split_sentences(source_text, &options.language, None);
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
    let bullets = bullets(
        source_text,
        entities.as_slice(),
        &sentences,
        &entity_sentence,
    );
    // ONE numeric collection, two typed views: `numeric_facts` keeps every
    // grounded value (top-N independent) and `chart_points` is the subset with
    // an unambiguous parse. No second scanner can disagree with the first.
    let numeric_facts = numeric_facts(source_text, &options.language);
    let chart_points = chart_points(source_text, &numeric_facts, &sentences, &entities);
    // The date parser never spells a word itself: the locale's ICU RBNF
    // rules locate "tredici", "duemilasette" or "two thousand seven", and
    // semantic-core only resolves the calendar around the month token.
    let spelled_numbers: Vec<SpelledNumber> =
        crate::spellout::candidates(source_text, &options.language)
            .unwrap_or_default()
            .into_iter()
            .map(|candidate| SpelledNumber {
                start: candidate.start,
                end: candidate.end,
                value: candidate.value.round() as i64,
            })
            .collect();
    let temporal_facts =
        temporal_facts_with_words(source_text, &options.language, &spelled_numbers);

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
    sentences: &[SentenceSpan],
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
                evidence: source_text[sentence.start_byte..sentence.end_byte].to_string(),
            });
        }
    }
    out
}

/// Projects the shared bullet candidates (one generation and one scoring
/// algorithm, owned by `semantic-core`) into VisualNER's wire type. The
/// entity indices stay scene-local; the score is the shared salience.
fn bullets(
    source_text: &str,
    entities: &[VisualEntity],
    sentences: &[SentenceSpan],
    entity_sentence: &[Option<usize>],
) -> Vec<Bullet> {
    let signals: Vec<BulletSignal> = entities
        .iter()
        .map(|entity| BulletSignal {
            start_byte: entity.start,
            end_byte: entity.end,
            kind: if is_measurement(&entity.r#type) {
                SignalKind::Measurement
            } else {
                SignalKind::Visual
            },
        })
        .collect();
    generate_with_sentences(source_text, sentences, &signals)
        .into_iter()
        .map(|candidate| {
            let entities: Vec<usize> = entity_sentence
                .iter()
                .enumerate()
                .filter(|(_, sentence)| **sentence == Some(candidate.sentence_index))
                .map(|(entity, _)| entity)
                .collect();
            Bullet {
                sentence_index: candidate.sentence_index,
                text: candidate.text,
                start: candidate.start_byte,
                end: candidate.end_byte,
                entities,
                score: candidate.score,
            }
        })
        .collect()
}

fn is_measurement(entity_type: &str) -> bool {
    matches!(entity_type, "NUMBER" | "PERCENT" | "MONEY" | "DATE")
}

/// Projects the unambiguous subset of `numeric_facts` into chart points.
///
/// The numeric facts are the only scanner; a chart point adds the sentence it
/// belongs to and the matching NER entity index (when one survived the top-N
/// visual budget). Because the fact collection is top-N independent, a scene
/// with more numbers than entities no longer loses chart values.
fn chart_points(
    source_text: &str,
    numeric_facts: &[NumericFact],
    sentences: &[SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<ChartPoint> {
    numeric_facts
        .iter()
        .filter_map(|fact| {
            let value = fact.value?;
            let sentence_index = sentence_index_of(sentences, fact.start_byte)?;
            let sentence = sentences[sentence_index];
            let entity = entities
                .iter()
                .position(|entity| entity.start == fact.start_byte && entity.end == fact.end_byte);
            Some(ChartPoint {
                text: fact.surface.clone(),
                kind: fact.kind.clone(),
                value,
                label: source_text[sentence.start_byte..sentence.end_byte].to_string(),
                sentence_index,
                start: fact.start_byte,
                end: fact.end_byte,
                entity,
            })
        })
        .collect()
}

fn same_surface(a: &VisualEntity, b: &VisualEntity) -> bool {
    a.text.to_lowercase() == b.text.to_lowercase()
}

fn sentence_index_of(sentences: &[SentenceSpan], offset: usize) -> Option<usize> {
    sentences
        .iter()
        .position(|sentence| sentence.start_byte <= offset && offset < sentence.end_byte)
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
        assert_eq!(
            parse_unambiguous_value("1.200.000", "de"),
            Some(1_200_000.0)
        );
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
            VisualEntity {
                text: "25".into(),
                r#type: "NUMBER".into(),
                start: 12,
                end: 14,
                score: 1.0,
                evidence: String::new(),
            },
            VisualEntity {
                text: "25%".into(),
                r#type: "PERCENT".into(),
                start: 12,
                end: 15,
                score: 1.0,
                evidence: String::new(),
            },
            VisualEntity {
                text: "$1,200".into(),
                r#type: "MONEY".into(),
                start: 19,
                end: 25,
                score: 1.0,
                evidence: String::new(),
            },
            VisualEntity {
                text: "1,200".into(),
                r#type: "NUMBER".into(),
                start: 20,
                end: 25,
                score: 1.0,
                evidence: String::new(),
            },
        ];
        let sentences = split_sentences(source, "en", None);
        let facts = numeric_facts(source, "en");
        let points = chart_points(source, &facts, &sentences, &entities);
        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        assert_eq!(values, vec![25.0, 1200.0]);
    }

    #[test]
    fn inner_number_survives_when_outer_span_has_no_value() {
        let source = "Units: 3.25 units sold.";
        let entities = vec![
            VisualEntity {
                text: "3.25 units".into(),
                r#type: "NUMBER".into(),
                start: 7,
                end: 17,
                score: 1.0,
                evidence: String::new(),
            },
            VisualEntity {
                text: "3.25".into(),
                r#type: "NUMBER".into(),
                start: 7,
                end: 11,
                score: 1.0,
                evidence: String::new(),
            },
        ];
        let sentences = split_sentences(source, "en", None);
        let facts = numeric_facts(source, "en");
        let points = chart_points(source, &facts, &sentences, &entities);
        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        assert_eq!(values, vec![3.25]);
    }

    #[test]
    fn decimal_point_does_not_split_sentences() {
        let sentences = split_sentences("Pi is 3.14. Done", "en", None);
        assert_eq!(sentences.len(), 2);
        assert_eq!(
            &"Pi is 3.14. Done"[sentences[0].start_byte..sentences[0].end_byte],
            "Pi is 3.14."
        );
    }
}
