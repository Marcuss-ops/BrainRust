//! Extraction entry points: `extract`, `extract_with_context`,
//! `script_person_tokens` and the script-wide name context.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::types::{ExtractOptions, VisualEntity};

use super::location::{is_phrase_breaking_byte, mark_contextual_locations, starts_uppercase};
use super::names::{is_heading_surface, is_known_place, is_name_particle};
use super::phrases::{
    keeps_as_visual_candidate, noun_phrase_candidates, object_lexicon_candidates,
};
use super::pos::is_stop_token;
use super::ranking::{entity_identity_key, named_entity_rank, score_entity};
use super::tokenizer::{tokenize, tokenize_untyped, Token};
use super::validation::validate_evidence;
use super::values::{self, value_candidates, Candidate};

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
) -> Result<(Vec<VisualEntity>, Vec<crate::spellout::WordNumberCandidate>), String> {
    let language = options.language.trim();
    if language.is_empty() {
        return Err("VisualNER language is required".to_string());
    }
    let tokens = tokenize(source_text, language)?;
    // One case-folded copy per token, shared by the curated-name matcher and
    // the object lexicon instead of re-folding every token per probe.
    let token_lower: Vec<String> = tokens
        .iter()
        .map(|token| token.text.to_lowercase())
        .collect();
    let mut candidates = noun_phrase_candidates(&tokens, &token_lower, source_text, language);
    for object in object_lexicon_candidates(&tokens, &token_lower, source_text, language) {
        if let Some(existing) = candidates
            .iter_mut()
            .find(|candidate| candidate.start == object.start && candidate.end == object.end)
        {
            existing.entity_type = Some("OBJECT".to_string());
        } else {
            candidates.push(object);
        }
    }
    candidates.extend(value_candidates(source_text));
    mark_contextual_locations(&mut candidates, &tokens, source_text, language);
    let spelled_candidates = crate::spellout::candidates(source_text, language)?;
    let spelled_ranges: Vec<(usize, usize, String)> = spelled_candidates
        .iter()
        .map(|number| (number.start, number.end, number.entity_type.to_string()))
        .collect();
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
        .filter(|candidate| keeps_as_visual_candidate(candidate, &tokens, language))
        .filter_map(|c| validate_evidence(c, source_text, &tokens, language))
        .map(score_entity)
        .collect();
    let mut scored = drop_name_fragments(scored, context);
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
    for entity in scored {
        let key = entity_identity_key(&entity);
        if let Some(&existing_idx) = index_by_key.get(&key) {
            // Prefer the shorter non-possessive surface when both mentions
            // occur; otherwise retain the first deterministic evidence span.
            let existing = &mut unique[existing_idx];
            if existing.r#type == "PERSON" && entity.text.len() < existing.text.len() {
                *existing = entity;
            }
            continue;
        }
        index_by_key.insert(key, unique.len());
        unique.push(entity);
    }
    Ok((
        reserve_numeric_slots(unique, options.top_n()),
        spelled_candidates,
    ))
}

