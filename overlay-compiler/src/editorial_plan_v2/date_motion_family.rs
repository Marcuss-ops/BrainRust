//! Certified date-motion families and their payload selector aliases.
//!
//! `canvas.date_motion_family` is the only place the request chooses the date
//! animation vocabulary. Before this module the request validator and the
//! planner's pool filter each carried their own copy of the aliases, so a new
//! family (or a typo in one copy) could validate and then fall through to the
//! whole 39-motion pool. Both sides now go through [`DateMotionFamily`].

/// A certified family of date motions. `All` is `auto` or an absent selector:
/// the planner may choose from every registered date motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DateMotionFamily {
    All,
    Classic,
    Didone,
    Typewriter,
}

impl DateMotionFamily {
    /// Parses the payload selector. `None` is an unknown selector; the request
    /// validator turns that into a typed `invalid` error instead of silently
    /// widening the pool.
    pub(super) fn parse(selector: &str) -> Option<Self> {
        match selector.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Some(Self::All),
            "classic" | "date_v1" => Some(Self::Classic),
            "didone" | "date_didone_v1" => Some(Self::Didone),
            "typewriter" | "date_typewriter_v1" => Some(Self::Typewriter),
            _ => None,
        }
    }

    /// Whether a registered date motion belongs to this family.
    pub(super) fn allows(self, motion: &str) -> bool {
        match self {
            Self::All => true,
            Self::Typewriter => motion.starts_with("date_typewriter_"),
            Self::Didone => motion.starts_with("date_didone_"),
            Self::Classic => {
                !motion.starts_with("date_typewriter_") && !motion.starts_with("date_didone_")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DateMotionFamily;

    #[test]
    fn selector_aliases_share_one_vocabulary() {
        for selector in ["", "auto", "AUTO"] {
            assert_eq!(
                Some(DateMotionFamily::All),
                DateMotionFamily::parse(selector)
            );
        }
        for selector in ["classic", "date_v1", "Classic"] {
            assert_eq!(
                Some(DateMotionFamily::Classic),
                DateMotionFamily::parse(selector)
            );
        }
        for selector in ["didone", "date_didone_v1"] {
            assert_eq!(
                Some(DateMotionFamily::Didone),
                DateMotionFamily::parse(selector)
            );
        }
        for selector in ["typewriter", "date_typewriter_v1"] {
            assert_eq!(
                Some(DateMotionFamily::Typewriter),
                DateMotionFamily::parse(selector)
            );
        }
    }

    #[test]
    fn unknown_selector_is_rejected() {
        assert_eq!(None, DateMotionFamily::parse("didone_v2"));
        assert_eq!(None, DateMotionFamily::parse("date_typewriter"));
    }

    #[test]
    fn families_are_disjoint() {
        let typewriter = "date_typewriter_year_type";
        let didone = "date_didone_era_settle";
        let classic = "date_fade_rise";

        assert!(DateMotionFamily::Typewriter.allows(typewriter));
        assert!(!DateMotionFamily::Typewriter.allows(didone));
        assert!(!DateMotionFamily::Typewriter.allows(classic));

        assert!(DateMotionFamily::Didone.allows(didone));
        assert!(!DateMotionFamily::Didone.allows(typewriter));
        assert!(!DateMotionFamily::Didone.allows(classic));

        assert!(DateMotionFamily::Classic.allows(classic));
        assert!(!DateMotionFamily::Classic.allows(typewriter));
        assert!(!DateMotionFamily::Classic.allows(didone));

        assert!(DateMotionFamily::All.allows(typewriter));
        assert!(DateMotionFamily::All.allows(didone));
        assert!(DateMotionFamily::All.allows(classic));
    }
}
