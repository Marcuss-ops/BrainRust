//! Selection origin: WHY an item got its motion.
//!
//! ANIMRAND-16. Without provenance an operator cannot tell a deliberate pin
//! from an automatic draw or a fallback, and the audit's "zero silent
//! fallbacks" cannot be measured. Every animated item carries
//! `params.motion_origin` with one of four values:
//!
//! - `explicit` — the producer pinned it (directive, bullet pin, authored plan);
//! - `auto`     — drawn from the certified pool by the plan's creative seed;
//! - `group`    — choreography assigned for a group/container as a unit;
//! - `fallback` — a last-resort value that normal requests should never need.
//!
//! The value is observability, not render input, but it rides `params` (an
//! open map on both sides) so it survives the seal without a contract change.

use serde_json::{json, Value};

use super::PlanItemV2;

pub(super) const ORIGIN_EXPLICIT: &str = "explicit";
pub(super) const ORIGIN_AUTO: &str = "auto";
pub(super) const ORIGIN_GROUP: &str = "group";
pub(super) const ORIGIN_FALLBACK: &str = "fallback";

/// The closed vocabulary, kept in one place so a new value cannot be invented
/// silently.
pub(super) const ORIGINS: [&str; 4] = [ORIGIN_EXPLICIT, ORIGIN_AUTO, ORIGIN_GROUP, ORIGIN_FALLBACK];

/// Writes the origin of the item's entrance (and caption, when present).
pub(super) fn mark_motion_origin(item: &mut PlanItemV2, origin: &str) {
    debug_assert!(ORIGINS.contains(&origin), "unknown motion origin {origin}");
    let mut params = item
        .params
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    params.insert("motion_origin".to_string(), json!(origin));
    item.params = Some(Value::Object(params));
}

pub(super) fn mark_caption_origin(item: &mut PlanItemV2, origin: &str) {
    let mut params = item
        .params
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    params.insert("caption_motion_origin".to_string(), json!(origin));
    item.params = Some(Value::Object(params));
}

/// Marks every entrance that arrived with the candidate as `explicit`. Runs
/// once, after admission, before any automatic lane: from then on an empty id
/// means "the planner still owns this choice" and a non-empty one means the
/// producer already did.
pub(super) fn stamp_explicit_existing(items: &mut [PlanItemV2]) {
    for item in items.iter_mut() {
        if !item.motion_id.trim().is_empty() {
            mark_motion_origin(item, ORIGIN_EXPLICIT);
        }
        if !item.caption_motion_id.trim().is_empty() {
            mark_caption_origin(item, ORIGIN_EXPLICIT);
        }
    }
}

/// Closes the invariant: an item that still has a motion but no recorded
/// origin came from outside the lanes (a group/container authority), so it is
/// labelled conservatively instead of being reported as automatic.
pub(super) fn stamp_missing_origins(items: &mut [PlanItemV2]) {
    for item in items.iter_mut() {
        let has_origin = item
            .params
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|params| params.get("motion_origin").is_some());
        if !item.motion_id.trim().is_empty() && !has_origin {
            mark_motion_origin(item, ORIGIN_EXPLICIT);
        }
        let has_caption_origin = item
            .params
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|params| params.get("caption_motion_origin").is_some());
        if !item.caption_motion_id.trim().is_empty() && !has_caption_origin {
            mark_caption_origin(item, ORIGIN_EXPLICIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::PlanItemV2;
    use super::{mark_motion_origin, ORIGIN_AUTO, ORIGIN_EXPLICIT, ORIGIN_FALLBACK, ORIGIN_GROUP};

    #[test]
    fn the_origin_set_is_closed() {
        let known = [ORIGIN_AUTO, ORIGIN_EXPLICIT, ORIGIN_GROUP, ORIGIN_FALLBACK];
        assert_eq!(4, known.len());
        for origin in known {
            let mut item = PlanItemV2::default();
            mark_motion_origin(&mut item, origin);
            assert_eq!(
                Some(origin),
                item.params
                    .as_ref()
                    .and_then(|params| params.get("motion_origin"))
                    .and_then(|value| value.as_str())
            );
        }
    }
}
