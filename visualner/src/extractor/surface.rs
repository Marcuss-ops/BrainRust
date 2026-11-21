//! Canonical (dictionary) surfaces for identities.
//!
//! The mention keeps the source span verbatim, so evidence stays grounded; the
//! identity may publish a dictionary form instead. Polish is the first language
//! with a documented rule: the gold annotations and the overlays use the
//! nominative apposition, while the scripts contain oblique cases
//! ("Krakowie", "mapę", "Ewę Zielińską"). The UDPipe model already supplies the
//! lemma and the morphology, so the rule is data-driven and falls back to the
//! source surface when no model is installed.
//!
//! Only singular nouns, feminine adjectives and personal names are rewritten.
//! Plurals keep the source surface ("kaski" is both nominative and accusative
//! plural), and acronyms or multi-token surfaces that do not tokenize back to
//! the same text are left untouched.

use super::tokenizer::{tokenize, Token};

/// Dictionary surface for one identity mention. `kind` is the closed entity
/// type; `language` is the BCP-47 tag of the scene.
pub fn canonical_surface(surface: &str, language: &str, kind: &str) -> String {
    let key = language.split(['-', '_']).next().unwrap_or("");
    // Place inflections are curated aliases with exact spans, so they resolve
    // even when the tagger proposes a wrong lemma ("Krakowie" -> "Kraek").
    // `canonical_query_place` applies the language's documented policy: Polish
    // publishes the nominative apposition, Russian keeps the source form.
    if kind == "LOCATION" {
        if let Some(canonical) =
            semantic_core::lexicon::lexicon().canonical_query_place(surface, key)
        {
            return canonical.to_string();
        }
    }
    if key != "pl" || !matches!(kind, "PERSON" | "VISUAL_CONCEPT" | "OBJECT") {
        return surface.to_string();
    }
    let Ok(tokens) = tokenize(surface, key) else {
        return surface.to_string();
    };
    let joined = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if joined != surface {
        // Punctuation, hyphens or a tokenizer split mean the rewrite would no
        // longer describe the mention; keep the source surface.
        return surface.to_string();
    }
    let mut words = Vec::with_capacity(tokens.len());
    let mut changed = false;
    for token in &tokens {
        let word = normalized_token(token, kind).unwrap_or_else(|| token.text.clone());
        changed |= word != token.text;
        words.push(word);
    }
    if changed {
        words.join(" ")
    } else {
        surface.to_string()
    }
}

/// One token in its dictionary form, or `None` when the source form must stay.
fn normalized_token(token: &Token, kind: &str) -> Option<String> {
    let lemma = token.lemma.as_deref()?;
    let upos = token.upos.as_deref().unwrap_or("");
    if lemma.is_empty()
        || lemma == "_"
        || lemma.contains(char::is_whitespace)
        || !lemma
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '\'')
    {
        return None;
    }
    // Acronyms and all-caps surfaces are never inflected dictionary words.
    if token.text.chars().count() > 1
        && token
            .text
            .chars()
            .all(|ch| ch.is_uppercase() || !ch.is_alphabetic())
    {
        return None;
    }
    if lemma == token.text {
        return None;
    }
    match kind {
        "PERSON" => person_form(token, lemma, upos),
        "VISUAL_CONCEPT" | "OBJECT" => concept_form(token, lemma, upos),
        _ => None,
    }
}

/// Personal names take the nominative lemma. The tagger occasionally labels a
/// first name as an adverb ("Annie" -> "Anna"), so a capitalized lemma is
/// accepted for ADV too; function words are never rewritten.
fn person_form(token: &Token, lemma: &str, upos: &str) -> Option<String> {
    if !matches!(upos, "NOUN" | "PROPN" | "ADJ" | "ADV") {
        return None;
    }
    if !lemma
        .chars()
        .next()
        .is_some_and(|ch| ch.is_uppercase() || ch.is_lowercase())
    {
        return None;
    }
    let mut word = capitalize(lemma);
    // A feminine adjective in an oblique case gets a masculine lemma
    // ("Zielińską" -> "Zieliński"); agreement requires the feminine ending.
    if feature(&token.features, "Gender") == Some("Fem")
        && !word.ends_with('a')
        && matches!(token.text.chars().last(), Some('ą' | 'ę'))
    {
        word.pop();
        word.push('a');
    }
    (word != token.text).then_some(word)
}

/// A singular common noun in an oblique case takes the nominative lemma
/// ("mapę" -> "mapa"). Plurals keep the source surface: "kaski" is both the
/// nominative and the accusative plural, and the gold keeps the plural.
fn concept_form(token: &Token, lemma: &str, upos: &str) -> Option<String> {
    if upos != "NOUN" {
        return None;
    }
    if feature(&token.features, "Number") != Some("Sing") {
        return None;
    }
    match feature(&token.features, "Case") {
        Some("Nom") | None => None,
        Some(_) => Some(lemma.to_string()),
    }
}

/// One `Key=Value` feature from the `Feats` column.
fn feature<'a>(features: &'a Option<String>, key: &str) -> Option<&'a str> {
    features.as_deref()?.split('|').find_map(|item| {
        let (name, value) = item.split_once('=')?;
        (name == key).then_some(value)
    })
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_polish_model() -> bool {
        super::super::model_path("pl").is_some()
    }

    #[test]
    fn polish_oblique_nouns_use_the_nominative_lemma() {
        if !has_polish_model() {
            return;
        }
        assert_eq!(canonical_surface("mapę", "pl", "VISUAL_CONCEPT"), "mapa");
        assert_eq!(canonical_surface("siatkę", "pl", "OBJECT"), "siatka");
        assert_eq!(canonical_surface("kamerę", "pl", "OBJECT"), "kamera");
    }

    #[test]
    fn polish_plurals_and_nominatives_keep_the_source_surface() {
        if !has_polish_model() {
            return;
        }
        assert_eq!(canonical_surface("kaski", "pl", "VISUAL_CONCEPT"), "kaski");
        assert_eq!(
            canonical_surface("raport", "pl", "VISUAL_CONCEPT"),
            "raport"
        );
    }

    #[test]
    fn polish_person_names_take_the_nominative_form() {
        if !has_polish_model() {
            return;
        }
        assert_eq!(
            canonical_surface("Annie Kowalskiej", "pl", "PERSON"),
            "Anna Kowalska"
        );
        assert_eq!(
            canonical_surface("Ewę Zielińską", "pl", "PERSON"),
            "Ewa Zielińska"
        );
        assert_eq!(
            canonical_surface("Piotr Nowak", "pl", "PERSON"),
            "Piotr Nowak"
        );
    }

    #[test]
    fn polish_places_resolve_through_the_curated_table() {
        if !has_polish_model() {
            return;
        }
        assert_eq!(canonical_surface("Krakowie", "pl", "LOCATION"), "Kraków");
        assert_eq!(canonical_surface("Poznaniu", "pl", "LOCATION"), "Poznań");
    }

    #[test]
    fn other_languages_keep_the_source_surface() {
        assert_eq!(
            canonical_surface("Stadion Noord", "nl", "FACILITY"),
            "Stadion Noord"
        );
        assert_eq!(canonical_surface("Annie", "nl", "PERSON"), "Annie");
    }
}
