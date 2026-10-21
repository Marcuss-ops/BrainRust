//! Contextual verbal-predicate detection (GEN-01).
//!
//! Deterministic, model-free rule that separates verbal predicates from
//! visualizable concepts in Italian text. The distinction is CONTEXTUAL,
//! not lexical: the same surface form is dropped or kept depending on its
//! slot ("guarda" in "e guarda il mare" is a predicate and must not leak as
//! a VISUAL_CONCEPT, while "guarda" in "La guarda del castello" is a noun
//! and stays extractable). A global black list of verb lemmas would erase the
//! nominal readings, so the decision is taken from morphology plus the
//! surrounding slots:
//!
//! ```text
//! nominal slot (il/una/della/... before)  → concept, keep
//! clitic + finite verb ("si trova")       → predicate, drop
//! verbal left slot (e/che/... or PROPN subject) AND
//! verbal right slot (object/complement introducer) → predicate, drop
//! ```
//!
//! When the UDPipe model is installed the `upos` tags refine the decision;
//! without a model the curated Italian vocabularies below are the fallback,
//! mirroring how `pos.rs` defers to the stopword lists.
//!
//! Scope: Italian (`it`). Other languages keep their current behavior until
//! their own vocabularies are curated and corpus-verified.

use super::Token;

/// Elided clitic particles that introduce a reflexive/pronominal predicate
/// ("si trova", "si affaccia", "ci sono").
const CLITICS: &[&str] = &["si", "ci", "mi", "ti", "vi", "ne"];

/// Clause linkers: what follows them is predicative, not a bare conjunct,
/// as long as the morphology is verbal and the right slot selects a verb
/// reading. Deliberately excludes "o"/"oppure", which coordinate noun
/// phrases as often as verb phrases ("la guida o il manuale").
const CLAUSE_LINKERS: &[&str] = &[
    "e",
    "ed",
    "che",
    "ma",
    "mentre",
    "quando",
    "perché",
    "poiché",
    "giacché",
    "siccome",
    "se",
    "dove",
    "affinché",
    "benché",
    "finché",
];

/// Determiners, prepositions and their contractions. A token immediately
/// AFTER one of these is nominal by construction ("il salto", "della
/// Lombardia", "sull'Aterno"), whatever its morphology says.
const NOMINAL_INTRODUCERS: &[&str] = &[
    "il", "lo", "la", "i", "gli", "le", "un", "uno", "una", "l'", "l’", "un'", "un’", "di", "a",
    "da", "in", "con", "su", "per", "tra", "fra", "del", "dello", "della", "dei", "degli", "delle",
    "al", "allo", "alla", "ai", "agli", "alle", "dal", "dallo", "dalla", "dai", "dagli", "dalle",
    "nel", "nello", "nella", "nei", "negli", "nelle", "sul", "sullo", "sulla", "sui", "sugli",
    "sulle", "dell'", "dell’", "nell'", "nell’", "sull'", "sull’", "all'", "all’", "dall'",
    "dall’", "questo", "questa", "questi", "queste", "quel", "quella", "quello", "quei", "quelle",
    "quegli", "suo", "sua", "suoi", "sue", "mio", "mia", "miei", "mie", "tuo", "tua", "tuoi",
    "tue", "nostro", "nostra", "nostri", "nostre", "vostro", "vostra", "vostri", "vostre", "loro",
    "ogni", "alcuni", "alcune", "alcun", "qualche", "tutti", "tutte", "molti", "molte", "pochi",
    "poche",
];

/// Right-slot verbal continuations: an accusative/dative determiner, an
/// object clitic, a verbal adverb, or a locative/terminative contraction
/// ("sorge nella pianura", "sorge sull'Aterno"). Genitive "di/del/..." is
/// deliberately excluded because a noun also takes a genitive complement
/// ("vino del Chianti", "latte della fattoria").
const VERBAL_CONTINUATIONS: &[&str] = &[
    // accusative determiners
    "il", "lo", "la", "i", "gli", "le", "un", "uno", "una", "l'", "l’", "un'", "un’",
    // object clitics
    "mi", "ti", "ci", "vi", "li", "ne", "si", // verbal adverbs
    "non", "più", "anche", "sempre", "spesso", "già", "ancora", "mai", "poi", "solo", "proprio",
    "invece", "quasi", // locative / terminative contractions
    "nel", "nello", "nella", "nei", "negli", "nelle", "sul", "sullo", "sulla", "sui", "sugli",
    "sulle", "al", "allo", "alla", "ai", "agli", "alle", "nell'", "nell’", "sull'", "sull’",
    "all'", "all’",
];

/// Multi-character Italian inflection endings compatible with a finite verb,
/// participle or gerund. Single-character endings are handled below.
const VERB_ENDINGS: &[&str] = &[
    "iamo", "ate", "ete", "ite", "ono", "ano", "ino", "ava", "avi", "avamo", "avate", "avano",
    "eva", "evi", "evamo", "evate", "evano", "iva", "ivi", "ivamo", "ivate", "ivano", "arono",
    "erono", "irono", "erò", "erai", "erà", "eranno", "irà", "irai", "iremo", "irete", "iranno",
    "erei", "eresti", "erebbe", "eremmo", "ereste", "erebbero", "irei", "iresti", "irebbe",
    "iremmo", "ireste", "irebbero", "ando", "endo", "ato", "ata", "ati", "ate", "uto", "uta",
    "uti", "ute", "ò",
];

