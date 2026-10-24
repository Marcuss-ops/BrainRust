//! The deterministic VisualNER extraction pipeline (V1):
//!
//! ```text
//! text
//!  ↓
//! tokenize
//!  ↓
//! noun/noun-phrase candidates
//!  ↓
//! stop phrases
//!  ↓
//! visualness scoring
//!  ↓
//! source evidence validation
//!  ↓
//! rank
//!  ↓
//! top N
//! ```
//!
//! V1 is deterministic: NO ONNX, NO LLM, NO GPU. The killer rule is
//! `NO EVIDENCE → NO ENTITY`: every returned entity must be a verbatim
//! substring of the source text, with byte offsets proving the evidence.

mod location;
mod pos;
mod stopwords;
mod verb;
use location::*;
use pos::{is_stop_token, tag_tokens};
use stopwords::*;

use std::collections::{BTreeSet, HashMap};

use crate::types::{ExtractOptions, VisualEntity};

mod ranking;
mod values;

use ranking::{entity_identity_key, named_entity_rank, score_entity};
use values::{classify_value_type, value_candidates, Candidate};

/// Byte span of one numeric-value candidate (NUMBER/PERCENT/MONEY),
/// independent of the visual top-N budget ( NumericFacts).
#[derive(Debug, Clone, PartialEq)]
pub struct NumericSpan {
    pub start: usize,
    pub end: usize,
    pub entity_type: String,
}

/// Collects every numeric-value span from `source_text` in textual order.
/// This projection exists so a scene with more numbers than the visual
/// top-N budget keeps its numeric facts: charts and composed statistics read
/// from this list, not from the ranked entities.
pub fn numeric_value_spans(source_text: &str) -> Vec<NumericSpan> {
    values::value_candidates(source_text)
        .into_iter()
        .filter(|candidate| {
            matches!(
                candidate.entity_type.as_deref(),
                Some("NUMBER") | Some("PERCENT") | Some("MONEY")
            )
        })
        .map(|candidate| NumericSpan {
            start: candidate.start,
            end: candidate.end,
            entity_type: candidate.entity_type.unwrap_or_default(),
        })
        .collect()
}

/// Extract the top-N source-grounded visual entities from `source_text`.
/// This is the single canonical entry point. Returns entities in score-desc
/// order (ties broken by earliest source position).
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
}

