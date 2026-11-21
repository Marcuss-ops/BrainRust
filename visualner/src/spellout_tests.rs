use super::*;

// Assistant draft pending human review, as required by the semantic
// expansion roadmap. Expected values are independent of the scanner.
const VALUES: [i64; 21] = [
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 20, 21, 30, 42, 100, 1200, 2000, 1_000_000,
];

const FRACTIONS: &[(&str, &str)] = &[
    ("it", "un terzo"),
    ("en", "one third"),
    ("es", "un tercio"),
    ("pt", "um terço"),
    ("fr", "un tiers"),
    ("de", "ein Drittel"),
    ("nl", "een derde"),
    ("pl", "jedna trzecia"),
    ("ru", "одна треть"),
    ("tr", "üçte biri"),
    ("id", "sepertiga"),
];

const APPROXIMATORS: &[(&str, &str)] = &[
    ("it", "circa"),
    ("en", "about"),
    ("es", "alrededor de"),
    ("pt", "cerca de"),
    ("fr", "environ"),
    ("de", "rund"),
    ("nl", "ongeveer"),
    ("pl", "około"),
    ("ru", "около"),
    ("tr", "yaklaşık"),
    ("id", "sekitar"),
];

const UNITS: &[(&str, &str)] = &[
    ("it", "persone"),
    ("en", "people"),
    ("es", "personas"),
    ("pt", "pessoas"),
    ("fr", "personnes"),
    ("de", "Menschen"),
    ("nl", "mensen"),
    ("pl", "osób"),
    ("ru", "человек"),
    ("tr", "kişi"),
    ("id", "orang"),
];

const NEGATIVES: &[(&str, &[&str; 20])] = &[
    (
        "it",
        &[
            "il",
            "lo",
            "la",
            "i",
            "gli",
            "le",
            "un",
            "una",
            "di",
            "da",
            "in",
            "con",
            "su",
            "per",
            "tra",
            "il gatto",
            "la casa",
            "mille grazie",
            "grazie mille",
            "mille scuse",
        ],
    ),
    (
        "en",
        &[
            "the",
            "a",
            "an",
            "of",
            "to",
            "in",
            "with",
            "on",
            "for",
            "by",
            "from",
            "at",
            "it",
            "they",
            "we",
            "the cat",
            "a house",
            "thanks a million",
            "a million thanks",
            "one in a million",
        ],
    ),
    (
        "es",
        &[
            "el",
            "la",
            "los",
            "las",
            "un",
            "una",
            "de",
            "a",
            "en",
            "con",
            "por",
            "para",
            "sin",
            "sobre",
            "entre",
            "el gato",
            "la casa",
            "mil gracias",
            "gracias mil",
            "por favor",
        ],
    ),
    (
        "pt",
        &[
            "o",
            "a",
            "os",
            "as",
            "um",
            "uma",
            "de",
            "do",
            "da",
            "em",
            "com",
            "por",
            "para",
            "sem",
            "entre",
            "o gato",
            "a casa",
            "mil obrigados",
            "obrigado mil vezes",
            "por favor",
        ],
    ),
    (
        "fr",
        &[
            "le",
            "la",
            "les",
            "un",
            "une",
            "de",
            "du",
            "des",
            "en",
            "avec",
            "par",
            "pour",
            "sans",
            "sur",
            "entre",
            "le chat",
            "la maison",
            "mille mercis",
            "merci mille fois",
            "s'il vous plaît",
        ],
    ),
    (
        "de",
        &[
            "der",
            "die",
            "das",
            "den",
            "dem",
            "des",
            "ein",
            "eine",
            "von",
            "zu",
            "in",
            "mit",
            "auf",
            "für",
            "ohne",
            "die Katze",
            "das Haus",
            "tausend dank",
            "vielen dank",
            "bitte sehr",
        ],
    ),
    (
        "nl",
        &[
            "de",
            "het",
            "een",
            "van",
            "naar",
            "in",
            "met",
            "op",
            "voor",
            "zonder",
            "tussen",
            "over",
            "bij",
            "aan",
            "door",
            "de kat",
            "het huis",
            "duizendmaal dank",
            "hartelijk dank",
            "alsjeblieft",
        ],
    ),
    (
        "pl",
        &[
            "ten",
            "ta",
            "to",
            "te",
            "ci",
            "te",
            "jeden",
            "jedna",
            "z",
            "do",
            "w",
            "na",
            "pod",
            "przez",
            "bez",
            "kot",
            "dom",
            "tysiąc podziękowań",
            "stokrotne dzięki",
            "proszę",
        ],
    ),
    (
        "ru",
        &[
            "этот",
            "эта",
            "это",
            "эти",
            "один",
            "одна",
            "из",
            "к",
            "в",
            "с",
            "на",
            "для",
            "без",
            "между",
            "через",
            "кот",
            "дом",
            "тысяча благодарностей",
            "спасибо тысячу раз",
            "пожалуйста",
        ],
    ),
    (
        "tr",
        &[
            "bu",
            "şu",
            "o",
            "bir",
            "ve",
            "ile",
            "için",
            "den",
            "dan",
            "de",
            "da",
            "üzerinde",
            "altında",
            "arasında",
            "gibi",
            "kedi",
            "ev",
            "bin teşekkür",
            "binlerce teşekkür",
            "lütfen",
        ],
    ),
    (
        "id",
        &[
            "ini",
            "itu",
            "sebuah",
            "satu",
            "dan",
            "dengan",
            "untuk",
            "dari",
            "di",
            "pada",
            "atas",
            "bawah",
            "antara",
            "seperti",
            "tanpa",
            "kucing",
            "rumah",
            "seribu terima kasih",
            "terima kasih banyak",
            "tolong",
        ],
    ),
];

