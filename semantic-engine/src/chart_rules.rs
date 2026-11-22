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

    // The lexical matches are word based, not substring based. Keep this
    // intentionally small: false negatives must be reviewed, but a hard veto
    // must not accidentally suppress unrelated words such as "nonetheless".
    !semantic_core::chart_cues::global()
        .hard_vetoes
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
    if let Some(last) = words.last() {
        let last_word = last.lowered.as_str();
        let cues = semantic_core::chart_cues::global();
        if words.len() >= 2 {
            let previous = words[words.len() - 2].lowered.as_str();
            if cues
                .approximator_pairs
                .iter()
                .any(|pair| pair.len() == 2 && pair[0] == previous && pair[1] == last_word)
            {
                return (words[words.len() - 2].start, end);
            }
        }
        if cues.approximators.iter().any(|cue| cue == last_word) {
            return (last.start, end);
        }
    }
    let mut display_start = start;
    let mut display_end = end;
    let suffix_words = semantic_core::discourse::word_spans(&sentence[end..]);
    let prefix_currency = prefix
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '€' | '$' | '£'));
    if let Some(first) = suffix_words.first() {
        let suffix = first.lowered.as_str();
        if semantic_core::chart_cues::global()
            .display_unit_cues
            .iter()
            .any(|cue| cue == suffix)
        
        {
            display_end = end + first.end;
        }
    }
    let suffix_currency = suffix_words.first().is_some_and(|word| {
        semantic_core::chart_cues::global()
            .currency_cues
            .values()
            .flatten()
            .any(|cue| cue == &word.lowered)
    });
    if prefix_currency || suffix_currency {
        if prefix_currency {
            display_start = prefix
                .char_indices()
                .rev()
                .find(|(_, ch)| !ch.is_whitespace())
                .map_or(start, |(index, _)| index);
        }
        let remainder = sentence[display_end..].trim_start();
        let leading_space = sentence[display_end..].len() - remainder.len();
        let lower = remainder.to_lowercase();
        let rate_cues = &semantic_core::chart_cues::global().rate_cues;
        if let Some(cue) = rate_cues.iter().find(|cue| lower.starts_with(cue.as_str())) {
            display_end += leading_space + cue.len();
        }
    }
    (display_start, display_end)
}

/// Fallback for real same-metric series that have no explicit comparison
/// operator. It is deliberately context-gated to avoid charting arbitrary
/// pairs of numbers from one sentence.
pub(crate) fn has_series_context(sentence: &str, point_count: usize, language: &str) -> bool {
    if point_count < 2 {
        return false;
    }
    let lower = sentence.to_lowercase();
    let cues = semantic_core::chart_cues::global();
    let language_cues = semantic_core::chart_cues::language_cues(language);
    if cues
        .rank_veto_cues
        .iter()
        .any(|cue| lower.contains(cue.as_str()))
    {
        return false;
    }
    let words = normalize_words(sentence);
    let years = words.iter().filter(|word| is_year_token(word)).count();
    if years >= 2 {
        if !sentence.contains(';') {
            return true;
        }
        return same_metric_across_semicolon(sentence, language);
    }

    if language_cues
        .survey_cues
        .iter()
        .any(|cue| lower.contains(cue.as_str()))
    {
        if let Some((same_scope, _rest)) = lower.split_once(';') {
            return same_scope.matches('%').count() >= 2;
        }
        return true;
    }
    if language_cues
        .series_context_cues
        .iter()
        .any(|cue| lower.contains(cue.as_str()))
    {
        return true;
    }
    let language = language.split(['-', '_']).next().unwrap_or(language);
    if semantic_core::units::language_units(language).is_some_and(|units| {
        units
            .predicates
            .values()
            .flatten()
            .any(|cue| lower.contains(&cue.to_lowercase()))
    }) {
        return true;
    }
    if cues.paired_series_rules.iter().any(|rule| {
        !rule.any_in_each.is_empty()
            && rule
                .any_in_each
                .iter()
                .all(|group| group.iter().any(|cue| lower.contains(&cue.to_lowercase())))
            && (rule.repeated_units.is_empty()
                || rule.repeated_units.iter().any(|unit| {
                    lower.matches(&unit.to_lowercase()).count() >= rule.min_repeats.max(2)
                }))
    }) {
        return true;
    }
    false
}

fn is_year_token(word: &str) -> bool {
    let year_digits = word.get(..4).unwrap_or_default();
    let Ok(year) = year_digits.parse::<u16>() else {
        return false;
    };
    if !(1900..=2099).contains(&year) {
        return false;
    }
    if word.len() == 4 {
        return true;
    }
    let suffix = &word[4..];
    suffix
        .strip_prefix('\'')
        .or_else(|| suffix.strip_prefix('’'))
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(char::is_alphabetic))
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

    let global = semantic_core::chart_cues::global();
    let currency = global.currency_cues.iter().find_map(|(code, cues)| {
        cues.iter()
            .any(|cue| context.contains(cue.as_str()))
            .then_some(code.as_str())
    });
    if let Some(currency) = currency {
        let monthly = words.windows(2).any(|pair| {
            global
                .monthly_pairs
                .iter()
                .any(|expected| expected.len() == 2 && expected[0] == pair[0] && expected[1] == pair[1])
        });
        let yearly = global
            .yearly_cues
            .iter()
            .any(|cue| context.contains(cue.as_str()));
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
        global
            .length_cues
            .get("km")
            .is_some_and(|cues| cues.contains(word))
    }) {
        Some(("length:m", 1000.0))
    } else if after_words.iter().take(2).any(|word| {
        global
            .length_cues
            .get("m")
            .is_some_and(|cues| cues.contains(word))
    }) {
        Some(("length:m", 1.0))
    } else if after_words.iter().take(2).any(|word| {
        global
            .length_cues
            .get("mile")
            .is_some_and(|cues| cues.contains(word))
    }) {
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

    let known_units = &global.count_unit_cues;
    if let Some(unit) = after_words
        .iter()
        .take(3)
        .find(|word| known_units.contains(word))
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

fn same_metric_across_semicolon(sentence: &str, language: &str) -> bool {
    let parts: Vec<String> = sentence
        .split(';')
        .map(|part| part.to_lowercase())
        .collect();
    if parts.len() < 2 {
        return false;
    }
    // Repeated metric predicates make a temporal comparison explicit even
    // when the sentence uses neither a comparison operator nor a shared noun
    // (for example, "Aurora sold 120 cars; in 2023 it sold 180").
    let stems = &semantic_core::chart_cues::language_cues(language).semicolon_metric_stems;
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

fn contains_word_sequence(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
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

    #[test]
    fn turkish_repeated_sales_predicate_supports_annual_series() {
        let text = "2022'de Aurora 120 otomobil sattı; 2023'te 180 otomobil sattı.";
        assert!(has_series_context(text, 2, "tr"));
    }

    #[test]
    fn chart_measure_text_keeps_currency_and_rate_period() {
        let text = "Il costo annuo di A è di 20 euro al mese e quello di B è 150 euro all’anno.";
        let monthly = text.find("20").unwrap();
        let span = qualified_numeric_span(text, monthly, monthly + 2);
        assert_eq!(&text[span.0..span.1], "20 euro al mese");

        let yearly = text.find("150").unwrap();
        let span = qualified_numeric_span(text, yearly, yearly + 3);
        assert_eq!(&text[span.0..span.1], "150 euro all’anno");

        let symbol = "€20 per month";
        let amount = symbol.find("20").unwrap();
        let span = qualified_numeric_span(symbol, amount, amount + 2);
        assert_eq!(&symbol[span.0..span.1], "€20 per month");
    }
}
