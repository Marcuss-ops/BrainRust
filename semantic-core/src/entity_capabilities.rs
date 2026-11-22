//! One table for what an entity kind can do.
//!
//! The same kind list used to be written by hand in five places
//! (`bullets::signal_kind_for_entity_type`, VisualNER's co-mention composition,
//! the identity registry's named kinds, `groups::is_visual_entity` and the
//! contextual image-query filter). Adding an additive type to four of them and
//! forgetting the fifth is exactly how ANIMAL/FOOD/… ended up extracted but
//! never pictured, so the flags now live here and every caller reads them.
//!
//! The families below are deliberately derived from three short lists. A new
//! kind is one entry, never a new `matches!` arm in every crate.

use crate::bullets::SignalKind;

/// Kinds that name a subject a planner can picture and search. This is the
/// original closed set; it has not changed since the taxonomy was introduced.
const NAMED_SUBJECTS: &[&str] = &[
    "PERSON",
    "ORGANIZATION",
    "ORG",
    "LOCATION",
    "FACILITY",
    "BRAND",
    "PRODUCT",
    "WORK",
];

/// Concrete visual subjects that are not named identities: they can be
/// pictured, but they are not searched as names nor promoted to identity
/// cards.
const EXTRA_VISUAL_SUBJECTS: &[&str] = &["OBJECT", "STRUCTURE", "EVENT"];

/// TICKET-43 additive types. They share a visual frame and a bullet signal but
/// are not named subjects: no identity card and no image query.
const ADDITIVE_VISUAL_TYPES: &[&str] = &[
    "ANIMAL",
    "FOOD",
    "VEHICLE",
    "LAW",
    "COURT_CASE",
    "NORP",
    "DISEASE",
    "TECHNOLOGY",
    "ARTWORK",
    "CELESTIAL",
];

/// Kinds whose mention is a measurement, never a subject to picture.
const MEASUREMENT_TYPES: &[&str] = &["NUMBER", "PERCENT", "MONEY", "MEASUREMENT", "DATE"];

/// Kinds that carry a named identity in the identity registry although they
/// are not pictureable subjects (a role token is a name, not a portrait).
const NAMED_KIND_EXTRAS: &[&str] = &["OBJECT", "EVENT", "ROLE"];

/// Named subjects that may additionally drive a contextual image query.
const VISUAL_QUERY_EXTRAS: &[&str] = &["OBJECT", "STRUCTURE"];

/// Media outlets are organizations for identity and image binding, but they
/// carry their own hint so a source-highlight recipe can name them.
const MEDIA_OUTLETS: &[&str] = &["MEDIA_OUTLET"];

/// What one entity kind may be used for. Every flag answers a question a
/// caller already asked; no caller re-implements the membership test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntityCapabilities {
    /// Belongs in a visual frame at all (not a measurement, not a concept).
    pub visual_subject: bool,
    /// Can be pictured and searched by name (identity card / image query).
    pub named_subject: bool,
    /// Can share an editorial co-mention frame with another subject.
    pub co_mention_subject: bool,
    /// Carries a named identity in the identity registry.
    pub named_kind: bool,
    /// May drive a contextual visual query.
    pub visual_query: bool,
    /// Signal a bullet sentence is built from, when the kind carries one.
    pub bullet_signal: Option<SignalKind>,
    /// A news outlet or publication, i.e. a citable source.
    pub media_outlet: bool,
}

/// Resolves capabilities for one extractor spelling.
///
/// The spelling is trimmed and ASCII-uppercased so a caller cannot silently
/// lose a kind because it forwarded `" org "` or `"Org"`. Unknown kinds are
/// capabilities-free, which keeps the old fail-closed default.
pub fn entity_capabilities(kind: &str) -> EntityCapabilities {
    let kind = kind.trim().to_ascii_uppercase();
    let in_list = |list: &[&str]| list.contains(&kind.as_str());
    let named = in_list(NAMED_SUBJECTS);
    let additive = in_list(ADDITIVE_VISUAL_TYPES);
    let outlet = in_list(MEDIA_OUTLETS);
    let visual_subject = named || additive || outlet || in_list(EXTRA_VISUAL_SUBJECTS);
    EntityCapabilities {
        visual_subject,
        named_subject: named || outlet,
        co_mention_subject: visual_subject,
        named_kind: named || additive || outlet || in_list(NAMED_KIND_EXTRAS),
        visual_query: named || outlet || in_list(VISUAL_QUERY_EXTRAS),
        bullet_signal: if visual_subject {
            Some(SignalKind::Visual)
        } else if in_list(MEASUREMENT_TYPES) {
            Some(SignalKind::Measurement)
        } else {
            None
        },
        media_outlet: outlet,
    }
}

