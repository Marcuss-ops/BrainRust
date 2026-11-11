//! Deterministic detection of explicit pronouns and role nominals.
//!
//! Detection alone never assigns an identity. It only reports the exact source
//! span and the compatibility constraints (kind and number) that the resolver
//! must satisfy. A token inside a recognised named entity is not re-detected.

use semantic_core::discourse::word_spans;

/// One detected non-explicit referring expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DetectedReference {
    pub start: usize,
    pub end: usize,
    pub surface: String,
    pub reference_type: &'static str,
    /// `PERSON`, `ORG` or `ANY`.
    pub kind_hint: &'static str,
    /// `singular`, `plural` or `unknown`.
    pub number: &'static str,
}

#[derive(Clone, Copy)]
struct Term {
    text: &'static str,
    kind: &'static str,
    number: &'static str,
}

const fn term(text: &'static str, kind: &'static str, number: &'static str) -> Term {
    Term { text, kind, number }
}

/// Pronouns that can refer to a person. Demonstratives are `ANY` so the
/// resolver may select a non-person subject only when the scene leaves no
/// competing candidate.
const PERSON_PRONOUNS: &[Term] = &[
    term("lui", "PERSON", "singular"),
    term("lei", "PERSON", "singular"),
    term("egli", "PERSON", "singular"),
    term("ella", "PERSON", "singular"),
    term("loro", "PERSON", "plural"),
    term("essi", "PERSON", "plural"),
    term("esse", "PERSON", "plural"),
    term("he", "PERSON", "singular"),
    term("she", "PERSON", "singular"),
    term("him", "PERSON", "singular"),
    term("her", "PERSON", "singular"),
    term("they", "PERSON", "plural"),
    term("them", "PERSON", "plural"),
    term("él", "PERSON", "singular"),
    term("ella", "PERSON", "singular"),
    term("ellos", "PERSON", "plural"),
    term("ellas", "PERSON", "plural"),
    term("ele", "PERSON", "singular"),
    term("ela", "PERSON", "singular"),
    term("eles", "PERSON", "plural"),
    term("elas", "PERSON", "plural"),
    term("il", "PERSON", "singular"),
    term("elle", "PERSON", "singular"),
    term("ils", "PERSON", "plural"),
    term("elles", "PERSON", "plural"),
    term("er", "PERSON", "singular"),
    term("ihn", "PERSON", "singular"),
    term("ihm", "PERSON", "singular"),
    term("ihnen", "PERSON", "plural"),
    term("hij", "PERSON", "singular"),
    term("hem", "PERSON", "singular"),
    term("haar", "PERSON", "singular"),
    term("hen", "PERSON", "plural"),
    term("hun", "PERSON", "plural"),
    term("on", "PERSON", "singular"),
    term("ona", "PERSON", "singular"),
    term("oni", "PERSON", "plural"),
    term("one", "PERSON", "plural"),
    term("он", "PERSON", "singular"),
    term("она", "PERSON", "singular"),
    term("они", "PERSON", "plural"),
    term("dia", "PERSON", "singular"),
    term("ia", "PERSON", "singular"),
    term("mereka", "PERSON", "plural"),
    // German/Dutch third-person forms are syncretic; leave the number open
    // rather than silently choosing "she" or "they".
    term("sie", "PERSON", "unknown"),
    term("zij", "PERSON", "unknown"),
    term("ze", "PERSON", "unknown"),
];

