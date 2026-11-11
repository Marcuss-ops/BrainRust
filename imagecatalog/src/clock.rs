//! UTC timestamp parsing and the existing candidate freshness policy.
//!
//! The Go owner defines `CandidateFreshAfter = 7 * 24h`: a row seen exactly
//! seven days ago is still fresh, one second later is stale. Broken and
//! retired states are preserved regardless of age, and a row without a
//! timestamp keeps its stored state instead of inventing an age.

/// Seven days, matching `entitycatalog.CandidateFreshAfter`.
pub const FRESH_AFTER_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Parses the UTC timestamps SQLite and the catalog use:
/// `YYYY-MM-DD`, `YYYY-MM-DD HH:MM:SS`, `YYYY-MM-DDTHH:MM:SS`,
/// with optional fractional seconds and a `Z` / `+00:00` suffix.
///
/// Non-UTC offsets are rejected rather than silently shifted: the catalog
/// stores UTC and a wrong shift would change freshness.
pub fn parse_utc_timestamp(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.len() < 10 {
        return None;
    }
    let (date, rest) = value.split_at(10);
    let mut date_parts = date.split('-');
    let year = parse_i64(date_parts.next()?, 4)?;
    let month = parse_i64(date_parts.next()?, 2)?;
    let day = parse_i64(date_parts.next()?, 2)?;
    if date_parts.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
    {
        return None;
    }

    let rest = rest.trim_start_matches(['T', ' ']);
    if rest.is_empty() {
        return days_from_civil(year, month, day).checked_mul(86_400);
    }

    let (time, zone) = split_time_zone(rest)?;
    if !matches!(
        zone,
        "" | "Z" | "z" | "+00:00" | "+0000" | "-00:00" | "-0000"
    ) {
        return None;
    }
    let (time, fraction) = match time.split_once('.') {
        Some((time, fraction)) => (time, Some(fraction)),
        None => (time, None),
    };
    let mut time_parts = time.split(':');
    let hour = parse_i64(time_parts.next()?, 2)?;
    let minute = parse_i64(time_parts.next()?, 2)?;
    let second = parse_i64(time_parts.next()?, 2)?;
    if time_parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }

    let days = days_from_civil(year, month, day);
    days.checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)
}

/// Applies the existing seven-day state policy to one candidate.
pub fn classify_candidate_state(now: i64, stored_status: &str, last_seen_at: &str) -> String {
    let stored = stored_status.trim().to_lowercase();
    match stored.as_str() {
        "broken" | "retired" => stored,
        "fresh" | "stale" => match parse_utc_timestamp(last_seen_at) {
            Some(last_seen) if now - last_seen > FRESH_AFTER_SECONDS => "stale".to_string(),
            Some(_) => "fresh".to_string(),
            None => stored,
        },
        // `active` is the only legacy status that maps to fresh. Anything
        // else is unrecognized and must fail closed instead of becoming a
        // selectable fresh candidate.
        "active" => "fresh".to_string(),
        _ => "unknown".to_string(),
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn split_time_zone(time: &str) -> Option<(&str, &str)> {
    if let Some(index) = time.find(['Z', 'z', '+', '-']) {
        Some((&time[..index], &time[index..]))
    } else {
        Some((time, ""))
    }
}

fn parse_i64(value: &str, digits: usize) -> Option<i64> {
    if value.len() != digits || !value.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// Howard Hinnant's civil-date algorithm: days since 1970-01-01.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_catalog_timestamp_shapes() {
        assert_eq!(
            parse_utc_timestamp("2026-11-10T00:00:00Z"),
            Some(1_794_268_800)
        );
        assert_eq!(
            parse_utc_timestamp("2026-11-10 00:00:00"),
            Some(1_794_268_800)
        );
        assert_eq!(
            parse_utc_timestamp("2026-11-10T00:00:00.250+00:00"),
            Some(1_794_268_800)
        );
        assert_eq!(parse_utc_timestamp("2026-11-10"), Some(1_794_268_800));
        assert_eq!(parse_utc_timestamp("2026-11-10T02:00:00+02:00"), None);
        assert_eq!(parse_utc_timestamp("not a date"), None);
        assert!(parse_utc_timestamp("2024-02-29").is_some());
        assert!(parse_utc_timestamp("2025-02-29").is_none());
        assert!(parse_utc_timestamp("2026-04-31").is_none());
        assert!(parse_utc_timestamp("2026-02-30").is_none());
    }

    #[test]
    fn exactly_seven_days_is_still_fresh_and_one_second_later_is_stale() {
        let now = parse_utc_timestamp("2026-11-10T00:00:00Z").unwrap();
        let boundary = "2026-11-03T00:00:00Z";
        assert_eq!(classify_candidate_state(now, "fresh", boundary), "fresh");
        assert_eq!(classify_candidate_state(now, "stale", boundary), "fresh");
        assert_eq!(
            classify_candidate_state(now, "fresh", "2026-11-02T23:59:59Z"),
            "stale"
        );
    }

    #[test]
    fn explicit_broken_and_retired_states_are_never_aged_away() {
        let now = parse_utc_timestamp("2026-11-10T00:00:00Z").unwrap();
        assert_eq!(classify_candidate_state(now, "broken", ""), "broken");
        assert_eq!(classify_candidate_state(now, "retired", ""), "retired");
    }

    #[test]
    fn a_missing_timestamp_keeps_the_stored_state() {
        let now = parse_utc_timestamp("2026-11-10T00:00:00Z").unwrap();
        assert_eq!(classify_candidate_state(now, "stale", ""), "stale");
        assert_eq!(classify_candidate_state(now, "active", ""), "fresh");
        assert_eq!(classify_candidate_state(now, "", ""), "unknown");
        assert_eq!(classify_candidate_state(now, "pending", ""), "unknown");
    }
}
