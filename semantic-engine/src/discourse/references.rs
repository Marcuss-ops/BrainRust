//! Deterministic detection of explicit pronouns and role nominals.
//!
//! Detection alone never assigns an identity. It only reports the exact source
//! span and the compatibility constraints (kind and number) that the resolver
//! must satisfy. A token inside a recognised named entity is not re-detected.

use semantic_core::discourse::word_spans;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// One detected non-explicit referring expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DetectedReference {
    pub start: usize,
    pub end: usize,
    pub surface: String,
    pub reference_type: String,
    /// `PERSON`, `ORG` or `ANY`.
    pub kind_hint: String,
    /// `masculine`, `feminine`, or `unknown` from the matched pronoun/role.
    pub gender_hint: String,
    /// `singular`, `plural` or `unknown`.
    pub number: String,
}

#[derive(Deserialize)]
struct ReferenceInventory {
    schema: String,
    languages: BTreeMap<String, LanguageReferenceTerms>,
}

#[derive(Deserialize)]
struct LanguageReferenceTerms {
    terms: Vec<ReferenceTerm>,
    phrases: Vec<ReferencePhrase>,
}

#[derive(Deserialize)]
struct ReferenceTerm {
    text: String,
    #[serde(rename = "type")]
    reference_type: String,
    kind: String,
    number: String,
}

#[derive(Deserialize)]
struct ReferencePhrase {
    text: String,
    kind: String,
    number: String,
}

#[derive(Deserialize)]
struct PronounInventory {
    pronouns: Vec<PronounGender>,
}

#[derive(Deserialize)]
struct PronounGender {
    surface: String,
    gender: String,
}

#[derive(Deserialize)]
struct RoleInventory {
    roles: Vec<RoleDefinition>,
}

#[derive(Deserialize)]
struct RoleDefinition {
    surfaces: Vec<RoleSurface>,
}

#[derive(Deserialize)]
struct RoleSurface {
    text: String,
    gender: String,
}

fn pronoun_gender(language: &str, surface: &str) -> String {
    static INVENTORIES: OnceLock<BTreeMap<(String, String), String>> = OnceLock::new();
    let inventories = INVENTORIES.get_or_init(|| {
        let sources = [
            (
                "de",
                include_str!("../../../lexicon/languages/de/pronouns.json"),
            ),
            (
                "en",
                include_str!("../../../lexicon/languages/en/pronouns.json"),
            ),
            (
                "es",
                include_str!("../../../lexicon/languages/es/pronouns.json"),
            ),
            (
                "fr",
                include_str!("../../../lexicon/languages/fr/pronouns.json"),
            ),
            (
                "id",
                include_str!("../../../lexicon/languages/id/pronouns.json"),
            ),
            (
                "it",
                include_str!("../../../lexicon/languages/it/pronouns.json"),
            ),
            (
                "nl",
                include_str!("../../../lexicon/languages/nl/pronouns.json"),
            ),
            (
                "pl",
                include_str!("../../../lexicon/languages/pl/pronouns.json"),
            ),
            (
                "pt",
                include_str!("../../../lexicon/languages/pt/pronouns.json"),
            ),
            (
                "ru",
                include_str!("../../../lexicon/languages/ru/pronouns.json"),
            ),
            (
                "tr",
                include_str!("../../../lexicon/languages/tr/pronouns.json"),
            ),
        ];
        let mut output = BTreeMap::new();
        for (language, source) in sources {
            let inventory: PronounInventory = serde_json::from_str(source)
                .expect("embedded structured pronoun inventory must be valid");
            for item in inventory.pronouns {
                let gender = if matches!(item.gender.as_str(), "masculine" | "feminine") {
                    item.gender
                } else {
                    "unknown".to_string()
                };
                output
                    .entry((language.to_string(), item.surface.to_lowercase()))
                    .and_modify(|existing: &mut String| {
                        if existing != &gender {
                            *existing = "unknown".to_string();
                        }
                    })
                    .or_insert(gender);
            }
        }
        output
    });
    inventories
        .get(&(language.to_string(), surface.to_lowercase()))
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}

