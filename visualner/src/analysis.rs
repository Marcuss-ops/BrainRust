//! Grounded analysis over extracted entities.
//!
//! Every output here is derived from [`extract`] and carries byte offsets into
//! the source text, so the same `NO EVIDENCE → NO ENTITY` rule applies to the
//! derived structures: a relation, lower-third, bullet or chart point is only
//! emitted for a sentence that exists verbatim in the source.
//!
//! Entities are referenced by their index in `Analysis::entities`.

use crate::extractor::numeric_value_spans;
use crate::temporal::{SpelledNumber, TemporalFact};
use crate::types::{ExtractOptions, VisualEntity};
use semantic_core::bullets::{generate_with_sentences, BulletSignal};
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
    analyze_with_context(
        source_text,
        options,
        &crate::extractor::ScriptContext::default(),
    )
}

/// Same analysis with script-wide evidence: the engine passes the name tokens
/// of the whole script so a scene that says only "Chiara" does not turn a
/// person's first name into a place.
pub fn analyze_with_context(
    source_text: &str,
    options: &ExtractOptions,
    context: &crate::extractor::ScriptContext,
) -> Result<Analysis, String> {
    analyze_with_context_mode(source_text, options, context, false, None)
}

/// Same analysis while retaining every distinct grounded entity mention for
/// identity resolution. The semantic engine uses this mode; standalone
/// extraction keeps its compact one-row-per-identity result by default.
pub fn analyze_with_context_preserving_mentions(
    source_text: &str,
    options: &ExtractOptions,
    context: &crate::extractor::ScriptContext,
) -> Result<Analysis, String> {
    analyze_with_context_mode(source_text, options, context, true, None)
}

/// Same analysis with an optional ISO reference date for deictic relative
/// dates (`ieri`, `3 years ago`). The semantic engine passes the request's
/// `reference_date`; absent means those facts stay unresolved.
pub fn analyze_with_context_preserving_mentions_and_reference(
    source_text: &str,
    options: &ExtractOptions,
    context: &crate::extractor::ScriptContext,
    reference_date: Option<&str>,
) -> Result<Analysis, String> {
    analyze_with_context_mode(source_text, options, context, true, reference_date)
}