const DEMONSTRATIVES: &[Term] = &[
    term("questo", "ANY", "singular"),
    term("questa", "ANY", "singular"),
    term("questi", "ANY", "plural"),
    term("queste", "ANY", "plural"),
    term("quello", "ANY", "singular"),
    term("quella", "ANY", "singular"),
    term("quelli", "ANY", "plural"),
    term("quelle", "ANY", "plural"),
    term("quest'ultimo", "ANY", "singular"),
    term("quest'ultima", "ANY", "singular"),
    term("this", "ANY", "singular"),
    term("that", "ANY", "singular"),
    term("these", "ANY", "plural"),
    term("those", "ANY", "plural"),
    term("este", "ANY", "singular"),
    term("esta", "ANY", "singular"),
    term("estos", "ANY", "plural"),
    term("estas", "ANY", "plural"),
    term("aquel", "ANY", "singular"),
    term("aquella", "ANY", "singular"),
    term("aquellos", "ANY", "plural"),
    term("aquellas", "ANY", "plural"),
    term("aquele", "ANY", "singular"),
    term("aquela", "ANY", "singular"),
    term("aqueles", "ANY", "plural"),
    term("aquelas", "ANY", "plural"),
    term("celui-ci", "ANY", "singular"),
    term("celle-ci", "ANY", "singular"),
    term("ceux-ci", "ANY", "plural"),
    term("celles-ci", "ANY", "plural"),
    term("dieser", "ANY", "singular"),
    term("diese", "ANY", "singular"),
    term("dieses", "ANY", "singular"),
    term("jener", "ANY", "singular"),
    term("jene", "ANY", "singular"),
    term("deze", "ANY", "singular"),
    term("dit", "ANY", "singular"),
    term("die", "ANY", "unknown"),
    term("dat", "ANY", "unknown"),
    term("ten", "ANY", "singular"),
    term("ta", "ANY", "singular"),
    term("to", "ANY", "singular"),
    term("te", "ANY", "plural"),
    term("этот", "ANY", "singular"),
    term("эта", "ANY", "singular"),
    term("это", "ANY", "singular"),
    term("эти", "ANY", "plural"),
    term("ini", "ANY", "singular"),
    term("itu", "ANY", "singular"),
    term("tersebut", "ANY", "unknown"),
];

