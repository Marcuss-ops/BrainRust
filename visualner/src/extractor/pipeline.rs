//! Extraction entry points: `extract`, `extract_with_context`,
//! `script_person_tokens` and the script-wide name context.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::types::{ExtractOptions, VisualEntity};

use super::location::{is_phrase_breaking_byte, mark_contextual_locations, starts_uppercase};
use super::names::{is_heading_surface, is_known_place, is_name_particle, opens_organization_name};
use super::phrases::{
    delimited_work_candidates, keeps_as_visual_candidate, noun_phrase_candidates,
    object_lexicon_candidates, type_object_head_nouns, type_relation_introduced_people,
    work_title_candidates,
};
use super::pos::is_stop_token;
use super::ranking::{entity_identity_key, named_entity_rank, score_entity};
use super::tokenizer::{tokenize, tokenize_untyped, Token};
use super::validation::validate_evidence;
use super::values::{self, value_candidates, Candidate};

/// One extraction pass returns entities, spelled-number candidates and the
/// analyzed tokens retained for downstream event grounding.
pub(crate) type SpelloutExtraction = (
    Vec<VisualEntity>,
    Vec<crate::spellout::WordNumberCandidate>,
    Vec<crate::types::AnalyzedToken>,
);

pub fn extract(source_text: &str, options: &ExtractOptions) -> Result<Vec<VisualEntity>, String> {
    extract_with_context(source_text, options, &ScriptContext::default())
}

/// Script-wide evidence that is only available after every scene is tokenized:
/// the tokens of the person names the script actually states. A one-token place
/// candidate that matches one of them ("Chiara", "Poggi" next to "Chiara
/// Poggi") is a name fragment, not a place. The set is derived from the text,
/// never from a word list.
#[derive(Debug, Default, Clone)]
pub struct ScriptContext {
    pub person_name_tokens: BTreeSet<String>,
    /// Surname -> full name for the two-word person names the script states
    /// ("Alberto Stasi" -> "stasi" -> "Alberto Stasi"). A later bare "Stasi"
    /// is that person, not a visual concept and not a second identity. Only
    /// unambiguous two-token runs qualify: a longer run ("Hall of Fame"), an
    /// institution ("Corte d'Assise") or a surname shared by two people
    /// contributes nothing, so a common phrase never becomes a person.
    pub person_surnames: BTreeMap<String, String>,
}

/// `extract` with script-wide context. The engine builds the context once per
/// script and passes the same value to every scene.
pub fn extract_with_context(
    source_text: &str,
    options: &ExtractOptions,
    context: &ScriptContext,
) -> Result<Vec<VisualEntity>, String> {
    Ok(extract_with_context_and_spellout(source_text, options, context)?.0)
}

/// Same extraction as [`extract_with_context`], but also returns the spelled
/// number candidates it already computed. The scene analysis needs them for
/// dates and charts; recomputing the ICU RBNF scan would double the cost of
/// every scene for the exact same spans.
pub(crate) fn extract_with_context_and_spellout(
    source_text: &str,
    options: &ExtractOptions,
    context: &ScriptContext,
) -> Result<SpelloutExtraction, String> {
    extract_with_context_and_spellout_mode(source_text, options, context, false)
}

pub(crate) fn extract_with_context_and_spellout_preserving_mentions(
    source_text: &str,
    options: &ExtractOptions,
    context: &ScriptContext,
) -> Result<SpelloutExtraction, String> {
    extract_with_context_and_spellout_mode(source_text, options, context, true)
}

/// Inputs shared by every extraction stage. One borrow-only struct keeps the
/// stage functions at a single argument instead of threading the same five
/// references through each call.
struct ExtractionStage<'a> {
    source_text: &'a str,
    language: &'a str,
    tokens: &'a [Token],
    token_lower: &'a [String],
}

