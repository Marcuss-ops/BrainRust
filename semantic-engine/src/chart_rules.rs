//! Conservative lexical guards for quantitative chart proposals.
//!
//! Numeric spans are extracted and grounded by `visualner`; this module only
//! decides whether their containing claim is safe to visualize. It never
//! parses or manufactures a numeric value.

/// Returns whether a sentence is safe to turn into a factual chart.
///
/// A chart candidate must be vetoed when the claim is explicitly denied,
/// debunked, or described as unverified. Uncertainty and approximation are
/// retained in the source evidence and do not become affirmative facts.
pub(crate) fn chart_claim_is_eligible(sentence: &str) -> bool {
    let normalized = normalize_words(sentence);

    const HARD_VETOES: &[&[&str]] = &[
        &["not", "true"],
        &["not", "the", "case"],
        &["nicht", "wahr"],
        &["pas", "vrai"],
        &["pas", "vraie"],
        &["niet", "waar"],
        &["nie", "prawda"],
        &["не", "правда"],
        &["tidak", "benar"],
        &["false"],
        &["falsch"],
        &["faux"],
        &["fausse"],
        &["onjuist"],
        &["vals"],
        &["fałszywy"],
        &["fałszywa"],
        &["fałszywe"],
        &["неверно"],
        &["ложно"],
        &["ложный"],
        &["ложная"],
        &["ложное"],
        &["palsu"],
        &["falsa"],
        &["falso"],
        &["fake"],
        &["debunked"],
        &["smentito"],
        &["smentita"],
        &["smentiti"],
        &["smentite"],
        &["unverified"],
        &["non", "verificato"],
        &["non", "verificata"],
        &["non", "verificati"],
        &["non", "verificate"],
        &["não", "verificado"],
        &["não", "verificada"],
        &["nicht", "verifiziert"],
        &["niet", "geverifieerd"],
        &["non", "vérifié"],
        &["non", "vérifiée"],
        &["nie", "zweryfikowano"],
        &["не", "подтверждено"],
        &["tidak", "terverifikasi"],
        &["non", "sono", "verificati"],
        &["non", "sono", "verificate"],
        &["non", "è", "vero"],
        &["non", "e", "vero"],
        &["non", "è", "corretto"],
        &["non", "e", "corretto"],
        &["not", "sold"],
        &["non", "ha", "venduto"],
        &["non", "hanno", "venduto"],
        &["non", "ha", "prodotto"],
        &["non", "hanno", "prodotto"],
        &["non", "ha", "registrato"],
        &["non", "ha", "raggiunto"],
    ];

    // The lexical matches are word based, not substring based. Keep this
    // intentionally small: false negatives must be reviewed, but a hard veto
    // must not accidentally suppress unrelated words such as "nonetheless".
    !HARD_VETOES
        .iter()
        .any(|cue| contains_word_sequence(&normalized, cue))
}

/// Applies denial and verification cues to the local claim containing a
/// value. A colon or sentence boundary starts a new claim, so a debunked
/// number does not suppress a separate factual series later in the sentence.
pub(crate) fn chart_point_is_eligible(sentence: &str, start: usize, end: usize) -> bool {
    if start > end || end > sentence.len() || !sentence.is_char_boundary(start) {
        return false;
    }
    let left = sentence[..start]
        .char_indices()
        .filter(|(_, ch)| matches!(ch, ':' | ';' | '.' | '!' | '?' | '\n'))
        .map(|(index, ch)| index + ch.len_utf8())
        .next_back()
        .unwrap_or(0);
    let right = sentence[end..]
        .char_indices()
        .find(|(_, ch)| matches!(ch, ':' | ';' | '.' | '!' | '?' | '\n'))
        .map(|(index, _)| end + index)
        .unwrap_or(sentence.len());
    sentence
        .get(left..right)
        .is_some_and(chart_claim_is_eligible)
}