/// Role nominals whose head is a person. They are resolved only through a
/// compatible identity; the role word alone never invents a subject.
const PERSON_ROLES: &[Term] = &[
    term("imprenditore", "PERSON", "singular"),
    term("imprenditrice", "PERSON", "singular"),
    term("fondatore", "PERSON", "singular"),
    term("fondatrice", "PERSON", "singular"),
    term("investitore", "PERSON", "singular"),
    term("investitrice", "PERSON", "singular"),
    term("uomo", "PERSON", "singular"),
    term("donna", "PERSON", "singular"),
    term("ragazzo", "PERSON", "singular"),
    term("ragazza", "PERSON", "singular"),
    term("dirigente", "PERSON", "singular"),
    term("avvocato", "PERSON", "singular"),
    term("avvocatessa", "PERSON", "singular"),
    term("giudice", "PERSON", "singular"),
    term("testimone", "PERSON", "singular"),
    term("vittima", "PERSON", "singular"),
    term("sospettato", "PERSON", "singular"),
    term("indagato", "PERSON", "singular"),
    term("imputato", "PERSON", "singular"),
    term("architetto", "PERSON", "singular"),
    term("ingegnere", "PERSON", "singular"),
    term("medico", "PERSON", "singular"),
    term("presidente", "PERSON", "singular"),
    term("amministratore", "PERSON", "singular"),
    term("socio", "PERSON", "singular"),
    term("entrepreneur", "PERSON", "singular"),
    term("founder", "PERSON", "singular"),
    term("investor", "PERSON", "singular"),
    term("businessman", "PERSON", "singular"),
    term("businesswoman", "PERSON", "singular"),
    term("executive", "PERSON", "singular"),
    term("lawyer", "PERSON", "singular"),
    term("judge", "PERSON", "singular"),
    term("witness", "PERSON", "singular"),
    term("victim", "PERSON", "singular"),
    term("suspect", "PERSON", "singular"),
    term("defendant", "PERSON", "singular"),
    term("architect", "PERSON", "singular"),
    term("engineer", "PERSON", "singular"),
    term("doctor", "PERSON", "singular"),
    term("president", "PERSON", "singular"),
    term("director", "PERSON", "singular"),
    term("partner", "PERSON", "singular"),
    term("empresario", "PERSON", "singular"),
    term("empresaria", "PERSON", "singular"),
    term("fundador", "PERSON", "singular"),
    term("fundadora", "PERSON", "singular"),
    term("inversor", "PERSON", "singular"),
    term("inversora", "PERSON", "singular"),
    term("abogado", "PERSON", "singular"),
    term("juez", "PERSON", "singular"),
    term("testigo", "PERSON", "singular"),
    term("víctima", "PERSON", "singular"),
    term("sospechoso", "PERSON", "singular"),
    term("acusado", "PERSON", "singular"),
    term("arquitecto", "PERSON", "singular"),
    term("ingeniero", "PERSON", "singular"),
    term("médico", "PERSON", "singular"),
    term("empresário", "PERSON", "singular"),
    term("empresária", "PERSON", "singular"),
    term("fundador", "PERSON", "singular"),
    term("fundadora", "PERSON", "singular"),
    term("investidor", "PERSON", "singular"),
    term("investidora", "PERSON", "singular"),
    term("advogado", "PERSON", "singular"),
    term("juiz", "PERSON", "singular"),
    term("testemunha", "PERSON", "singular"),
    term("vítima", "PERSON", "singular"),
    term("suspeito", "PERSON", "singular"),
    term("acusado", "PERSON", "singular"),
    term("arquiteto", "PERSON", "singular"),
    term("engenheiro", "PERSON", "singular"),
    term("fondateur", "PERSON", "singular"),
    term("fondatrice", "PERSON", "singular"),
    term("investisseur", "PERSON", "singular"),
    term("investisseuse", "PERSON", "singular"),
    term("avocat", "PERSON", "singular"),
    term("juge", "PERSON", "singular"),
    term("témoin", "PERSON", "singular"),
    term("victime", "PERSON", "singular"),
    term("suspect", "PERSON", "singular"),
    term("accusé", "PERSON", "singular"),
    term("architecte", "PERSON", "singular"),
    term("ingénieur", "PERSON", "singular"),
    term("médecin", "PERSON", "singular"),
    term("unternehmer", "PERSON", "singular"),
    term("unternehmerin", "PERSON", "singular"),
    term("gründer", "PERSON", "singular"),
    term("gründerin", "PERSON", "singular"),
    term("investor", "PERSON", "singular"),
    term("anwalt", "PERSON", "singular"),
    term("richter", "PERSON", "singular"),
    term("zeuge", "PERSON", "singular"),
    term("opfer", "PERSON", "singular"),
    term("verdächtiger", "PERSON", "singular"),
    term("angeklagter", "PERSON", "singular"),
    term("architekt", "PERSON", "singular"),
    term("ingenieur", "PERSON", "singular"),
    term("arzt", "PERSON", "singular"),
    term("ondernemer", "PERSON", "singular"),
    term("oprichter", "PERSON", "singular"),
    term("investeerder", "PERSON", "singular"),
    term("advocaat", "PERSON", "singular"),
    term("rechter", "PERSON", "singular"),
    term("getuige", "PERSON", "singular"),
    term("slachtoffer", "PERSON", "singular"),
    term("verdachte", "PERSON", "singular"),
    term("beklaagde", "PERSON", "singular"),
    term("ingenieur", "PERSON", "singular"),
    term("arts", "PERSON", "singular"),
    term("przedsiębiorca", "PERSON", "singular"),
    term("założyciel", "PERSON", "singular"),
    term("inwestor", "PERSON", "singular"),
    term("prawnik", "PERSON", "singular"),
    term("sędzia", "PERSON", "singular"),
    term("świadek", "PERSON", "singular"),
    term("ofiara", "PERSON", "singular"),
    term("podejrzany", "PERSON", "singular"),
    term("oskarżony", "PERSON", "singular"),
    term("architekt", "PERSON", "singular"),
    term("inżynier", "PERSON", "singular"),
    term("lekarz", "PERSON", "singular"),
    term("предприниматель", "PERSON", "singular"),
    term("основатель", "PERSON", "singular"),
    term("инвестор", "PERSON", "singular"),
    term("адвокат", "PERSON", "singular"),
    term("судья", "PERSON", "singular"),
    term("свидетель", "PERSON", "singular"),
    term("жертва", "PERSON", "singular"),
    term("подозреваемый", "PERSON", "singular"),
    term("обвиняемый", "PERSON", "singular"),
    term("архитектор", "PERSON", "singular"),
    term("инженер", "PERSON", "singular"),
    term("врач", "PERSON", "singular"),
    term("pengusaha", "PERSON", "singular"),
    term("pendiri", "PERSON", "singular"),
    term("investor", "PERSON", "singular"),
    term("pengacara", "PERSON", "singular"),
    term("hakim", "PERSON", "singular"),
    term("saksi", "PERSON", "singular"),
    term("korban", "PERSON", "singular"),
    term("tersangka", "PERSON", "singular"),
    term("terdakwa", "PERSON", "singular"),
    term("arsitek", "PERSON", "singular"),
    term("insinyur", "PERSON", "singular"),
    term("dokter", "PERSON", "singular"),
];

