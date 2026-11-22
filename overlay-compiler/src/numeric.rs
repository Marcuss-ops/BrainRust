//! One strict parser for verbatim chart labels.
//!
//! Both the raw projection (`apply_chart_values`) and the planner's chart
//! placement counter answer the same question — *is this token a chartable
//! number?* — and used to carry byte-identical copies. The projection copy and
//! the planner copy could drift independently, so a label admitted by one path
//! was rejected by the other. This module is the single implementation.

/// Parses a verbatim numeric label: digits with at most one decimal separator
/// and one optional leading sign, optionally wrapped in a percentage/currency
/// sign. Number words, embedded digits and unit suffixes are rejected instead
/// of coerced.
pub(crate) fn parse_chart_value(text: &str) -> Option<f64> {
    let mut value = text.trim();
    if value.is_empty() {
        return None;
    }
    value = value.trim_matches(|ch| matches!(ch, '%' | '€' | '$' | '£' | '¥'));
    let compact = value.trim().replace(' ', "");
    if compact.is_empty() {
        return None;
    }
    let numeric = if compact.matches(',').count() == 1 && !compact.contains('.') {
        compact.replace(',', ".")
    } else {
        compact.replace(',', "")
    };
    if numeric.is_empty() || matches!(numeric.as_str(), "-" | "." | "-.") {
        return None;
    }
    let mut digits = 0;
    let mut separator = false;
    for (index, ch) in numeric.char_indices() {
        match ch {
            '0'..='9' => digits += 1,
            '.' if !separator && index > 0 => separator = true,
            '-' if index == 0 => {}
            _ => return None,
        }
    }
    if digits == 0 {
        return None;
    }
    let parsed: f64 = numeric.parse().ok()?;
    (!parsed.is_nan() && !parsed.is_infinite()).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::parse_chart_value;

    #[test]
    fn parses_plain_decimal_comma_and_currency_labels() {
        assert_eq!(parse_chart_value("42"), Some(42.0));
        assert_eq!(parse_chart_value("3,5"), Some(3.5));
        assert_eq!(parse_chart_value("1234,5"), Some(1234.5));
        assert_eq!(parse_chart_value("18%"), Some(18.0));
        assert_eq!(parse_chart_value("€ 1 200"), Some(1200.0));
        assert_eq!(parse_chart_value("-7,25"), Some(-7.25));
    }

    #[test]
    fn rejects_number_words_units_and_embedded_digits() {
        for rejected in [
            "",
            "tre",
            "42 per cento",
            "42px",
            "1.2.3",
            "-",
            ".",
            "-.",
            "1e3",
            "--4",
        ] {
            assert_eq!(parse_chart_value(rejected), None, "{rejected:?}");
        }
    }
}