/// `extract` with script-wide context. The engine builds the context once per
/// script and passes the same value to every scene.
pub fn extract_with_context(
    source_text: &str,
    options: &ExtractOptions,
    context: &ScriptContext,
) -> Result<Vec<VisualEntity>, String> {
    let language = options.language.trim();
    if language.is_empty() {
        return Err("VisualNER language is required".to_string());
    }
    let tokens = tokenize(source_text, language)?;
    let mut candidates = noun_phrase_candidates(&tokens, source_text, language);
    for object in object_lexicon_candidates(&tokens, source_text, language) {
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
    candidates.extend(spelled_candidates.into_iter().map(|number| {
        let text = source_text[number.start..number.end].to_string();
        Candidate {
            normalized: text.to_lowercase(),
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
    Ok(reserve_numeric_slots(unique, options.top_n()))
}

/// Collects the tokens of every multi-token capitalized run in the script.
/// Tokenization uses the same ICU rules as extraction, so no per-language
/// list is involved; a caseless script contributes nothing because
/// capitalization is not name evidence there.
pub fn script_person_tokens(source_texts: &[&str], language: &str) -> ScriptContext {
    let mut context = ScriptContext::default();
    if !semantic_core::lexicon::lexicon()
        .language_profiles
        .is_cased(language)
    {
        return context;
    }
    for text in source_texts {
        let Ok(tokens) = tokenize(text, language) else {
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
                            .any(|byte| location::is_phrase_breaking_byte(*byte))
                });
            if name_like || particle {
                // A sentence break ends the name: "Garlasco. La mattina" is
                // a place and a new sentence, never the place "Garlasco La".
                if let Some(previous) = run.last() {
                    if text.as_bytes()[previous.end..token.start]
                        .iter()
                        .any(|byte| location::is_phrase_breaking_byte(*byte))
                    {
                        flush_name_run(&mut run, &mut context, language);
                        if particle {
                            // A particle cannot open a name run.
                            continue;
                        }
                    }
                }
                run.push(token);
                continue;
            }
            flush_name_run(&mut run, &mut context, language);
        }
        flush_name_run(&mut run, &mut context, language);
    }
    context
}

fn flush_name_run(run: &mut Vec<&Token>, context: &mut ScriptContext, language: &str) {
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
        // fragment that can suppress the LOCATION candidate later.
        let skip_leading_place = run.len() >= 3
            && is_known_place(&run[0].text, language)
            && !is_name_particle(&run[1].text, language);
        for token in run.iter().skip(usize::from(skip_leading_place)) {
            if !is_stop_token(token, language) {
                context.person_name_tokens.insert(token.text.to_lowercase());
            }
        }
    }
    run.clear();
}

/// Keeps a generic phrase only when the POS model still sees a noun phrase:
/// a clause that opens with a verb, auxiliary or pronoun ("c'è nulla") or an
/// adverb stack ("ancora oggi") is not a visual subject. Typed candidates
/// (objects, values, names) are not affected, and without a model every
/// candidate is kept, so the model-free path is unchanged.
fn keeps_as_visual_candidate(candidate: &Candidate, tokens: &[Token], language: &str) -> bool {
    if candidate.entity_type.is_some() {
        return true;
    }
    // A predicate or function word opens a clause, never a visual phrase.
    // UDPipe tags common names as PRON ("OpenAI"), so a pronoun only counts
    // when the orthography still marks it as a name.
    // A name can be mistagged as a pronoun, but a pronoun cannot be talked
    // into a name: the language pack's function inventory still rules it out,
    // which is what keeps a sentence-initial "Ciò" or "Noi" out while
    // "OpenAI" (absent from every inventory) stays.
    let pronoun_is_name = |token: &Token| {
        if !semantic_core::lexicon::lexicon()
            .language_profiles
            .is_cased(language)
            || !starts_uppercase(&token.text)
        {
            return false;
        }
        let lower = token.text.to_lowercase();
        // A capitalized surface the pack lists as a function word is still a
        // function word: "C'è" and "Ciò" open a sentence, they do not name
        // anything.
        !semantic_core::language_packs::get(language)
            .is_some_and(|pack| pack.is_function_word(&lower) || pack.is_stopword(&lower))
    };
    let opens_a_clause = |token: &Token| match token.upos.as_deref() {
        Some("VERB" | "AUX" | "SCONJ" | "CCONJ" | "INTJ" | "DET" | "ADP") => true,
        Some("PRON") => !pronoun_is_name(token),
        _ => false,
    };
    let span: Vec<&Token> = tokens
        .iter()
        .filter(|token| token.start >= candidate.start && token.end <= candidate.end)
        .collect();
    if span.is_empty() || span.iter().all(|token| token.upos.is_none()) {
        return true;
    }
    if opens_a_clause(span[0]) {
        return false;
    }
    // A verb inside the phrase means the candidate spans a predicate
    // ("anni sarebbe tornato"); a trailing participle is the only form that
    // still modifies a noun ("condanna anticipata").
    let last = span.len() - 1;
    let predicate_inside = span
        .iter()
        .enumerate()
        .any(|(index, token)| index != last && opens_a_clause(token));
    if predicate_inside {
        return false;
    }
    // An adverb or adjective stack without any noun ("ancora oggi") is not a
    // subject either. Unknown tags (X) and adjectives are kept: UDPipe tags
    // many proper names and brands as X or ADJ, and refusing them here would
    // drop real identities the typers can still recognize later.
    span.iter().any(|token| match token.upos.as_deref() {
        Some("NOUN" | "PROPN" | "NUM" | "ADJ" | "X") => true,
        Some("PRON") => pronoun_is_name(token),
        _ => false,
    })
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

/// Adds typed candidates for a small, high-precision visual-object vocabulary.
/// This lexical backstop catches common objects even when a POS model tags the
/// noun unreliably; it still uses exact tokenizer spans and never rewrites the
/// source surface. The vocabulary is deliberately explicit and expandable
/// from corpus evidence rather than guessing that every noun is an object.
fn object_lexicon_candidates(
    tokens: &[Token],
    source_text: &str,
    language: &str,
) -> Vec<Candidate> {
    let terms = semantic_core::lexicon::lexicon().object_terms(language);
    if terms.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for start_index in 0..tokens.len() {
        let mut normalized = String::new();
        let max_end = (start_index + 3).min(tokens.len());
        for end_index in start_index..max_end {
            if end_index > start_index {
                let gap = &source_text[tokens[end_index - 1].end..tokens[end_index].start];
                if gap.is_empty() || !gap.chars().all(char::is_whitespace) {
                    break;
                }
                normalized.push(' ');
            }
            normalized.push_str(&tokens[end_index].text.to_lowercase());
            if !terms.iter().any(|term| term == &normalized) {
                continue;
            }
            let start = tokens[start_index].start;
            let end = tokens[end_index].end;
            let Some(text) = source_text.get(start..end) else {
                continue;
            };
            candidates.push(Candidate {
                text: text.to_string(),
                normalized: normalized.clone(),
                start,
                end,
                entity_type: Some("OBJECT".to_string()),
            });
        }
    }
    candidates
}

/// Applies the top-N cap while keeping a minimum share of numeric value
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

/// Token is a maximal run of letters (ASCII + accented Latin) and
/// apostrophes/hyphens inside a word, with its byte offsets into the
/// source text. Non-letter chars are separators.
#[derive(Debug, Clone)]
struct Token {
    text: String,
    start: usize,
    end: usize,
    /// Universal POS tag from UDPipe, when a model for the language is installed.
    upos: Option<String>,
    /// Dictionary form from the same UDPipe model, when it is installed.
    lemma: Option<String>,
}

/// tokenize segments `source_text` into words with ICU's word-break rules for
/// `language`. ICU handles scripts without spaces (Chinese, Japanese, Thai)
/// and language-specific apostrophes, so no per-language lists are needed.
/// Hyphenated and apostrophe-joined compounds (Jean-Luc) become one token.
/// ICU offsets are UTF-16 units; they are converted to UTF-8 byte offsets.
fn tokenize(source_text: &str, language: &str) -> Result<Vec<Token>, String> {
    let breaks = rust_icu_ubrk::UBreakIterator::try_new(
        rust_icu_sys::UBreakIteratorType::UBRK_WORD,
        language,
        source_text,
    )
    .map_err(|error| format!("ICU word segmentation failed for {language:?}: {error}"))?;

    let mut utf16_to_byte: Vec<usize> = Vec::with_capacity(source_text.len() + 1);
    for (offset, ch) in source_text.char_indices() {
        for _ in 0..ch.len_utf16() {
            utf16_to_byte.push(offset);
        }
    }
    utf16_to_byte.push(source_text.len());

    // (byte_start, byte_end, is_word) for every ICU segment, in order.
    let mut segments: Vec<(usize, usize, bool)> = Vec::new();
    let mut start16 = breaks.first();
    loop {
        let end16 = breaks.following(start16);
        if end16 < 0 {
            break;
        }
        // UBRK_WORD_NONE_LIMIT is 100: rule status >= 100 marks word-like text.
        let is_word = breaks.get_rule_status() >= 100;
        segments.push((
            utf16_to_byte[start16 as usize],
            utf16_to_byte[end16 as usize],
            is_word,
        ));
        start16 = end16;
    }

    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < segments.len() {
        let (start, end, is_word) = segments[index];
        index += 1;
        if !is_word {
            continue;
        }
        let mut token_end = end;
        // Join word, joiner, word where the joiner is a hyphen or apostrophe.
        while index + 1 < segments.len() {
            let (_, joiner_end, joiner_word) = segments[index];
            let (next_start, next_end, next_word) = segments[index + 1];
            let joiner = &source_text[token_end..joiner_end];
            let is_joiner = joiner == "-" || joiner == "'" || joiner == "\u{2019}";
            if is_joiner && !joiner_word && next_word && next_start == joiner_end {
                token_end = next_end;
                index += 2;
            } else {
                break;
            }
        }
        tokens.push(Token {
            text: source_text[start..token_end].to_string(),
            start,
            end: token_end,
            upos: None,
            lemma: None,
        });
    }
    let mut tokens = split_elided_clitics(source_text, tokens);
    tag_tokens(source_text, language, &mut tokens);
    Ok(tokens)
}

/// split_elided_clitics separates an elided preposition/article from the
/// proper name it is attached to: ICU keeps "dell'Abruzzo", "dell'Umbria"
/// and "sull'Aterno" as single word tokens, so the name can never be a
/// candidate and the geographic cue never sees its preposition. Splitting
/// yields the cue token ("dell'") and the place ("Abruzzo"), both still
/// verbatim spans of the source. A one-letter prefix stays attached
/// ("L'Aquila", "d'Artagnan", "O'Brien"), because there the prefix is part
/// of the name; a lower-case remainder keeps the compound ("dell'arte",
/// "l'arte").
fn split_elided_clitics(source_text: &str, tokens: Vec<Token>) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len() + 4);
    for token in tokens {
        match elided_clitic_split(source_text, &token) {
            Some((clitic_end, name_start)) => {
                let mut clitic = token.clone();
                clitic.text = source_text[token.start..clitic_end].to_string();
                clitic.end = clitic_end;
                let mut name = token;
                name.text = source_text[name_start..name.end].to_string();
                name.start = name_start;
                out.push(clitic);
                out.push(name);
            }
            None => out.push(token),
        }
    }
    out
}

/// elided_clitic_split returns the byte end of the elided prefix (the
/// apostrophe included) and the byte start of the proper name, when the
/// token is `<two-or-more ASCII letters>'<Uppercase...>`.
fn elided_clitic_split(source_text: &str, token: &Token) -> Option<(usize, usize)> {
    let text = source_text.get(token.start..token.end)?;
    if text != token.text || !text.is_ascii() {
        return None;
    }
    let apostrophe = text.find(['\'', '\u{2019}'])?;
    let prefix = &text[..apostrophe];
    if prefix.len() < 2 || !prefix.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let remainder_start = token.start + apostrophe + text[apostrophe..].chars().next()?.len_utf8();
    if !starts_uppercase(source_text.get(remainder_start..token.end)?) {
        return None;
    }
    Some((token.start + apostrophe + 1, remainder_start))
}

/// noun_phrase_candidates groups adjacent non-stopword tokens into maximal
/// noun phrases. A phrase is one or more consecutive tokens whose lowercase
/// surface is not a stopword, AND that are separated only by whitespace
/// (a comma, period, semicolon or other punctuation between two tokens is a
/// hard phrase boundary). Multi-word phrases ("feta cheese",
/// "olive oil") score higher than singletons, which is what makes concrete
/// multi-word objects win over generic singletons.
fn noun_phrase_candidates(tokens: &[Token], source_text: &str, language: &str) -> Vec<Candidate> {
    let bytes = source_text.as_bytes();
    let mut out = Vec::new();
    // Named people/organizations are bounded to the contiguous title-case run.
    // This prevents lower-case lead-ins such as "figures like Phil Jackson"
    // from becoming a PERSON entity; the grounded candidate is "Phil Jackson".
    for (start, end) in proper_name_runs(tokens, bytes, language) {
        out.push(candidate_from_tokens(tokens, source_text, start, end));
    }
    // Curated names are entities even when a function word sits inside them
    // ("Will Always Love You" ends in "you"), so they are emitted verbatim
    // before the generic phrase scan can truncate them. The gazetteer decides
    // the type here: a title that opens with a modal ("Will Always Love You")
    // must not be read as a clause by the POS gate.
    for index in 0..tokens.len() {
        if let Some(end) = curated_name_span(tokens, bytes, index) {
            let mut candidate = candidate_from_tokens(tokens, source_text, index, end);
            candidate.entity_type = Some(classify_value_type(
                &candidate.text,
                true,
                false,
                true,
                language,
            ));
            out.push(candidate);
        }
    }
    let mut i = 0;
    while i < tokens.len() {
        // A single uppercase initial between periods is a tokenization
        // artifact of dotted initialisms (e.g. U.S.), not an entity span.
        if is_stop_token(&tokens[i], language)
            || (tokens[i].text.len() == 1 && tokens[i].text.as_bytes()[0].is_ascii_uppercase())
        {
            i += 1;
            continue;
        }
        // Proper names are bounded by the title-case run. The old maximal
        // noun-phrase rule incorrectly absorbed predicates and their objects
        // (for example "Floyd Mayweather became one"), producing invalid
        // semantic spans. Lowercase phrases retain the noun-phrase behavior
        // used by the media-object fixtures.
        let phrase_start = i;
        let mut j = i;
        if starts_uppercase(&tokens[i].text) {
            if has_dotted_initial_prefix(tokens, bytes, i) {
                i += 1;
                continue;
            }
            while j < tokens.len() && starts_uppercase(&tokens[j].text) {
                if j > phrase_start {
                    let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                    if gap.iter().any(|b| is_phrase_breaking_byte(*b)) {
                        break;
                    }
                }
                j += 1;
            }
        } else {
            while j < tokens.len() && !is_stop_token(&tokens[j], language) {
                if j > phrase_start {
                    let gap = &bytes[tokens[j - 1].end..tokens[j].start];
                    if gap.iter().any(|b| is_phrase_breaking_byte(*b))
                        || starts_uppercase(&tokens[j].text)
                    {
                        // A lower-case clause/predicate must not absorb the
                        // next title-cased name into a VISUAL_CONCEPT span
                        // (e.g. "discussed Tesla"). Let the proper-name
                        // candidate own the capitalized token instead.
                        break;
                    }
                    // A finite verbal predicate continues the clause, not
                    // the noun phrase ("I tifosi festeggiano sugli spalti"):
                    // the verb breaks the phrase here and is skipped below,
                    // so it can never glue to a subject or become an entity.
                    if verb::is_verbal_continuation(tokens, j, language) {
                        break;
                    }
                }
                j += 1;
            }
        }
        let phrase_end = j; // exclusive
        let start = tokens[phrase_start].start;
        let end = tokens[phrase_end - 1].end;
        let text = source_text[start..end].to_string();
        let normalized = text.to_lowercase();
        // The run decomposition depends only on the phrase, so it is computed
        // ONCE here instead of twice (the previous form called
        // proper_name_runs twice, each allocating a fresh Vec).
        let phrase_runs = proper_name_runs(&tokens[phrase_start..phrase_end], bytes, language);
        let has_inner_run = phrase_runs.iter().any(|(run_start, _)| *run_start > 0);
        let is_full_run = phrase_runs
            .iter()
            .any(|(run_start, run_end)| *run_start == 0 && *run_end == phrase_end - phrase_start);
        if has_inner_run && !is_full_run {
            i = phrase_end;
            continue;
        }
        // Reject phrases whose normalized surface is a stop phrase, a
        // generic phrase, or a contextual verbal predicate ("e sorge nella
        // pianura"): GEN-01 keeps verbs out of the entity set without a
        // lexical blacklist, so nominal readings of the same surface
        // ("La guarda del castello") stay extractable.
        if !is_stop_phrase(&normalized)
            && !is_generic_phrase(&normalized)
            && !is_heading_surface(&text)
            && !verb::is_verbal_predicate(tokens, phrase_start, phrase_end, language)
        {
            out.push(Candidate {
                text,
                normalized,
                start,
                end,
                entity_type: None,
            });
        }
        i = phrase_end;
        // The finite verb that broke the phrase is not candidate material.
        if i < tokens.len() && verb::is_verbal_continuation(tokens, i, language) {
            i += 1;
        }
    }
    out
}

/// End token index of the longest curated gazetteer name (work, person,
/// brand, product, place) that starts exactly at `tokens[index]`. Such a span
/// is an entity even when one of its words is a function word
/// ("Will Always Love You" ends in the pronoun "you").
fn curated_name_span(tokens: &[Token], bytes: &[u8], index: usize) -> Option<usize> {
    let gazetteer = &semantic_core::lexicon::lexicon().gazetteer;
    let names = gazetteer
        .works
        .iter()
        .chain(gazetteer.persons.iter())
        .chain(gazetteer.brands.iter())
        .chain(gazetteer.product_names.iter())
        .chain(gazetteer.locations.iter());
    let mut longest: Option<usize> = None;
    for name in names {
        let words: Vec<&str> = name.split_whitespace().collect();
        if words.is_empty() || index + words.len() > tokens.len() {
            continue;
        }
        let matches = words.iter().enumerate().all(|(offset, word)| {
            tokens[index + offset].text.to_lowercase() == *word
                && (offset == 0
                    || !bytes[tokens[index + offset - 1].end..tokens[index + offset].start]
                        .iter()
                        .any(|byte| is_phrase_breaking_byte(*byte)))
        });
        if matches {
            longest = Some(longest.map_or(index + words.len(), |end| end.max(index + words.len())));
        }
    }
    longest
}

fn proper_name_runs(tokens: &[Token], bytes: &[u8], language: &str) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        // Dotted initials belong to the following title-cased name (J. D.
        // Vance), but dotted initialisms such as U.S. have no trailing name
        // and must not create fragments. Keep the boundary evidence verbatim.
        if is_uppercase_initial(&tokens[i].text) {
            let mut final_index = i;
            while final_index + 1 < tokens.len()
                && gap_has_period(&bytes[tokens[final_index].end..tokens[final_index + 1].start])
            {
                final_index += 1;
                if !is_uppercase_initial(&tokens[final_index].text) {
                    break;
                }
            }
            if final_index > i
                && !is_uppercase_initial(&tokens[final_index].text)
                && starts_uppercase(&tokens[final_index].text)
                && !is_stop_token(&tokens[final_index], language)
            {
                runs.push((i, final_index + 1));
                i = final_index + 1;
                continue;
            }
            i += 1;
            continue;
        }
        if is_stop_token(&tokens[i], language) || !starts_uppercase(&tokens[i].text) {
            i += 1;
            continue;
        }
        let mut start = i;
        let mut end = i + 1;
        while end < tokens.len() && name_run_continues(tokens, bytes, end, language) {
            end += 1;
        }
        // An all-caps run is a heading ("NOME CHE TORNA"), not a person.
        // Keeping it out of the proper-name set stops the heading from
        // becoming a PERSON and from seeding image queries.
        let heading = tokens[start..end]
            .iter()
            .filter(|token| token.text.chars().filter(|ch| ch.is_alphabetic()).count() >= 2)
            .all(|token| is_all_caps_word(&token.text));
        // "Пермь Мария Соколова" is a city followed by a person, not a
        // three-token person name. A known place stays out of the run so its
        // own gazetteer candidate survives instead of being dropped as a
        // fragment of the person.
        if end - start >= 3
            && is_known_place(&tokens[start].text, language)
            && !is_name_particle(&tokens[start + 1].text, language)
        {
            start += 1;
        }
        if end - start >= 2 && !heading {
            runs.push((start, end));
        }
        i = end;
    }
    runs
}