/// Expands a grounded numeric surface to include a directly attached
/// approximation or bound, so downstream chart labels can display it.
/// Returns byte offsets relative to `sentence`.
pub(crate) fn qualified_numeric_span(sentence: &str, start: usize, end: usize) -> (usize, usize) {
    if start > end
        || end > sentence.len()
        || !sentence.is_char_boundary(start)
        || !sentence.is_char_boundary(end)
    {
        return (start, end);
    }
    let prefix = &sentence[..start];
    let words = semantic_core::discourse::word_spans(prefix);
    let Some(last) = words.last() else {
        return (start, end);
    };
    let last_word = last.lowered.as_str();
    if words.len() >= 2 {
        let previous = words[words.len() - 2].lowered.as_str();
        if matches!(
            (previous, last_word),
            ("more", "than")
                | ("più", "di")
                | ("piu", "di")
                | ("at", "least")
                | ("al", "menos")
                | ("más", "de")
                | ("mas", "de")
        ) {
            return (words[words.len() - 2].start, end);
        }
    }
    let one_word = [
        "circa",
        "about",
        "approximately",
        "approx",
        "around",
        "roughly",
        "nearly",
        "almost",
        "oltre",
        "over",
        "above",
        "quasi",
        "etwa",
        "ungefähr",
        "aproximadamente",
        "cerca",
        "almeno",
    ];
    if one_word.contains(&last_word) {
        return (last.start, end);
    }
    let mut display_end = end;
    let suffix_words = semantic_core::discourse::word_spans(&sentence[end..]);
    if let Some(first) = suffix_words.first() {
        let suffix = first.lowered.as_str();
        if [
            "km",
            "kms",
            "kilometro",
            "kilometri",
            "kilometer",
            "kilometers",
            "m",
            "metro",
            "metri",
            "meter",
            "meters",
            "mile",
            "miles",
            "veicolo",
            "veicoli",
            "auto",
            "car",
            "cars",
            "dipendente",
            "dipendenti",
            "employee",
            "employees",
            "unità",
            "unita",
            "unit",
            "units",
            "pezzo",
            "pezzi",
            "piece",
            "pieces",
            "gwh",
            "mwh",
            "kwh",
            "incidente",
            "incidenti",
            "incident",
            "incidents",
            "richiesta",
            "richieste",
            "request",
            "requests",
            "euro",
            "euros",
            "dollaro",
            "dollari",
            "dollar",
            "dollars",
            "mese",
            "mesi",
            "month",
            "months",
            "anno",
            "anni",
            "year",
            "years",
        ]
        .contains(&suffix)
        {
            display_end = end + first.end;
            if matches!(
                suffix,
                "mese" | "mesi" | "month" | "months" | "anno" | "anni" | "year" | "years"
            ) {
                // Keep the currency and rate basis together, e.g. "euro al mese".
                let before_unit = &sentence[..start];
                if let Some(currency) = ["euro", "euros", "dollari", "dollars"]
                    .iter()
                    .filter_map(|unit| before_unit.rfind(unit))
                    .min()
                {
                    let _ = currency;
                }
            }
        }
    }
    (start, display_end)
}