fn analyze_with_context_mode(
    source_text: &str,
    options: &ExtractOptions,
    context: &crate::extractor::ScriptContext,
    preserve_mentions: bool,
    reference_date: Option<&str>,
) -> Result<Analysis, String> {
    let extract = if preserve_mentions {
        crate::extractor::extract_with_context_and_spellout_preserving_mentions
    } else {
        crate::extractor::extract_with_context_and_spellout
    };
    let (entities, spelled_candidates) = extract(source_text, options, context)?;
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
    // The date parser never spells a word itself: the locale's ICU RBNF
    // rules locate "tredici", "duemilasette" or "two thousand seven", and
    // semantic-core only resolves the calendar around the month token.
    let spelled_numbers: Vec<SpelledNumber> = spelled_candidates
        .iter()
        // A value in the year range is a date only when the scanner typed it
        // as one (no unit and no count). "Mille e duecento persone" is a
        // quantity of 1200, not the year 1200.
        .filter(|candidate| {
            !(1000.0..=2099.0).contains(&candidate.value) || candidate.entity_type == "DATE"
        })
        .map(|candidate| SpelledNumber {
            start: candidate.start,
            end: candidate.end,
            value: candidate.value.round() as i64,
        })
        .collect();
    let temporal_facts = crate::temporal::temporal_facts_with_words_and_reference(
        source_text,
        &options.language,
        &spelled_numbers,
        reference_date,
    );
    // ONE numeric collection, two typed views: `numeric_facts` keeps every
    // grounded value (top-N independent) and `chart_points` is the subset with
    // an unambiguous parse. No second scanner can disagree with the first.
    // Spelled values ("quarantadue per cento") join the same collection, but
    // only when the span is not already a date: a spelled year is a date, not
    // a bar.
    let numeric_facts = numeric_facts(
        source_text,
        &options.language,
        &spelled_candidates,
        &temporal_facts,
    );
    let chart_points = chart_points(
        source_text,
        &options.language,
        &numeric_facts,
        &sentences,
        &entities,
    );

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

fn numeric_facts(
    source_text: &str,
    language: &str,
    spelled: &[crate::spellout::WordNumberCandidate],
    temporal_facts: &[TemporalFact],
) -> Vec<NumericFact> {
    let mut out: Vec<NumericFact> = numeric_value_spans(source_text)
        .into_iter()
        .filter_map(|span| {
            let surface = source_text.get(span.start..span.end)?;
            Some(NumericFact {
                surface: surface.to_string(),
                kind: span.entity_type,
                value: parse_unambiguous_value(surface, language)
                    .or_else(|| parse_scaled_surface(surface, language)),
                start_byte: span.start,
                end_byte: span.end,
                evidence: surface.to_string(),
                form: None,
            })
        })
        .collect();
    for candidate in spelled {
        // The ICU RBNF scanner already resolved and typed spelled numbers:
        // "sedici anni" and "cinque consulenze" arrive here as NUMBER with
        // their value, spelled percentages/money as PERCENT/MONEY. Dates and
        // ordinals are a different projection and must not be duplicated.
        if !matches!(
            candidate.entity_type,
            "NUMBER" | "PERCENT" | "MONEY" | "FRACTION"
        ) {
            continue;
        }
        // A bare spelled count is not a measurement: "quarantadue dei
        // tecnici" names no unit. Only a candidate that carries a unit
        // ("sedici anni") or is followed by an open-class noun ("cinque
        // consulenze", "five reports") enters the numeric projection.
        if candidate.entity_type == "NUMBER" && !candidate.has_unit {
            continue;
        }
        // A spelled date component ("ventiquattro" in "ventiquattro maggio")
        // is owned by the temporal projection. Only a candidate fully inside
        // the date is a component; a larger quantity that contains a date-like
        // token ("Mille e duecento persone") is not owned by the date.
        if temporal_facts
            .iter()
            .any(|fact| candidate.start >= fact.start_byte && candidate.end <= fact.end_byte)
        {
            continue;
        }
        let Some(surface) = source_text.get(candidate.start..candidate.end) else {
            continue;
        };
        out.push(NumericFact {
            surface: surface.to_string(),
            kind: candidate.entity_type.to_string(),
            value: Some(candidate.value),
            start_byte: candidate.start,
            end_byte: candidate.end,
            evidence: surface.to_string(),
            form: Some("words".to_string()),
        });
    }
    let mut out = merge_mixed_scales(source_text, language, out);
    out.sort_by_key(|fact| (fact.start_byte, fact.end_byte));
    out
}

/// "3 milioni e mezzo" is one quantity. The digit scanner emits `3` and the
/// scale noun is prose; this pass attaches the scale (and an additive half)
/// and keeps one fact whose span covers the whole phrase.
fn merge_mixed_scales(source: &str, language: &str, facts: Vec<NumericFact>) -> Vec<NumericFact> {
    let mut out = Vec::with_capacity(facts.len());
    for mut fact in facts {
        if fact.form.as_deref() == Some("words") {
            out.push(fact);
            continue;
        }
        if let Some(value) = fact.value {
            if let Some((end, multiplier)) = mixed_scale_tail(source, language, fact.end_byte) {
                fact.end_byte = end;
                fact.surface = source[fact.start_byte..end].to_string();
                fact.evidence = fact.surface.clone();
                fact.value = Some(value * multiplier);
                fact.form = Some("mixed".to_string());
            } else if let Some((end, denominator)) =
                mixed_fraction_tail(source, language, fact.end_byte)
            {
                let unit = surface_scale_factor(&fact.surface).unwrap_or(1.0);
                fact.end_byte = end;
                fact.surface = source[fact.start_byte..end].to_string();
                fact.evidence = fact.surface.clone();
                fact.value = Some(value + unit / f64::from(denominator));
                fact.form = Some("mixed".to_string());
            }
        }
        out.push(fact);
    }
    let spans: Vec<(usize, usize)> = out
        .iter()
        .map(|fact| (fact.start_byte, fact.end_byte))
        .collect();
    out.retain(|fact| {
        !spans
            .iter()
            .any(|(start, end)| *start < fact.start_byte && *end >= fact.end_byte)
    });
    out
}

/// Parses ` milioni [e mezzo]` / ` million` after `start`. Returns the new end
/// and the multiplier; only a recognized scale triggers a merge.
/// Numeric prefix plus a scale noun already inside the digit span
/// ("3 milioni", "3,2 miliardi").
fn parse_scaled_surface(surface: &str, language: &str) -> Option<f64> {
    let tokens = semantic_core::discourse::word_spans(surface);
    for token in tokens {
        let word = &surface[token.start..token.end];
        let Some(factor) = semantic_core::scale_word_factor(word) else {
            continue;
        };
        let number = surface[..token.start].trim();
        return semantic_core::parse_unambiguous_value(number, language)
            .map(|value| value * factor);
    }
    None
}

/// Scale word inside an already-built surface ("3 milioni").
fn surface_scale_factor(surface: &str) -> Option<f64> {
    semantic_core::discourse::word_spans(surface)
        .into_iter()
        .filter_map(|token| semantic_core::scale_word_factor(&surface[token.start..token.end]))
        .next_back()
}

/// ` e mezzo` / ` and a half` after a already-scaled fact. Returns the new end
/// and the additive denominator.
fn mixed_fraction_tail(source: &str, language: &str, start: usize) -> Option<(usize, u32)> {
    let rules = semantic_core::number_words::rules(language)?;
    let tail = source.get(start..)?;
    let tokens = semantic_core::discourse::word_spans(tail);
    let mut index = 0;
    if let Some(first) = tokens.first() {
        if rules
            .conjunctions
            .iter()
            .any(|word| word.eq_ignore_ascii_case(&tail[first.start..first.end]))
        {
            index += 1;
        }
    }
    let fraction = tokens.get(index)?;
    let word = tail[fraction.start..fraction.end].to_lowercase();
    let spec = rules.fractions.get(&word)?;
    spec.additive
        .then_some((start + fraction.end, spec.denominator))
}

fn mixed_scale_tail(source: &str, language: &str, start: usize) -> Option<(usize, f64)> {
    let tail = source.get(start..)?;
    let tokens = semantic_core::discourse::word_spans(tail);
    let scale = tokens.first()?;
    if !tail[..scale.start].chars().all(char::is_whitespace) {
        return None;
    }
    let factor = semantic_core::scale_word_factor(&tail[scale.start..scale.end])?;
    let mut end = scale.end;
    let mut multiplier = factor;
    let rules = semantic_core::number_words::rules(language);
    let mut index = 1;
    if let (Some(rules), Some(conjunction)) = (rules, tokens.get(index)) {
        if rules
            .conjunctions
            .iter()
            .any(|word| word.eq_ignore_ascii_case(&tail[conjunction.start..conjunction.end]))
        {
            index += 1;
        }
    }
    if let (Some(rules), Some(fraction)) = (rules, tokens.get(index)) {
        let word = tail[fraction.start..fraction.end].to_lowercase();
        if let Some(spec) = rules.fractions.get(&word) {
            if spec.additive {
                multiplier += factor / f64::from(spec.denominator);
                end = fraction.end;
            }
        }
    }
    Some((start + end, multiplier))
}

/// Safety bounds for co-mention generation. A normal editorial sentence names
/// a handful of subjects; an unpunctuated ASR passage can make thousands of
/// entity mentions one "sentence", and the pairwise projection would then
/// allocate O(n²) relations, each carrying the whole sentence as evidence. The
/// bounds keep that case finite without changing any ordinary scene.
const MAX_CO_MENTION_SUBJECTS_PER_SENTENCE: usize = 32;

fn relations(
    source_text: &str,
    entities: &[VisualEntity],
    sentences: &[SentenceSpan],
    entity_sentence: &[Option<usize>],
) -> Vec<EntityRelation> {
    // Group by sentence first: pairing inside one sentence is the definition of
    // a co-mention, and it lets the subject bound apply per sentence.
    let mut per_sentence: Vec<Vec<usize>> = vec![Vec::new(); sentences.len()];
    for (index, sentence) in entity_sentence.iter().enumerate() {
        if let Some(sentence) = sentence {
            if *sentence < per_sentence.len() {
                per_sentence[*sentence].push(index);
            }
        }
    }
    let mut out = Vec::new();
    for (sentence_index, indices) in per_sentence.iter().enumerate() {
        let sentence = sentences[sentence_index];
        // Co-mentions describe subjects the planner can frame together, not
        // every generic phrase: a sentence full of "tempo"/"modo" nouns must
        // not produce a relation per pair.
        let subjects: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&index| is_co_mention_subject(&entities[index]))
            .take(MAX_CO_MENTION_SUBJECTS_PER_SENTENCE)
            .collect();
        if subjects.len() < 2 {
            continue;
        }
        let evidence = source_text[sentence.start_byte..sentence.end_byte].to_string();
        for (position, source) in subjects.iter().enumerate() {
            for target in subjects.iter().skip(position + 1) {
                if same_surface(&entities[*source], &entities[*target]) {
                    continue;
                }
                out.push(EntityRelation {
                    source: *source,
                    target: *target,
                    sentence_index,
                    evidence: evidence.clone(),
                });
            }
        }
    }
    out
}