/// Elided determiner/preposition compounds that begin the next token
/// ("l'auto", "un'idea", "sull'Adriatico"). Prefix matching is safe here
/// because every entry carries the apostrophe.
const VERBAL_CONTINUATION_PREFIXES: &[&str] = &[
    "l'", "l’", "un'", "un’", "nell'", "nell’", "sull'", "sull’", "all'", "all’",
];

/// Surface endings that mark an unmistakable noun reading (città,
/// università, costruzione, velocemente): these never count as verbal
/// morphology, so a bare "e università ..." cannot be dropped as a verb.
const NOUN_ENDINGS: &[&str] = &[
    "zione", "sione", "mente", "tà", "ità", "ù", "ì", "è", "é", "à",
];

/// is_verbal_predicate reports whether the candidate phrase spanning
/// `tokens[phrase_start..phrase_end]` is a verbal predicate in context and
/// must not become a VISUAL_CONCEPT entity.
pub(super) fn is_verbal_predicate(
    tokens: &[Token],
    phrase_start: usize,
    phrase_end: usize,
    language: &str,
) -> bool {
    if !is_italian(language) || phrase_start >= phrase_end {
        return false;
    }
    let head = &tokens[phrase_start];
    // Proper names are never dropped by the predicate rule: "E Milano" is a
    // conjunct name, not the imperative "e mila..." reading.
    if super::starts_uppercase(&head.text) {
        return false;
    }
    // When the UDPipe model is installed its tag is authoritative.
    if let Some(upos) = head.upos.as_deref() {
        return matches!(upos, "VERB" | "AUX");
    }
    // "si trova", "si affaccia", "ci sono": clitic + finite verb is
    // predicative in every reading a noun could take.
    if phrase_end == phrase_start + 2 {
        let first = head.text.to_lowercase();
        let second = tokens[phrase_start + 1].text.to_lowercase();
        if CLITICS.contains(&first.as_str()) && has_verb_morphology(&second) {
            return true;
        }
    }
    if !has_verb_morphology(&head.text.to_lowercase()) {
        return false;
    }
    // A determiner, preposition or possessive immediately before the phrase
    // forces the nominal reading ("la guida", "il salto", "della sorge" is
    // never a predicate frame).
    if let Some(previous) = previous_token(tokens, phrase_start) {
        let previous = previous.text.to_lowercase();
        if NOMINAL_INTRODUCERS.contains(&previous.as_str()) {
            return false;
        }
    }
    let left_is_verbal = previous_token(tokens, phrase_start).is_some_and(|previous| {
        let lowercased = previous.text.to_lowercase();
        CLAUSE_LINKERS.contains(&lowercased.as_str())
            // A capitalized subject immediately before the phrase is the
            // subject-predicate slot ("Torino ospita la Mole").
            || super::starts_uppercase(&previous.text)
    });
    if !left_is_verbal {
        return false;
    }
    // The right slot must introduce a complement a verb takes: an object
    // determiner/clitic, a verbal adverb or a locative complement. End of
    // text and genitive complements stay ambiguous, so they never drop a
    // candidate ("... e vino." keeps "vino").
    match next_token(tokens, phrase_end) {
        Some(next) => {
            let next = next.text.to_lowercase();
            VERBAL_CONTINUATIONS.contains(&next.as_str())
                || VERBAL_CONTINUATION_PREFIXES
                    .iter()
                    .any(|prefix| next.starts_with(prefix))
        }
        None => false,
    }
}

fn is_italian(language: &str) -> bool {
    matches!(language.split(['-', '_']).next(), Some("it"))
}

fn previous_token(tokens: &[Token], phrase_start: usize) -> Option<&Token> {
    phrase_start.checked_sub(1).map(|index| &tokens[index])
}

fn next_token(tokens: &[Token], phrase_end: usize) -> Option<&Token> {
    tokens.get(phrase_end)
}

/// has_verb_morphology reports whether the lowercased word is compatible
/// with an Italian verb inflection. Single-character endings "a", "e", "i"
/// are compatible (sorge, ospita, guarda); the "o" ending is NOT, because
/// every Italian noun ends in "o" and the ambiguity is resolved by the
/// surrounding slots, not by erasing the word ("vino", "sindaco").
fn has_verb_morphology(word: &str) -> bool {
    if NOUN_ENDINGS.iter().any(|ending| word.ends_with(ending)) {
        return false;
    }
    if VERB_ENDINGS.iter().any(|ending| word.ends_with(ending)) {
        return true;
    }
    matches!(word.chars().last(), Some('a' | 'e' | 'i'))
}

#[cfg(test)]
#[path = "verb_tests.rs"]
mod tests;