/// The typed candidate pool plus the spans and NER state later stages need.
struct CandidatePool {
    candidates: Vec<Candidate>,
    work_spans: Vec<(usize, usize)>,
    model_active: bool,
    model_candidates: Vec<Candidate>,
}

/// Candidate generation and typing: tokenizer phrases, GLiNER spans, work
/// titles, object/values and the contextual type passes. This is the first
/// half of the former single 338-line function.
fn build_candidate_pool(stage: &ExtractionStage<'_>) -> Result<CandidatePool, String> {
    let ExtractionStage {
        source_text,
        language,
        tokens,
        token_lower,
    } = *stage;
    let mut candidates = noun_phrase_candidates(tokens, token_lower, source_text, language);
    let (model_active, model_candidates) = super::ner::candidates(source_text, language)?;
    for model_candidate in &model_candidates {
        candidates.retain(|candidate| {
            let nested =
                candidate.start >= model_candidate.start && candidate.end <= model_candidate.end;
            let conflicting_type =
                candidate.entity_type.as_deref() != model_candidate.entity_type.as_deref();
            !(nested && conflicting_type)
        });
    }
    let work_candidates: Vec<_> = delimited_work_candidates(source_text)
        .into_iter()
        .chain(work_title_candidates(tokens, source_text, language))
        .filter(|work| {
            !model_candidates.iter().any(|model| {
                model.start == work.start
                    && model.end == work.end
                    && model.entity_type.as_deref() != Some("WORK")
            })
        })
        .collect();
    candidates.extend(model_candidates.iter().cloned());
    let mut work_spans: Vec<(usize, usize)> = work_candidates
        .iter()
        .map(|work| (work.start, work.end))
        .collect();
    candidates.extend(work_candidates.iter().cloned());
    if model_active {
        candidates.retain(|candidate| {
            let Some(kind) = candidate.entity_type.as_deref() else {
                return true;
            };
            if !is_ner_entity_type(kind) {
                return true;
            }
            model_candidates.iter().any(|model| {
                model.start == candidate.start
                    && model.end == candidate.end
                    && model.entity_type.as_deref() == Some(kind)
            }) || (kind == "WORK"
                && work_candidates
                    .iter()
                    .any(|work| work.start == candidate.start && work.end == candidate.end))
        });
    }
    work_spans.extend(candidates.iter().filter_map(|candidate| {
        (candidate.entity_type.as_deref() == Some("WORK"))
            .then_some((candidate.start, candidate.end))
    }));
    for object in object_lexicon_candidates(tokens, token_lower, source_text, language) {
        if let Some(existing) = candidates
            .iter_mut()
            .find(|candidate| candidate.start == object.start && candidate.end == object.end)
        {
            existing.entity_type = Some("OBJECT".to_string());
        } else {
            candidates.push(object);
        }
    }
    type_object_head_nouns(&mut candidates, tokens, language);
    candidates.extend(value_candidates(source_text, language));
    // A curated outlet domain is one whole token the noun-phrase pass cannot
    // see (the tokenizer splits at the dot). It joins after the model filters,
    // because it is explicit lexical knowledge, not an NER proposal.
    candidates.extend(values::media_outlet_domain_candidates(
        source_text,
        language,
    ));
    // Collapse the outlet twin BEFORE the contextual typing pass: the pass
    // may upgrade the surviving span to ORG when the sentence has a verb that
    // takes an organization as its object, and that evidence must win over the
    // default outlet type.
    collapse_media_outlet_duplicates(&mut candidates);
    if !model_active {
        type_relation_introduced_people(&mut candidates, tokens, source_text, language);
        mark_contextual_locations(&mut candidates, tokens, source_text, language);
        mark_contextual_organizations(&mut candidates, tokens, language);
        mark_subject_organizations(&mut candidates, tokens, source_text, language);
        split_organization_person_appositions(&mut candidates, tokens, source_text, language);
        // A proper-name run can claim a facility before the facility classifier
        // sees it ("Стадион Северный" is two `nsubj` tokens). Promote the run
        // when the curated facility head plus a capitalized companion is
        // stronger evidence than the person fallback.
        for candidate in &mut candidates {
            if candidate.entity_type.as_deref() == Some("PERSON")
                && values::classify_value_type(&candidate.text, true, true, true, language)
                    == "FACILITY"
            {
                candidate.entity_type = Some("FACILITY".to_string());
            }
        }
    }
    let organization_spans: Vec<(usize, usize)> = candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.entity_type.as_deref(),
                Some("ORG" | "ORGANIZATION")
            ) || matches!(
                values::classify_value_type(&candidate.text, true, true, true, language).as_str(),
                "ORG" | "ORGANIZATION"
            )
        })
        .map(|candidate| (candidate.start, candidate.end))
        .collect();
    candidates.retain(|candidate| {
        let nested_in_work = work_spans.iter().any(|(start, end)| {
            candidate.start >= *start
                && candidate.end <= *end
                && (candidate.start, candidate.end) != (*start, *end)
        });
        let nested_in_organization = candidate.entity_type.as_deref() == Some("LOCATION")
            && organization_spans.iter().any(|(org_start, org_end)| {
                let linked_word = word_before(tokens, candidate.start)
                    .map(str::to_lowercase)
                    .unwrap_or_default();
                let base_language = language.split(['-', '_']).next().unwrap_or(language);
                let articulated_link = semantic_core::lexicon::lexicon()
                    .gazetteer
                    .name_particles
                    .get(base_language)
                    .is_some_and(|particles| particles.contains(&linked_word));
                *org_start < candidate.start && *org_end >= candidate.end && articulated_link
            });
        !nested_in_work && !nested_in_organization
    });
    // Explicitly typed names outrank contextual guesses on the same surface.
    // This prevents one mention from registering once as a facility and again
    // as a nearby location just because a relation cue appeared in its sentence.
    if !model_active {
        for candidate in &mut candidates {
            if candidate.entity_type.is_some() {
                continue;
            }
            if let Some(kind) = values::curated_typed_entity(&candidate.text) {
                let organization =
                    values::classify_value_type(&candidate.text, true, true, true, language)
                        == "ORG";
                candidate.entity_type = Some(if kind == "LOCATION" && organization {
                    "ORG".to_string()
                } else {
                    kind
                });
            }
        }
    }
    let appositional_location_fragments = appositional_fragment_spans(&candidates);
    candidates.retain(|candidate| {
        !appositional_location_fragments.contains(&(candidate.start, candidate.end))
    });
    Ok(CandidatePool {
        candidates,
        work_spans,
        model_active,
        model_candidates,
    })
}