/// True for every kind that is a measurement. Kept as a named helper because
/// chart/timeline lanes ask exactly this question and nothing else.
pub fn is_measurement(kind: &str) -> bool {
    entity_capabilities(kind).bullet_signal == Some(SignalKind::Measurement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(kind: &str) -> Vec<&'static str> {
        let caps = entity_capabilities(kind);
        let mut out = Vec::new();
        if caps.visual_subject {
            out.push("visual_subject");
        }
        if caps.named_subject {
            out.push("named_subject");
        }
        if caps.co_mention_subject {
            out.push("co_mention_subject");
        }
        if caps.named_kind {
            out.push("named_kind");
        }
        if caps.visual_query {
            out.push("visual_query");
        }
        if caps.media_outlet {
            out.push("media_outlet");
        }
        out
    }

    /// The five call sites this module replaced disagreed by design. Pinning
    /// the exact sets makes an accidental widening a test failure instead of a
    /// silent change in what production extracts and pictures.
    #[test]
    fn capability_sets_match_the_legacy_lists_exactly() {
        assert_eq!(
            names("PERSON"),
            [
                "visual_subject",
                "named_subject",
                "co_mention_subject",
                "named_kind",
                "visual_query"
            ]
        );
        assert_eq!(
            names("OBJECT"),
            [
                "visual_subject",
                "co_mention_subject",
                "named_kind",
                "visual_query"
            ]
        );
        assert_eq!(
            names("STRUCTURE"),
            ["visual_subject", "co_mention_subject", "visual_query"]
        );
        assert_eq!(
            names("EVENT"),
            ["visual_subject", "co_mention_subject", "named_kind"]
        );
        assert_eq!(
            names("ROLE"),
            ["named_kind"],
            "a role token is a name, never a portrait"
        );
        assert_eq!(
            names("ANIMAL"),
            ["visual_subject", "co_mention_subject", "named_kind"]
        );
        assert_eq!(
            names("ARTWORK"),
            ["visual_subject", "co_mention_subject", "named_kind"]
        );
        assert_eq!(names("CONCEPT"), Vec::<&str>::new());
        assert_eq!(names("NUMBER"), Vec::<&str>::new());
    }

    #[test]
    fn measurements_are_not_visual_but_do_carry_a_signal() {
        for kind in ["NUMBER", "PERCENT", "MONEY", "MEASUREMENT", "DATE"] {
            let caps = entity_capabilities(kind);
            assert!(!caps.visual_subject, "{kind}");
            assert!(!caps.co_mention_subject, "{kind}");
            assert_eq!(caps.bullet_signal, Some(SignalKind::Measurement), "{kind}");
        }
        assert!(is_measurement("percent"));
        assert!(!is_measurement("PERSON"));
    }

    #[test]
    fn media_outlet_is_a_named_visual_subject_and_a_source() {
        let caps = entity_capabilities("media_outlet");
        assert!(caps.media_outlet);
        assert!(caps.visual_subject);
        assert!(caps.named_subject);
        assert!(caps.co_mention_subject);
        assert!(caps.named_kind);
        assert!(caps.visual_query);
        assert_eq!(caps.bullet_signal, Some(SignalKind::Visual));
        assert!(!entity_capabilities("ORG").media_outlet);
    }

    #[test]
    fn spelling_never_changes_the_answer() {
        assert_eq!(entity_capabilities(" org "), entity_capabilities("ORG"));
        assert_eq!(entity_capabilities("Person"), entity_capabilities("PERSON"));
    }
}