fn reference_inventory(language: &str) -> Option<&'static LanguageReferenceTerms> {
    static INVENTORY: OnceLock<ReferenceInventory> = OnceLock::new();
    let inventory = INVENTORY.get_or_init(|| {
        let inventory: ReferenceInventory = serde_json::from_str(include_str!(
            "../../../lexicon/discourse_reference_terms_v1.json"
        ))
        .expect("embedded discourse reference terms must be valid");
        assert_eq!(
            inventory.schema, "discourse_reference_terms.v1",
            "embedded discourse reference terms schema must be supported"
        );
        inventory
    });
    inventory.languages.get(language)
}

fn role_gender(language: &str, surface: &str) -> String {
    static INVENTORIES: OnceLock<BTreeMap<String, BTreeMap<String, String>>> = OnceLock::new();
    let inventories = INVENTORIES.get_or_init(|| {
        let sources = [
            ("en", include_str!("../../../lexicon/roles/en.json")),
            ("it", include_str!("../../../lexicon/roles/it.json")),
            ("es", include_str!("../../../lexicon/roles/es.json")),
            ("de", include_str!("../../../lexicon/roles/de.json")),
            ("fr", include_str!("../../../lexicon/roles/fr.json")),
            ("id", include_str!("../../../lexicon/roles/id.json")),
            ("nl", include_str!("../../../lexicon/roles/nl.json")),
            ("pl", include_str!("../../../lexicon/roles/pl.json")),
            ("pt", include_str!("../../../lexicon/roles/pt.json")),
            ("ru", include_str!("../../../lexicon/roles/ru.json")),
            ("tr", include_str!("../../../lexicon/roles/tr.json")),
        ];
        let mut output = BTreeMap::new();
        for (language, source) in sources {
            let inventory: RoleInventory =
                serde_json::from_str(source).expect("embedded role inventory must be valid");
            let mut forms = BTreeMap::new();
            for role in inventory.roles {
                for surface in role.surfaces {
                    let gender = if matches!(surface.gender.as_str(), "masculine" | "feminine") {
                        surface.gender
                    } else {
                        "unknown".to_string()
                    };
                    forms
                        .entry(surface.text.to_lowercase())
                        .and_modify(|known| {
                            if known != &gender {
                                *known = "unknown".to_string();
                            }
                        })
                        .or_insert(gender);
                }
            }
            output.insert(language.to_string(), forms);
        }
        output
    });
    inventories
        .get(language)
        .and_then(|forms| forms.get(&surface.to_lowercase()))
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}

/// Finds pronouns and role nominals, preserving exact byte offsets.
pub(super) fn detect(text: &str, language: &str) -> Vec<DetectedReference> {
    let Some(inventory) = reference_inventory(language) else {
        return Vec::new();
    };
    let words = word_spans(text);
    let mut found = Vec::new();
    let mut index = 0;
    while index < words.len() {
        // Multi-word phrases first (longest match wins).
        let mut matched = None;
        for phrase in &inventory.phrases {
            let count = phrase.text.split_whitespace().count();
            if index + count > words.len() {
                continue;
            }
            let candidate = words[index..index + count]
                .iter()
                .map(|word| word.lowered.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if candidate == phrase.text {
                matched = Some((
                    words[index].start,
                    words[index + count - 1].end,
                    count,
                    phrase,
                ));
                break;
            }
        }
        if let Some((start, end, count, phrase)) = matched {
            found.push(DetectedReference {
                start,
                end,
                surface: text[start..end].to_string(),
                reference_type: "pronoun".to_string(),
                kind_hint: phrase.kind.clone(),
                gender_hint: "unknown".to_string(),
                number: phrase.number.clone(),
            });
            index += count;
            continue;
        }

        let word = &words[index];
        for (start, lowered) in variants(text, word) {
            if let Some(term) = lookup(&lowered, language) {
                found.push(DetectedReference {
                    start,
                    end: word.end,
                    surface: text[start..word.end].to_string(),
                    ..term
                });
                break;
            }
        }
        index += 1;
    }
    found
}

/// Full token plus the suffix after an apostrophe, so Italian `l'imprenditore`
/// yields the role span `imprenditore` without inventing an offset.
fn variants(text: &str, word: &semantic_core::discourse::WordSpan) -> Vec<(usize, String)> {
    let mut values = vec![(word.start, word.lowered.clone())];
    let raw = &text[word.start..word.end];
    if let Some((offset, character)) = raw
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '\'' | '\u{2019}'))
    {
        let start = word.start + offset + character.len_utf8();
        if start < word.end {
            values.push((start, text[start..word.end].to_lowercase()));
        }
    }
    values
}