/// Grounding validation, scoring and the apposition/fragment filters. This is
/// the second half of the former single function.
fn validate_and_score(
    stage: &ExtractionStage<'_>,
    candidates: Vec<Candidate>,
    spelled_candidates: &[crate::spellout::WordNumberCandidate],
    model_active: bool,
    model_candidates: &[Candidate],
    work_spans: &[(usize, usize)],
    context: &ScriptContext,
) -> Vec<VisualEntity> {
    let ExtractionStage {
        source_text,
        language,
        tokens,
        ..
    } = *stage;
    let spelled_ranges: Vec<(usize, usize, String)> = spelled_candidates
        .iter()
        .map(|number| (number.start, number.end, number.entity_type.to_string()))
        .collect();
    let mut candidates = candidates;
    candidates.extend(spelled_candidates.iter().map(|number| {
        let text = source_text[number.start..number.end].to_string();
        Candidate {
            text,
            start: number.start,
            end: number.end,
            entity_type: Some(number.entity_type.to_string()),
        }
    }));
    let scored: Vec<VisualEntity> = candidates
        .into_iter()
        // The spelled-number projection owns its span: a bare token candidate
        // over the same bytes ("duemilasette") would otherwise shadow the
        // typed DATE with a generic VISUAL_CONCEPT.
        .filter(|candidate| {
            !spelled_ranges.iter().any(|(start, end, kind)| {
                candidate.start < *end
                    && *start < candidate.end
                    && candidate.entity_type.as_deref() != Some(kind.as_str())
            })
        })
        // A bare number followed by a scale word ("2" in "2 Millionen") is
        // part of a scaled quantity; the value span carries it, so the bare
        // number must not become an entity that charts as 2.0.
        .filter(|candidate| {
            let bare = candidate
                .text
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | ','));
            !(bare && values::scale_word_follows(source_text, candidate.end))
        })
        .filter(|candidate| !is_heading_surface(&candidate.text))
        .filter(|candidate| keeps_as_visual_candidate(candidate, tokens, language))
        .filter_map(|c| validate_evidence(c, source_text, tokens, language))
        .filter(|entity| {
            if !model_active || !is_ner_entity_type(&entity.r#type) {
                return true;
            }
            model_candidates.iter().any(|model| {
                model.start == entity.start
                    && model.end == entity.end
                    && model.entity_type.as_deref() == Some(entity.r#type.as_str())
            }) || (entity.r#type == "WORK" && work_spans.contains(&(entity.start, entity.end)))
        })
        .filter(|entity| structurally_specific_visual_concept(entity, tokens))
        .map(score_entity)
        .collect();
    // The untyped twin of an outlet is only classified during evidence
    // validation, so the candidate-level collapse cannot see it yet. Collapse
    // again on the validated entities: one span, one type.
    let scored = collapse_media_outlet_entities(scored);
    let appositional_location_fusions = appositional_entity_spans(&scored);
    let scored: Vec<VisualEntity> = scored
        .into_iter()
        .filter(|entity| !appositional_location_fusions.contains(&(entity.start, entity.end)))
        .collect();
    drop_name_fragments(scored, context, source_text)
}

/// Deterministic ranking, identity dedupe and the top-N selection (plus every
/// mention when the caller asked to preserve them).
fn rank_and_select(
    scored: Vec<VisualEntity>,
    options: &ExtractOptions,
    preserve_mentions: bool,
    context: &ScriptContext,
) -> Vec<VisualEntity> {
    let mut scored = scored;
    // Rank: highest score first; ties broken by earliest start offset so
    // the order is deterministic across 100/100 runs.
    scored.sort_by(|a, b| {
        named_entity_rank(a)
            .cmp(&named_entity_rank(b))
            .then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.start.cmp(&b.start))
            .then(a.text.cmp(&b.text))
            // Same span and surface can still differ by type (a curated name
            // and a contextual guess); without this the stable sort would
            // preserve an order that depends on the producer.
            .then_with(|| a.r#type.cmp(&b.r#type))
    });
    // A possessive mention is the same identity as its non-possessive form.
    // Deduplicate before applying top-N so "Michael Jordan" and
    // "Michael Jordan's" cannot consume two entity slots.
    //
    // The identity key is hashed into an index map: the previous scan
    // recomputed `entity_identity_key` (a `to_lowercase` + `format!`) for every
    // already-accepted candidate on every incoming entity, which is O(N²)
    // String allocations per extraction call.
    let mut unique: Vec<VisualEntity> = Vec::with_capacity(scored.len());
    let mut index_by_key: HashMap<String, usize> = HashMap::with_capacity(scored.len());
    let mut mention_spans: HashMap<String, Vec<VisualEntity>> = HashMap::new();
    let mut seen_spans: HashMap<String, BTreeSet<(usize, usize)>> = HashMap::new();
    for entity in scored {
        let key = extraction_identity_key(&entity, context);
        if seen_spans
            .entry(key.clone())
            .or_default()
            .insert((entity.start, entity.end))
        {
            mention_spans
                .entry(key.clone())
                .or_default()
                .push(entity.clone());
        }
        if let Some(&existing_idx) = index_by_key.get(&key) {
            // The representative controls ranking and query text. Keep all
            // distinct evidence spans separately for the semantic manifest.
            let existing = &mut unique[existing_idx];
            // A possessive mention is the same identity as its stem, but it is
            // not the person's surface in the manifest: prefer "Donald Trump"
            // when the first representative was "Donald Trump's".
            let prefer_non_possessive = existing
                .text
                .strip_suffix("'s")
                .or_else(|| existing.text.strip_suffix("’s"))
                .is_some_and(|stem| stem == entity.text);
            if prefer_non_possessive
                || (existing.r#type == "PERSON"
                    && !is_canonical_person_surface(existing, context)
                    && is_canonical_person_surface(&entity, context))
            {
                *existing = entity;
            }
            continue;
        }
        index_by_key.insert(key, unique.len());
        unique.push(entity);
    }
    let selected = reserve_numeric_slots(unique, options.top_n());
    if !preserve_mentions {
        return selected;
    }
    let selected_keys: BTreeSet<String> = selected
        .iter()
        .map(|entity| extraction_identity_key(entity, context))
        .collect();
    let mut mentions: Vec<VisualEntity> = mention_spans
        .into_iter()
        .filter(|(key, _)| selected_keys.contains(key))
        .flat_map(|(_, mentions)| mentions)
        .collect();
    // `mention_spans` is a HashMap, so this comparator is the only thing that
    // makes the preserved-mention order deterministic; a complete ordering
    // must include the type, or two same-span candidates flip per process.
    mentions.sort_by(|a, b| {
        named_entity_rank(a)
            .cmp(&named_entity_rank(b))
            .then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.start.cmp(&b.start))
            .then(a.text.cmp(&b.text))
            .then_with(|| a.r#type.cmp(&b.r#type))
    });
    mentions
}

fn extract_with_context_and_spellout_mode(
    source_text: &str,
    options: &ExtractOptions,
    context: &ScriptContext,
    preserve_mentions: bool,
) -> Result<SpelloutExtraction, String> {
    let language = options.language.trim();
    if language.is_empty() {
        return Err("VisualNER language is required".to_string());
    }
    let tokens = tokenize(source_text, language)?;
    let analyzed_tokens = tokens
        .iter()
        .map(|token| crate::types::AnalyzedToken {
            text: token.text.clone(),
            lemma: token.lemma.clone(),
            upos: token.upos.clone(),
            features: token.features.clone(),
            dependency: token.dependency.clone(),
            id: token.id,
            head: token.head,
            sentence_id: token.sentence_id,
            start: token.start,
            end: token.end,
        })
        .collect();
    // One case-folded copy per token, shared by the curated-name matcher and
    // the object lexicon instead of re-folding every token per probe.
    let token_lower: Vec<String> = tokens
        .iter()
        .map(|token| token.text.to_lowercase())
        .collect();
    let stage = ExtractionStage {
        source_text,
        language,
        tokens: &tokens,
        token_lower: &token_lower,
    };
    let pool = build_candidate_pool(&stage)?;
    let spelled_candidates = crate::spellout::candidates(source_text, language)?;
    let scored = validate_and_score(
        &stage,
        pool.candidates,
        &spelled_candidates,
        pool.model_active,
        &pool.model_candidates,
        &pool.work_spans,
        context,
    );
    let selected = rank_and_select(scored, options, preserve_mentions, context);
    Ok((selected, spelled_candidates, analyzed_tokens))
}

/// Word immediately before `offset`, read from the token list instead of
/// re-splitting the source prefix for every candidate.
fn word_before(tokens: &[Token], offset: usize) -> Option<&str> {
    tokens
        .iter()
        .rev()
        .find(|token| token.start < offset)
        .map(|token| token.text.as_str())
}

#[path = "pipeline/names.rs"]
mod names;
pub use names::script_person_tokens;
use names::*;