const ORG_ROLES: &[Term] = &[
    term("azienda", "ORG", "singular"),
    term("impresa", "ORG", "singular"),
    term("società", "ORG", "singular"),
    term("compagnia", "ORG", "singular"),
    term("company", "ORG", "singular"),
    term("firm", "ORG", "singular"),
    term("business", "ORG", "singular"),
    term("startup", "ORG", "singular"),
    term("organization", "ORG", "singular"),
    term("empresa", "ORG", "singular"),
    term("organización", "ORG", "singular"),
    term("société", "ORG", "singular"),
    term("entreprise", "ORG", "singular"),
    term("unternehmen", "ORG", "singular"),
    term("firma", "ORG", "singular"),
    term("bedrijf", "ORG", "singular"),
    term("spółka", "ORG", "singular"),
    term("компания", "ORG", "singular"),
    term("perusahaan", "ORG", "singular"),
];

const PHRASES: &[(&str, &str, &str)] = &[
    ("the former", "ANY", "singular"),
    ("the latter", "ANY", "singular"),
    ("the two", "PERSON", "plural"),
    ("ce dernier", "ANY", "singular"),
    ("cette dernière", "ANY", "singular"),
    ("les deux", "PERSON", "plural"),
    ("questo ultimo", "ANY", "singular"),
    ("questa ultima", "ANY", "singular"),
    ("i due", "PERSON", "plural"),
    ("los dos", "PERSON", "plural"),
    ("os dois", "PERSON", "plural"),
    ("die beiden", "PERSON", "plural"),
    ("de twee", "PERSON", "plural"),
    ("obaj", "PERSON", "plural"),
    ("оба", "PERSON", "plural"),
    ("keduanya", "PERSON", "plural"),
];