fn lookup(lowered: &str, language: &str) -> Option<DetectedReference> {
    let term = reference_inventory(language)?
        .terms
        .iter()
        .find(|term| term.text == lowered)?;
    let gender_hint = match term.reference_type.as_str() {
        "pronoun" => pronoun_gender(language, lowered),
        "nominal_reference" if term.kind == "PERSON" => role_gender(language, lowered),
        _ => "unknown".to_string(),
    };
    Some(DetectedReference {
        start: 0,
        end: 0,
        surface: String::new(),
        reference_type: term.reference_type.clone(),
        kind_hint: term.kind.clone(),
        gender_hint,
        number: term.number.clone(),
    })
}

/// True when `offset` sits inside double quotes or guillemets. Quoted speech
/// is not attributed to a narrator: the resolver reports it as unresolved
/// instead of guessing the speaker.
pub(super) fn in_quotes(text: &str, offset: usize) -> bool {
    let mut quoted = false;
    for (index, character) in text.char_indices() {
        if index >= offset {
            break;
        }
        match character {
            '"' => quoted = !quoted,
            '\u{201c}' | '«' => quoted = true,
            '\u{201d}' | '»' => quoted = false,
            _ => {}
        }
    }
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italian_article_keeps_the_role_span_exact() {
        let text = "L'imprenditore decise di vendere.";
        let refs = detect(text, "it");
        let role = refs
            .iter()
            .find(|reference| reference.surface == "imprenditore")
            .expect("role must be found");
        assert_eq!(
            &text[role.start..role.end],
            "imprenditore",
            "the article must not be part of the role span"
        );
        assert_eq!(role.reference_type, "nominal_reference");
        assert_eq!(role.kind_hint, "PERSON");
    }

    #[test]
    fn pronouns_are_detected_in_all_supported_languages() {
        for (language, text, expected) in [
            ("it", "Lui decise.", "Lui"),
            ("en", "She decided.", "She"),
            ("es", "Ella decidió.", "Ella"),
            ("pt", "Ele decidiu.", "Ele"),
            ("fr", "Elle a décidé.", "Elle"),
            ("de", "Er entschied.", "Er"),
            ("nl", "Hij besloot.", "Hij"),
            ("pl", "On zdecydował.", "On"),
            ("ru", "Он решил.", "Он"),
            ("id", "Dia memutuskan.", "Dia"),
            ("tr", "O karar verdi.", "O"),
        ] {
            let refs = detect(text, language);
            assert!(
                refs.iter().any(|reference| reference.surface == expected),
                "{language}: {refs:?}"
            );
        }
    }

    #[test]
    fn role_gender_inventory_includes_localized_lexicons() {
        for (language, surface, expected) in [
            ("de", "Bürgermeisterin", "feminine"),
            ("pl", "ministra", "feminine"),
            ("ru", "певица", "feminine"),
            ("tr", "başkan", "unknown"),
        ] {
            assert_eq!(
                role_gender(language, surface),
                expected,
                "{language}: {surface}"
            );
        }
    }

    #[test]
    fn language_scope_excludes_cross_language_articles_and_prepositions() {
        assert!(
            detect("Il progetto continua.", "it")
                .iter()
                .all(|reference| reference.surface != "Il"),
            "Italian article `il` must not become a French pronoun"
        );
        assert!(
            detect("Go to the office.", "en")
                .iter()
                .all(|reference| reference.surface != "to"),
            "English preposition `to` must not become a Polish pronoun"
        );
        assert!(
            detect("O karar verdi.", "en")
                .iter()
                .all(|reference| reference.surface != "O"),
            "Turkish pronoun `o` must be language-scoped"
        );
        assert!(
            detect("On zdecydował.", "pl")
                .iter()
                .any(|reference| reference.surface == "On"),
            "Polish pronoun `on` must stay detectable"
        );
    }

    #[test]
    fn quoted_speech_is_flagged() {
        let text = "\"Lui è colpevole\", disse Mario.";
        let offset = text.find("Lui").unwrap();
        assert!(in_quotes(text, offset));
        assert!(!in_quotes(text, text.find("Mario").unwrap()));
    }
}