/// Whether the token at `index` extends the name run that starts before it.
/// A lowercase surname particle is allowed only between name tokens, so
/// "Emma de Vries" is one name while a trailing "de" stays out.
fn name_run_continues(tokens: &[Token], bytes: &[u8], index: usize, language: &str) -> bool {
    let gap_breaks = |left: usize, right: usize| {
        bytes[tokens[left].end..tokens[right].start]
            .iter()
            .any(|b| is_phrase_breaking_byte(*b))
    };
    let token = &tokens[index];
    if starts_uppercase(&token.text)
        && !is_stop_token(token, language)
        && !(token.text.len() == 1 && token.text.as_bytes()[0].is_ascii_uppercase())
        && !gap_breaks(index - 1, index)
    {
        return true;
    }
    is_name_particle(&token.text, language)
        && index + 1 < tokens.len()
        && starts_uppercase(&tokens[index + 1].text)
        && !is_stop_token(&tokens[index + 1], language)
        && !gap_breaks(index - 1, index)
        && !gap_breaks(index, index + 1)
}

/// True when the gazetteer or place index recognizes `text` as a place.
fn is_known_place(text: &str, language: &str) -> bool {
    semantic_core::lexicon::lexicon().is_place(text, language)
}

/// True when `text` is a surname particle declared for the language.
fn is_name_particle(text: &str, language: &str) -> bool {
    let key = language.split(['-', '_']).next().unwrap_or("");
    semantic_core::lexicon::lexicon()
        .gazetteer
        .name_particles
        .get(key)
        .is_some_and(|particles| {
            particles
                .iter()
                .any(|particle| particle.eq_ignore_ascii_case(text))
        })
}