/// Finds pronouns and role nominals, preserving exact byte offsets.
pub(super) fn detect(text: &str, language: &str) -> Vec<DetectedReference> {
    let words = word_spans(text);
    let mut found = Vec::new();
    let mut index = 0;
    while index < words.len() {
        // Multi-word phrases first (longest match wins).
        let mut matched = None;
        for &(phrase, kind, number) in PHRASES {
            if !phrase_applies(phrase, language) {
                continue;
            }
            let count = phrase.split_whitespace().count();
            if index + count > words.len() {
                continue;
            }
            let candidate = words[index..index + count]
                .iter()
                .map(|word| word.lowered.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if candidate == phrase {
                matched = Some((
                    words[index].start,
                    words[index + count - 1].end,
                    count,
                    kind,
                    number,
                ));
                break;
            }
        }
        if let Some((start, end, count, kind, number)) = matched {
            found.push(DetectedReference {
                start,
                end,
                surface: text[start..end].to_string(),
                reference_type: "pronoun",
                kind_hint: kind,
                number,
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

fn phrase_applies(phrase: &str, language: &str) -> bool {
    match phrase {
        "the former" | "the latter" | "the two" => language == "en",
        "ce dernier" | "cette dernière" | "les deux" => language == "fr",
        "questo ultimo" | "questa ultima" | "i due" => language == "it",
        "los dos" => language == "es",
        "os dois" => language == "pt",
        "die beiden" => language == "de",
        "de twee" => language == "nl",
        "obaj" => language == "pl",
        "оба" => language == "ru",
        "keduanya" => language == "id",
        _ => false,
    }
}

fn lookup(lowered: &str, language: &str) -> Option<DetectedReference> {
    let found = PERSON_PRONOUNS
        .iter()
        .chain(DEMONSTRATIVES)
        .find(|term| term.text == lowered && term_applies(term.text, language));
    if let Some(term) = found {
        return Some(DetectedReference {
            start: 0,
            end: 0,
            surface: String::new(),
            reference_type: "pronoun",
            kind_hint: term.kind,
            number: term.number,
        });
    }
    for (table, reference_type) in [
        (PERSON_ROLES, "nominal_reference"),
        (ORG_ROLES, "nominal_reference"),
    ] {
        if let Some(term) = table
            .iter()
            .find(|term| term.text == lowered && term_applies(term.text, language))
        {
            return Some(DetectedReference {
                start: 0,
                end: 0,
                surface: String::new(),
                reference_type,
                kind_hint: term.kind,
                number: term.number,
            });
        }
    }
    None
}

/// Cross-language false positives are excluded by the language tag. This is
/// vocabulary control, not a language model: a term resolves only in the
/// languages where it is a pronoun/role rather than an article or verb.
fn term_applies(text: &str, language: &str) -> bool {
    match text {
        "il" | "elle" | "ils" | "elles" => language == "fr",
        "to" | "ten" | "ta" | "on" | "ona" | "oni" | "one" => language == "pl",
        "dit" | "deze" | "hij" | "hem" | "haar" | "hen" | "hun" | "zij" | "ze" => language == "nl",
        "die" | "dat" => matches!(language, "de" | "nl"),
        "er" | "ihn" | "ihm" | "ihnen" | "sie" => language == "de",
        "ele" | "ela" | "eles" | "elas" => language == "pt",
        "él" | "ellos" | "ellas" => language == "es",
        "ella" => matches!(language, "it" | "es"),
        "he" | "she" | "him" | "her" | "they" | "them" | "this" | "that" | "these" | "those" => {
            language == "en"
        }
        "lui" => matches!(language, "it" | "fr"),
        "lei" | "egli" | "loro" | "essi" | "esse" => language == "it",
        "он" | "она" | "они" | "этот" | "эта" | "это" | "эти" => {
            language == "ru"
        }
        "dia" | "ia" | "mereka" | "ini" | "itu" | "tersebut" => language == "id",
        "firma" => matches!(language, "de" | "nl" | "pl"),
        "investor" => matches!(language, "en" | "de"),
        "entrepreneur" | "suspect" => matches!(language, "en" | "fr"),
        "president" | "director" | "partner" | "business" | "company" | "firm" | "startup" => {
            language == "en"
        }
        "empresa" | "fundador" | "fundadora" | "médico" | "acusado" => {
            matches!(language, "es" | "pt")
        }
        _ => true,
    }
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
        ] {
            let refs = detect(text, language);
            assert!(
                refs.iter().any(|reference| reference.surface == expected),
                "{language}: {refs:?}"
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