/// Collects the tokens of every multi-token capitalized run in the script.
/// Tokenization uses the same ICU rules as extraction, so no per-language
/// list is involved; a caseless script contributes nothing because
/// capitalization is not name evidence there.
pub fn script_person_tokens(source_texts: &[&str], language: &str) -> ScriptContext {
    let mut context = ScriptContext::default();
    let mut surnames: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if !semantic_core::lexicon::lexicon()
        .language_profiles
        .is_cased(language)
    {
        return context;
    }
    for text in source_texts {
        let Ok(tokens) = tokenize_untyped(text, language) else {
            continue;
        };
        let mut run: Vec<&Token> = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            let name_like = token.text.chars().next().is_some_and(char::is_uppercase);
            let particle = !run.is_empty()
                && is_name_particle(&token.text, language)
                && tokens.get(index + 1).is_some_and(|next| {
                    starts_uppercase(&next.text)
                        && !is_stop_token(next, language)
                        && !text.as_bytes()[token.end..next.start]
                            .iter()
                            .any(|byte| is_phrase_breaking_byte(*byte))
                });
            if name_like || particle {
                // A sentence break ends the name: "Garlasco. La mattina" is
                // a place and a new sentence, never the place "Garlasco La".
                if let Some(previous) = run.last() {
                    if text.as_bytes()[previous.end..token.start]
                        .iter()
                        .any(|byte| is_phrase_breaking_byte(*byte))
                    {
                        flush_name_run(&mut run, &mut context, &mut surnames, language);
                        if particle {
                            // A particle cannot open a name run.
                            continue;
                        }
                    }
                }
                run.push(token);
                continue;
            }
            flush_name_run(&mut run, &mut context, &mut surnames, language);
        }
        flush_name_run(&mut run, &mut context, &mut surnames, language);
    }
    // Only a surname that resolves to exactly one full name is an alias; two
    // people sharing it stay two people.
    for (surname, names) in surnames {
        if names.len() == 1 {
            if let Some(name) = names.into_iter().next() {
                context.person_surnames.insert(surname, name);
            }
        }
    }
    context
}
fn flush_name_run(
    run: &mut Vec<&Token>,
    context: &mut ScriptContext,
    surnames: &mut BTreeMap<String, BTreeSet<String>>,
    language: &str,
) {
    // Two *significant* tokens make a name. "Garlasco La" is a place followed
    // by an article, so it must not contribute "garlasco" as a person token.
    let significant = run
        .iter()
        .filter(|token| !is_stop_token(token, language))
        .count();
    let usable = significant >= 2 && !run.iter().any(|token| is_heading_surface(&token.text));
    if usable {
        // A leading known place is a city, not a person token: the run
        // "Пермь Мария Соколова" must not make "пермь" look like a name
        // fragment that can suppress the LOCATION candidate later. The same
        // holds for a curated institution head: "Staatsanwaltschaft Berlin"
        // is a court, not a person name, so seeding its tokens would make
        // "berlin" look like a name fragment and drop the LOCATION candidate
        // the nested-place mechanism just produced.
        let skip_leading_place = run.len() >= 3
            && is_known_place(&run[0].text, language)
            && !is_name_particle(&run[1].text, language);
        let significant_surface = run
            .iter()
            .filter(|token| !is_stop_token(token, language))
            .map(|token| token.text.as_str())
            .collect::<Vec<&str>>()
            .join(" ");
        let seeds_person_tokens =
            values::institution_cue_kind(&significant_surface, language).is_none();
        if seeds_person_tokens {
            for token in run.iter().skip(usize::from(skip_leading_place)) {
                if !is_stop_token(token, language) {
                    context.person_name_tokens.insert(token.text.to_lowercase());
                }
            }
        }
        // A surname is only claimed from an unambiguous two-word name that is
        // neither a place nor a curated institution. "Chiara Poggi" yields
        // "poggi"; "Corte d'Assise" and "Hall of Fame" yield nothing.
        let significant_tokens: Vec<&&Token> = run
            .iter()
            .filter(|token| !is_stop_token(token, language))
            .collect();
        if significant_tokens.len() == 2 {
            let first = significant_tokens[0].text.as_str();
            let last = significant_tokens[1].text.as_str();
            let surface = run
                .iter()
                .map(|token| token.text.as_str())
                .collect::<Vec<&str>>()
                .join(" ");
            if !is_known_place(first, language)
                && !is_known_place(last, language)
                && values::institution_cue_kind(&surface, language).is_none()
            {
                surnames
                    .entry(last.to_lowercase())
                    .or_default()
                    .insert(surface);
            }
        }
    }
    run.clear();
}

/// Drops two kinds of fragment, both derived from the script's own names:
/// a one-token place whose surface is a token of a person name stated
/// elsewhere, and a mention fully contained in a longer PERSON mention of the
/// same scene. The longer, better-typed mention owns the identity.
fn drop_name_fragments(entities: Vec<VisualEntity>, context: &ScriptContext) -> Vec<VisualEntity> {
    let person_spans: Vec<(usize, usize)> = entities
        .iter()
        .filter(|entity| entity.r#type == "PERSON")
        .map(|entity| (entity.start, entity.end))
        .collect();
    entities
        .into_iter()
        .filter(|entity| {
            if !matches!(entity.r#type.as_str(), "LOCATION" | "PERSON") {
                return true;
            }
            if entity.r#type == "LOCATION"
                && !entity.text.contains(char::is_whitespace)
                && context
                    .person_name_tokens
                    .contains(&entity.text.to_lowercase())
            {
                return false;
            }
            !person_spans.iter().any(|(start, end)| {
                (*start, *end) != (entity.start, entity.end)
                    && entity.start >= *start
                    && entity.end <= *end
            })
        })
        .collect()
}

/// entities (NUMBER/PERCENT/MONEY). Without this, a low cap ranks every value
/// below the identities and the chart loses its numbers entirely. The share is
/// a third of the cap (at least one slot); numeric entities beyond the cap
/// replace the lowest-ranked non-numeric entities only when fewer numerics
/// made the cut. Surviving entities keep their original rank order.
fn reserve_numeric_slots(ranked: Vec<VisualEntity>, top_n: usize) -> Vec<VisualEntity> {
    if ranked.len() <= top_n {
        return ranked;
    }
    let is_numeric = |e: &VisualEntity| matches!(e.r#type.as_str(), "NUMBER" | "PERCENT" | "MONEY");
    let reserve = (top_n / 3).max(1).min(top_n);
    let kept_numeric = ranked[..top_n].iter().filter(|e| is_numeric(e)).count();
    let spare: Vec<usize> = (top_n..ranked.len())
        .filter(|&i| is_numeric(&ranked[i]))
        .collect();
    let needed = reserve.saturating_sub(kept_numeric).min(spare.len());
    let mut keep = vec![true; top_n];
    let mut evicted = 0;
    for pos in (0..top_n).rev() {
        if evicted == needed {
            break;
        }
        if !is_numeric(&ranked[pos]) {
            keep[pos] = false;
            evicted += 1;
        }
    }
    let mut selected: Vec<usize> = (0..top_n)
        .filter(|&pos| keep[pos])
        .chain(spare.into_iter().take(needed))
        .collect();
    selected.sort_unstable();
    let mut slots: Vec<Option<VisualEntity>> = ranked.into_iter().map(Some).collect();
    selected
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}