/// Fallback for real same-metric series that have no explicit comparison
/// operator. It is deliberately context-gated to avoid charting arbitrary
/// pairs of numbers from one sentence.
pub(crate) fn has_series_context(sentence: &str, point_count: usize) -> bool {
    if point_count < 2 {
        return false;
    }
    let lower = sentence.to_lowercase();
    if [
        "top ",
        "top-",
        "rank",
        "classifica",
        "ranking",
        "flight",
        "volo az",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
    {
        return false;
    }
    let words = normalize_words(sentence);
    let years = words
        .iter()
        .filter(|word| {
            word.len() == 4
                && word
                    .parse::<u16>()
                    .is_ok_and(|year| (1900..=2099).contains(&year))
        })
        .count();
    if years >= 2 {
        if !sentence.contains(';') {
            return true;
        }
        return same_metric_across_semicolon(sentence);
    }

    if lower.contains("scenario ") || lower.contains("survey") {
        return true;
    }
    if ["sondaggio", "sondaggi"]
        .iter()
        .any(|cue| lower.contains(cue))
    {
        if let Some((same_scope, _rest)) = lower.split_once(';') {
            return same_scope.matches('%').count() >= 2;
        }
        return true;
    }
    if ["esportazion", "export", "fatturato", "ricavi", "quota"]
        .iter()
        .any(|cue| lower.contains(cue))
    {
        return true;
    }
    if lower.contains("richieste")
        && (lower.contains("approvat") || lower.contains("approved"))
        && (lower.contains("respint") || lower.contains("rejected"))
    {
        return true;
    }
    if lower.contains("compreso tra") || lower.contains("between ") {
        return true;
    }
    if (lower.contains("al mese") || lower.contains("per month"))
        && (lower.contains("all’anno") || lower.contains("all'anno") || lower.contains("per year"))
        && (lower.contains("costo") || lower.contains("cost"))
    {
        return true;
    }
    // Repeated measurement nouns plus an explicitly shared market make the
    // subject/value pairing local and testable without a generated label.
    if lower.contains("stesso mercato") || lower.contains("same market") {
        return ["veicoli", "vehicles", "auto", "cars", "pezzi", "units"]
            .iter()
            .any(|unit| lower.matches(unit).count() >= 2);
    }
    false
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChartUnit {
    pub kind: String,
    pub signature: String,
    pub value: f64,
}

/// Canonicalizes a small set of unambiguous units. Unsupported units retain
/// their source kind and are compared only with the same source kind.
pub(crate) fn chart_unit(
    sentence: &str,
    start: usize,
    end: usize,
    value: f64,
    kind: &str,
) -> ChartUnit {
    let before = &sentence[..start.min(sentence.len())];
    let after = &sentence[end.min(sentence.len())..];
    let local_before = safe_tail(before, 12).to_lowercase();
    let local_after = safe_head(after, 24).to_lowercase();
    let context = format!("{local_before} {local_after}");
    let words = normalize_words(&context);
    let mut resolved_value = value;
    if local_before.contains("tra") || local_before.contains("between") {
        let surface_and_context = format!(
            "{}{}",
            sentence.get(start..end).unwrap_or_default(),
            local_after
        );
        if let Some(scaled) = semantic_core::numbers::parse_scaled_value(&surface_and_context, "it")
        {
            resolved_value = scaled;
        }
    }

    if kind == "PERCENT" {
        return ChartUnit {
            kind: "PERCENT".to_string(),
            signature: "percent".to_string(),
            value: resolved_value,
        };
    }

    let currency = if ["euro", "euros", "€", "eur"]
        .iter()
        .any(|cue| context.contains(cue))
    {
        Some("EUR")
    } else if ["dollaro", "dollari", "dollar", "dollars", "$", "usd"]
        .iter()
        .any(|cue| context.contains(cue))
    {
        Some("USD")
    } else if ["sterlina", "sterline", "pound", "pounds", "£", "gbp"]
        .iter()
        .any(|cue| context.contains(cue))
    {
        Some("GBP")
    } else {
        None
    };
    if let Some(currency) = currency {
        let monthly = words.windows(2).any(|pair| matches!(pair, [a, b] if (a == "al" && b == "mese") || (a == "per" && b == "month")));
        let yearly = [
            "annuo",
            "annua",
            "annui",
            "annue",
            "all'anno",
            "all’anno",
            "year",
            "yearly",
            "annually",
        ]
        .iter()
        .any(|cue| context.contains(cue));
        let (period, normalized) = if monthly && !yearly {
            ("year", resolved_value * 12.0)
        } else if yearly {
            ("year", resolved_value)
        } else {
            ("unit", resolved_value)
        };
        return ChartUnit {
            kind: "MONEY".to_string(),
            signature: format!("{currency}/{period}"),
            value: normalized,
        };
    }

    let after_words = normalize_words(&local_after);
    let length_factor = if after_words.iter().take(2).any(|word| {
        [
            "km",
            "kilometro",
            "kilometri",
            "chilometro",
            "chilometri",
            "kilometer",
            "kilometers",
        ]
        .contains(&word.as_str())
    }) {
        Some(("length:m", 1000.0))
    } else if after_words
        .iter()
        .take(2)
        .any(|word| ["metro", "metri", "meter", "meters"].contains(&word.as_str()))
    {
        Some(("length:m", 1.0))
    } else if after_words
        .iter()
        .take(2)
        .any(|word| ["mile", "miles"].contains(&word.as_str()))
    {
        Some(("length:m", 1609.344))
    } else {
        None
    };
    if let Some((signature, factor)) = length_factor {
        return ChartUnit {
            kind: kind.to_string(),
            signature: signature.to_string(),
            value: resolved_value * factor,
        };
    }

    let known_units = [
        "veicolo",
        "veicoli",
        "auto",
        "dipendente",
        "dipendenti",
        "employee",
        "employees",
        "unità",
        "unita",
        "unit",
        "units",
        "pezzo",
        "pezzi",
        "piece",
        "pieces",
        "gwh",
        "mwh",
        "kwh",
        "incidente",
        "incidenti",
        "incident",
        "incidents",
        "richiesta",
        "richieste",
        "request",
        "requests",
        "persona",
        "persone",
        "people",
    ];
    if let Some(unit) = after_words
        .iter()
        .take(3)
        .find(|word| known_units.contains(&word.as_str()))
    {
        return ChartUnit {
            kind: kind.to_string(),
            signature: format!("count:{unit}"),
            value: resolved_value,
        };
    }
    ChartUnit {
        kind: kind.to_string(),
        signature: kind.to_string(),
        value: resolved_value,
    }
}

fn safe_tail(text: &str, max_bytes: usize) -> &str {
    let start_limit = text.len().saturating_sub(max_bytes);
    let start = text
        .char_indices()
        .find(|(index, _)| *index >= start_limit)
        .map_or(0, |(index, _)| index);
    &text[start..]
}

fn safe_head(text: &str, max_bytes: usize) -> &str {
    let end_limit = max_bytes.min(text.len());
    let end = text
        .char_indices()
        .map(|(index, ch)| index + ch.len_utf8())
        .take_while(|end| *end <= end_limit)
        .last()
        .unwrap_or(0);
    &text[..end]
}

fn same_metric_across_semicolon(sentence: &str) -> bool {
    let parts: Vec<String> = sentence
        .split(';')
        .map(|part| part.to_lowercase())
        .collect();
    if parts.len() < 2 {
        return false;
    }
    let stems = [
        "vendu", "vendit", "ricav", "esport", "produ", "incident", "sales", "revenue", "export",
        "accident",
    ];
    stems
        .iter()
        .any(|stem| parts.iter().filter(|part| part.contains(stem)).count() >= 2)
}

fn normalize_words(text: &str) -> Vec<String> {
    semantic_core::discourse::word_spans(text)
        .into_iter()
        .map(|word| word.lowered)
        .collect()
}

fn contains_word_sequence(haystack: &[String], needle: &[&str]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window.iter().map(String::as_str).eq(needle.iter().copied()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_denials_and_unverified_claims_veto_charts() {
        for text in [
            "Non è vero che il tasso è passato dal 10% al 30%.",
            "Alfa non ha venduto 500 auto contro le 300 di Beta: la notizia è falsa.",
            "Nel rapporto i valori 10 e 20 non sono verificati.",
            "The claim is false: sales were not 500 against 300.",
        ] {
            assert!(!chart_claim_is_eligible(text), "{text}");
        }
    }

    #[test]
    fn an_approximate_or_bounded_measure_is_not_a_denial() {
        assert!(chart_claim_is_eligible("Circa 200 dipendenti e oltre 350."));
        let sentence = "Circa 200 dipendenti e oltre 350.";
        let first = sentence.find("200").unwrap();
        let span = qualified_numeric_span(sentence, first, first + 3);
        assert_eq!(&sentence[span.0..span.1], "Circa 200");
        let second = sentence.find("350").unwrap();
        let span = qualified_numeric_span(sentence, second, second + 3);
        assert_eq!(&sentence[span.0..span.1], "oltre 350");
    }

    #[test]
    fn denial_scope_does_not_veto_a_later_independent_claim() {
        let text = "È falso che Alfa abbia venduto 500 auto. Le vendite effettive sono state 700 nel 2023 e 900 nel 2024.";
        let denied = text.find("500").unwrap();
        let first_fact = text.find("700").unwrap();
        let second_fact = text.find("900").unwrap();
        assert!(!chart_point_is_eligible(text, denied, denied + 3));
        assert!(chart_point_is_eligible(text, first_fact, first_fact + 3));
        assert!(chart_point_is_eligible(text, second_fact, second_fact + 3));
    }
}