/// True for a token spelled entirely in capitals, ignoring punctuation and
/// digits. Single letters are excluded so initials stay name fragments.
fn is_all_caps_word(text: &str) -> bool {
    let letters: Vec<char> = text.chars().filter(|ch| ch.is_alphabetic()).collect();
    letters.len() >= 2 && letters.iter().all(|ch| ch.is_uppercase())
}

/// True for editorial headings: a multi-word all-caps run, or a long single
/// all-caps word. Short all-caps tokens (DNA, GUP, ANSA, USA) stay ordinary
/// candidates; headings such as "NOME CHE TORNA" or "RITROVAMENTO" do not.
fn is_heading_surface(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    // Single-letter function words inside a heading ("LE INDAGINI E I PRIMI
    // PASSAGGI") do not make it a name, so the all-caps test ignores them.
    let substantial: Vec<&str> = words
        .iter()
        .copied()
        .filter(|word| word.chars().filter(|ch| ch.is_alphabetic()).count() >= 2)
        .collect();
    if substantial.is_empty() || !substantial.iter().all(|word| is_all_caps_word(word)) {
        return false;
    }
    words.len() >= 2
        || substantial[0]
            .chars()
            .filter(|ch| ch.is_alphabetic())
            .count()
            >= 6
}

fn candidate_from_tokens(
    tokens: &[Token],
    source_text: &str,
    start: usize,
    end: usize,
) -> Candidate {
    let byte_start = tokens[start].start;
    let byte_end = tokens[end - 1].end;
    let text = source_text[byte_start..byte_end].to_string();
    Candidate {
        normalized: text.to_lowercase(),
        text,
        start: byte_start,
        end: byte_end,
        entity_type: None,
    }
}