#[test]
fn twenty_positive_cases_per_language() {
    assert_eq!(UNITS.len(), 11);
    for (language, unit) in UNITS {
        let formatters = formatters(language).expect("locale");
        let spellout = formatters.spellout.as_ref().expect("spellout data");
        for value in VALUES {
            let spelled = spellout.format_i64(value).expect("format");
            let text = format!("{spelled} {unit}");
            let found = candidates(&text, language).expect("scan");
            assert!(
                found
                    .iter()
                    .any(|c| (c.value - value as f64).abs() < 1e-9 && c.start == 0),
                "{language} {value} {text:?}: {found:?}"
            );
        }
    }
}

#[test]
fn fractions_in_every_language() {
    for (language, phrase) in FRACTIONS {
        let found = candidates(phrase, language).expect("scan");
        assert!(
            found
                .iter()
                .any(|c| { c.entity_type == "FRACTION" && (c.value - 1.0 / 3.0).abs() < 1e-9 }),
            "{language} {phrase:?}: {found:?}"
        );
    }
}

#[test]
fn approximators_in_every_language() {
    for ((language, approximator), (_, unit)) in APPROXIMATORS.iter().zip(UNITS) {
        let formatters = formatters(language).expect("locale");
        let spellout = formatters.spellout.as_ref().expect("spellout data");
        let spelled = spellout.format_i64(5000).expect("format");
        let text = format!("{approximator} {spelled} {unit}");
        let found = candidates(&text, language).expect("scan");
        assert!(
            found.iter().any(|c| {
                (c.value - 5000.0).abs() < 1e-9 && c.start == 0 && c.end > approximator.len()
            }),
            "{language} {text:?}: {found:?}"
        );
    }
}

#[test]
fn twenty_negative_cases_per_language() {
    assert_eq!(NEGATIVES.len(), 11);
    for (language, phrases) in NEGATIVES {
        for phrase in phrases.iter() {
            let found = candidates(phrase, language).expect("scan");
            assert!(found.is_empty(), "{language} {phrase:?}: {found:?}");
        }
    }
}
