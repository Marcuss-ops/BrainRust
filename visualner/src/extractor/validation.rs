//! Source-evidence validation (NO EVIDENCE → NO ENTITY).

use crate::types::VisualEntity;

use super::names::is_name_particle;
use super::stopwords::{is_stop_word, is_title_word};
use super::tokenizer::Token;
use super::values::{classify_value_type, has_suspicious_asr_digit_fusion, Candidate};

/// validate_evidence enforces NO EVIDENCE → NO ENTITY. A candidate is
/// grounded when its byte span `[start, end)` is a verbatim substring of
/// `source_text` (which it is by construction, since the span was sliced
/// from the source). The function additionally guards against accidental
/// spans that don't round-trip (defensive: catches a future refactor bug
/// where offsets drift from the text). Returns `None` for an ungrounded
/// candidate.
pub(crate) fn validate_evidence(
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
    if has_suspicious_asr_digit_fusion(evidence) {
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
    // A sentence-initial name is lemmatized as its lowercase homograph by the
    // tagger ("Chiara Poggi" -> lemma "chiaro" at the start of a sentence,
    // "Chiara" mid-sentence). The tag survives that: when every significant
    // token of the surface is a PROPN, the surface is a proper name regardless
    // of where the sentence begins. Particles ("d'", "de", "van") are allowed
    // between the names; any other non-PROPN token vetoes the rule, so
    // "Hall of Fame" or "Udienza Preliminare" stay visual concepts.
    let span_tokens: Vec<&Token> = tokens
        .iter()
        .filter(|token| token.start >= c.start && token.end <= c.end)
        .collect();
    let significant_tokens: Vec<&&Token> = span_tokens
        .iter()
        .filter(|token| token.text.chars().filter(|ch| ch.is_alphabetic()).count() >= 2)
        .collect();
    let all_propn_names = multi_word
        && significant_tokens.len() >= 2
        && span_tokens.iter().all(|token| {
            token.text.chars().filter(|ch| ch.is_alphabetic()).count() < 2
                || token.upos.as_deref() == Some("PROPN")
                || is_name_particle(&token.text, language)
        })
        && significant_tokens
            .iter()
            .all(|token| token.upos.as_deref() == Some("PROPN"))
        // A procedural phrase ("Udienza Preliminare") is tagged PROPN on every
        // token too, but the tagger lowercases both lemmas. At least one real
        // name keeps its capital in the lemma, and that is the difference.
        && significant_tokens.iter().any(|token| {
            token
                .lemma
                .as_deref()
                .is_some_and(|lemma| lemma.chars().next().is_some_and(char::is_uppercase))
        });
    let name_evidence = !cased
        || !multi_word
        || all_propn_names
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
    Some(trim_leading_determiner(
        VisualEntity {
            text: c.text.clone(),
            r#type: entity_type,
            score: 0.0,
            start: c.start,
            end: c.end,
            evidence: evidence.to_string(),
        },
        tokens,
        source_text,
        language,
    ))
}

/// A determiner is not part of an organization's or facility's own name
/// ("The Harbour Stadium", "Das Stadion Süd", "Lo Stadio Aurora"). The
/// extractor can tag a sentence-initial article as part of a title-case run
/// when the tagger mislabels it, so the identity is re-anchored at the first
/// substantive token. Locations keep their surface ("The Hague" is a place
/// name); evidence is re-sliced from the source, so grounding is preserved.
fn trim_leading_determiner(
    mut entity: VisualEntity,
    tokens: &[Token],
    source_text: &str,
    language: &str,
) -> VisualEntity {
    if !matches!(entity.r#type.as_str(), "ORG" | "ORGANIZATION" | "FACILITY") {
        return entity;
    }
    while let Some(token) = tokens.iter().find(|token| token.start == entity.start) {
        if token.end >= entity.end {
            break;
        }
        if token.upos.as_deref() != Some("DET") && !is_stop_word(&token.text, language) {
            break;
        }
        let mut next = token.end;
        while next < entity.end && source_text.as_bytes()[next].is_ascii_whitespace() {
            next += 1;
        }
        let Some(text) = source_text.get(next..entity.end) else {
            break;
        };
        if text.split_whitespace().count() < 2 {
            break;
        }
        entity.start = next;
        entity.text = text.to_string();
        entity.evidence = text.to_string();
    }
    entity
}