/// validate_evidence enforces NO EVIDENCE → NO ENTITY. A candidate is
/// grounded when its byte span `[start, end)` is a verbatim substring of
/// `source_text` (which it is by construction, since the span was sliced
/// from the source). The function additionally guards against accidental
/// spans that don't round-trip (defensive: catches a future refactor bug
/// where offsets drift from the text). Returns `None` for an ungrounded
/// candidate.
fn validate_evidence(
    c: Candidate,
    source_text: &str,
    tokens: &[Token],
    language: &str,
) -> Option<VisualEntity> {
    if c.start >= c.end || c.end > source_text.len() {
        return None;
    }
    let evidence = source_text.get(c.start..c.end)?;
    // The evidence must equal the candidate text verbatim. A candidate
    // whose text drifted from its span (e.g. due to a normalization step
    // that rewrote the surface) fails NO EVIDENCE → NO ENTITY.
    if evidence != c.text {
        return None;
    }
    // The POS model decides whether a gazetteer hit is a place name. A
    // place is a PROPN whose lemma keeps its capital ("Pavia" -> "Pavia"),
    // while a common noun tagged PROPN by the tagger has a lowercase lemma
    // ("la Corte" -> "corte") and stays out of the gazetteer. Without a tag,
    // the capitalization guard alone keeps the model-free behavior.
    let cased = semantic_core::lexicon::lexicon()
        .language_profiles
        .is_cased(language);
    let lexicon = semantic_core::lexicon::lexicon();
    let lowercase_propn_lemmas = lexicon.language_profiles.lowercase_propn_lemmas(language);
    let start_token = tokens.iter().find(|token| token.start == c.start);
    let gazetteer_eligible = start_token.is_none_or(|token| match token.upos.as_deref() {
        // In an uncased script the tag is the only signal: there is no
        // capitalized lemma to confirm it against. A tagger that lowercases
        // proper-noun lemmas (Spanish, Portuguese) must not veto a place the
        // index recognizes: `is_place` still enforces capitalization and the
        // per-language ambiguity list.
        Some("PROPN") => {
            !cased
                || token
                    .lemma
                    .as_deref()
                    .is_none_or(|lemma| lemma.chars().next().is_some_and(char::is_uppercase))
                || (lowercase_propn_lemmas && lexicon.is_place(&c.text, language))
        }
        Some(_) => false,
        None => true,
    });
    let proper_noun = start_token.is_some_and(|token| token.upos.as_deref() == Some("PROPN"));
    // A real name keeps its capital in the lemma ("Alberto" -> "Alberto"),
    // while a common noun promoted by its position does not ("Udienza" ->
    // "udienza"). The evidence is read from the first substantial word: dotted
    // initials ("J. D. Vance") are one-letter tokens and prove nothing. A
    // single-word surface is not judged by its lemma, because UDPipe
    // lowercases many real names ("Lazio"); without a lemma the orthography
    // alone decides, exactly as it did before the model.
    let multi_word = c.text.split_whitespace().count() >= 2;
    let name_token = tokens
        .iter()
        .filter(|token| token.start >= c.start && token.end <= c.end)
        .find(|token| token.text.chars().filter(|ch| ch.is_alphabetic()).count() >= 2);
    let name_evidence = !cased
        || !multi_word
        || name_token.is_none_or(|token| {
            // A PROPN tag is name evidence even when the model lowercases its
            // lemma (Spanish "Lucía" -> "lucía", Portuguese -> "_"). The
            // multi-word, two-capitalized-token rule still has to pass, so a
            // common noun promoted by sentence position cannot become a name.
            (lowercase_propn_lemmas && token.upos.as_deref() == Some("PROPN"))
                || token
                    .lemma
                    .as_deref()
                    .is_none_or(|lemma| lemma.chars().next().is_some_and(char::is_uppercase))
        });
    let mut entity_type = c.entity_type.unwrap_or_else(|| {
        classify_value_type(
            &c.text,
            gazetteer_eligible,
            proper_noun,
            name_evidence,
            language,
        )
    });
    // In a language that capitalizes common nouns (German) the title-case
    // shape alone cannot prove a person, and without a UDPipe model there is
    // no PROPN tag either. A curated person title immediately before the name
    // ("Präsident Emmanuel Macron") restores that evidence in every language,
    // without turning "Neue Halle" into a person.
    if entity_type == "VISUAL_CONCEPT"
        && c.text.split_whitespace().count() >= 2
        && start_token
            .and_then(|token| {
                tokens
                    .iter()
                    .position(|candidate| candidate.start == token.start)
            })
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| tokens.get(index))
            .is_some_and(|previous| is_title_word(&previous.text))
    {
        entity_type = "PERSON".to_string();
    }
    Some(VisualEntity {
        text: c.text.clone(),
        r#type: entity_type,
        score: 0.0,
        start: c.start,
        end: c.end,
        evidence: evidence.to_string(),
    })
}

/// Resolves the UDPipe model file for a language tag. Exposed for the
/// profile/model acceptance test, so a certified language without an
/// installed model fails loudly instead of degrading silently.
pub fn model_path(language: &str) -> Option<std::path::PathBuf> {
    pos::model_path(language)
}

// ── Stopword + blocklist tables ──────────────────────────────────────
//
// V1 uses curated, dependency-free tables. The tables are intentionally
// small and explicit so the extractor stays deterministic and auditable.
// Adding a word here is a one-line change; no model retraining.

#[cfg(test)]
#[path = "extractor_tests.rs"]
mod tests;