/// The types that can share an editorial frame. Measurements are values, not
/// subjects, and a generic visual concept is a phrase, not a name.
fn is_co_mention_subject(entity: &VisualEntity) -> bool {
    matches!(
        entity.r#type.as_str(),
        "PERSON"
            | "ORGANIZATION"
            | "ORG"
            | "LOCATION"
            | "FACILITY"
            | "BRAND"
            | "PRODUCT"
            | "WORK"
            | "OBJECT"
            | "EVENT"
    )
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
        .filter_map(|entity| {
            let kind = semantic_core::bullets::signal_kind_for_entity_type(&entity.r#type)?;
            Some(BulletSignal {
                start_byte: entity.start,
                end_byte: entity.end,
                kind,
            })
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

/// Projects the unambiguous subset of `numeric_facts` into chart points.
///
/// The numeric facts are the only scanner; a chart point adds the sentence it
/// belongs to and the matching NER entity index (when one survived the top-N
/// visual budget). Because the fact collection is top-N independent, a scene
/// with more numbers than entities no longer loses chart values.
fn chart_points(
    source_text: &str,
    language: &str,
    numeric_facts: &[NumericFact],
    sentences: &[SentenceSpan],
    entities: &[VisualEntity],
) -> Vec<ChartPoint> {
    numeric_facts
        .iter()
        // A spelled NUMBER ("sedici anni", "cinque consulenze") is a grounded
        // fact but not a chart point: without a written unit or digit the
        // sentence measured a duration/tally, not a plotted quantity. Spelled
        // PERCENT/MONEY keep their existing chart behavior.
        .filter(|fact| {
            fact.kind != "NUMBER"
                || fact
                    .surface
                    .chars()
                    .any(|character| character.is_ascii_digit())
        })
        .filter_map(|fact| {
            let value = fact
                .value
                .or_else(|| semantic_core::numbers::parse_scaled_value(&fact.surface, language))?;
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
#[path = "analysis_tests.rs"]
mod tests;
