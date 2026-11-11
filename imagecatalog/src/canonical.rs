//! Canonical PERSON identity, mirroring
//! `internal/capabilities/entities.CanonicalEntityID`:
//! `person:` + `SafeEntityID(NormalizeName(name))`.
//!
//! The catalog join key is owned by PipelineGen. This module reproduces the
//! same deterministic derivation so a surface extracted from text resolves to
//! the same `person:<slug>` row without a database call or a competing slug
//! algorithm. Only entity *type* PERSON is relevant to the current catalog.

/// Derives `person:<slug>` from a surface name. Returns an empty string when
/// the name has no ASCII alphanumeric token (the Go owner never mints an id
/// for nothing).
pub fn canonical_person_id(name: &str) -> String {
    let slug = safe_entity_id(&normalize_name(strip_person_possessive(name)));
    if slug.is_empty() {
        String::new()
    } else {
        format!("person:{slug}")
    }
}

/// Removes an English possessive suffix, which is grammatical attachment and
/// not part of the person's identity (`Donald Trump's` -> `Donald Trump`).
fn strip_person_possessive(name: &str) -> &str {
    let trimmed = name.trim();
    for suffix in ["'s", "’s"] {
        if trimmed.len() > suffix.len()
            && trimmed.to_lowercase().ends_with(suffix)
            && trimmed.is_char_boundary(trimmed.len() - suffix.len())
        {
            return trimmed[..trimmed.len() - suffix.len()].trim();
        }
    }
    trimmed
}

/// Trimmed, lowercased, internal whitespace collapsed to single spaces.
fn normalize_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase ASCII alphanumerics are kept; every other rune becomes a dash,
/// and leading/trailing dashes are trimmed. This mirrors the Go
/// `SafeEntityID`, including consecutive dashes for punctuation runs.
fn safe_entity_id(name: &str) -> String {
    let mut slug = String::new();
    for character in name.trim().to_lowercase().chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            slug.push(character);
        } else if !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_existing_person_catalog_ids() {
        assert_eq!(
            canonical_person_id("Michael Jordan"),
            "person:michael-jordan"
        );
        assert_eq!(
            canonical_person_id("  Michael   Jordan "),
            "person:michael-jordan"
        );
        assert_eq!(
            canonical_person_id("Floyd Mayweather Jr."),
            "person:floyd-mayweather-jr"
        );
        assert_eq!(
            canonical_person_id("Michael B. Jordan"),
            "person:michael-b--jordan"
        );
        assert_eq!(canonical_person_id("Donald Trump's"), "person:donald-trump");
        assert_eq!(canonical_person_id("   "), "");
        assert_eq!(canonical_person_id("!!!"), "");
    }
}
