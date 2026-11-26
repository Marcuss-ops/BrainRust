//! Gap-aware admission and pacing (Cluster C: ITCLOSE-05 / AUGRENDER-05).
//!
//! Primary admission ranks candidates by kind and spends one global ceiling.
//! That is the right rule for quality — a verified portrait beats a date card —
//! but it is not a distribution rule: score-ordered phrases cluster where the
//! transcript is dense and leave 30–55 s deserts elsewhere. Raising
//! `max_phrase_overlays` in r86 only multiplied collisions and never closed the
//! 54 s hole.
//!
//! This module measures the uncovered intervals of the admitted set and runs
//! ONE targeted second pass. It promotes candidates that already exist, are
//! anchored to certified word timing, and fit entirely inside a gap while
//! respecting the scene end, the readable cooldown and zero overlap. Nothing is
//! invented, nothing is re-timed, and the promoted card cannot cover footage
//! that already carries copy.

use std::collections::{BTreeMap, BTreeSet};

use super::super::response::{CoverageGapV2, CoverageV2, PhraseDropV2, PhraseShortfallV2};
use super::*;

/// Editorial default: no uncovered stretch longer than 25 s.
pub(super) const DEFAULT_MAX_GAP_US: i64 = 25_000_000;
/// Shortest distance between a promoted card and the adjacent overlay (or the
/// gap boundary). 1.2 s is the shortest hold the editorial contract treats as
/// readable.
pub(super) const DEFAULT_GAP_COOLDOWN_US: i64 = 1_200_000;
/// Editorial coverage target the Go pre-render gate enforces.
pub(super) const DEFAULT_COVERAGE_TARGET_PERCENT: f64 = 25.0;
/// Bound on promotions per plan. A pathological request must not turn pacing
/// into an unbounded loop; real plans promote a few dozen cards at most.
const MAX_GAP_PROMOTIONS: usize = 256;

pub(super) fn max_gap_us(request: &EditorialPlanV2Request) -> i64 {
    if request.canvas.max_gap_us > 0 {
        request.canvas.max_gap_us
    } else {
        DEFAULT_MAX_GAP_US
    }
}

pub(super) fn gap_cooldown_us(request: &EditorialPlanV2Request) -> i64 {
    if request.canvas.gap_cooldown_us > 0 {
        request.canvas.gap_cooldown_us
    } else {
        DEFAULT_GAP_COOLDOWN_US
    }
}

pub(super) fn coverage_target_percent(request: &EditorialPlanV2Request) -> f64 {
    if request.canvas.coverage_min_percent > 0.0 {
        request.canvas.coverage_min_percent
    } else {
        DEFAULT_COVERAGE_TARGET_PERCENT
    }
}

/// The canonical plan extent: the last scene end, never shorter than the
/// certified audio. It mirrors `seal` so coverage and the sealed duration agree.
pub(super) fn plan_extent_us(request: &EditorialPlanV2Request) -> i64 {
    let scene_end = request
        .scenes
        .iter()
        .map(|scene| scene.timeline_start_us.saturating_add(scene.duration_us))
        .max()
        .unwrap_or(0);
    let audio_end = request
        .seal
        .as_ref()
        .map(|seal| seal.audio_duration_us)
        .unwrap_or(0);
    scene_end.max(audio_end)
}

/// A duration-relative phrase budget. With `phrase_per_minute` unset the fixed
/// `budgets.phrase_overlays` is authoritative. With it set, the ceiling is
/// derived from the plan duration and capped by an explicit caller ceiling, so
/// a nine-minute video stops requesting 50 cards it can never place.
pub(super) fn phrase_budget(request: &EditorialPlanV2Request, fixed: i64) -> i64 {
    let rate = request.canvas.phrase_per_minute;
    if !rate.is_finite() || rate <= 0.0 {
        return fixed;
    }
    let minutes = plan_extent_us(request) as f64 / 60_000_000.0;
    let derived = (rate * minutes).round().max(1.0) as i64;
    if fixed > 0 {
        derived.min(fixed)
    } else {
        derived
    }
}

/// True when a kind is an editorial surface that counts as coverage. A
/// full-frame background or a decorative light leak is not an editorial card
/// and must not mask a desert.
fn counts_as_coverage(item: &PlanItemV2) -> bool {
    if item.kind == "light_leak" {
        return false;
    }
    let template = item.template_id.trim().to_ascii_lowercase();
    !matches!(template.as_str(), "background" | "video_background")
}

/// The union of every admitted editorial window, clipped to the plan extent.
pub(super) fn covered_intervals(items: &[PlanItemV2], plan_end_us: i64) -> Vec<(i64, i64)> {
    let mut windows: Vec<(i64, i64)> = items
        .iter()
        .filter(|item| counts_as_coverage(item))
        .filter_map(|item| {
            let (start_us, end_us) = item.window_us();
            let start_us = start_us.max(0);
            let end_us = end_us.min(plan_end_us);
            (end_us > start_us).then_some((start_us, end_us))
        })
        .collect();
    windows.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(windows.len());
    for (start_us, end_us) in windows {
        if let Some(last) = merged.last_mut() {
            if start_us <= last.1 {
                last.1 = last.1.max(end_us);
                continue;
            }
        }
        merged.push((start_us, end_us));
    }
    merged
}

/// The complement of the covered union inside the plan.
pub(super) fn uncovered_intervals(covered: &[(i64, i64)], plan_end_us: i64) -> Vec<(i64, i64)> {
    let mut gaps = Vec::new();
    let mut cursor = 0_i64;
    for (start_us, end_us) in covered {
        if *start_us > cursor {
            gaps.push((cursor, *start_us));
        }
        cursor = cursor.max(*end_us);
    }
    if plan_end_us > cursor {
        gaps.push((cursor, plan_end_us));
    }
    gaps
}

/// A phrase/quote/bullet candidate carries certified TTS word timing only when
/// the builder stamped it. Without that anchor the gap pass may not place it:
/// guessing an instant is exactly how a phrase lands mid-sentence.
pub(super) fn word_timing_anchored(item: &PlanItemV2) -> bool {
    item.params
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|params| params.get("timing_anchor"))
        .and_then(Value::as_str)
        == Some("word_timing")
}

fn candidate_priority(item: &PlanItemV2) -> f64 {
    item.params
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|params| params.get("priority"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn is_gap_promotable(request: &EditorialPlanV2Request, item: &PlanItemV2) -> bool {
    if item.text.trim().is_empty() {
        return false;
    }
    match item.kind.as_str() {
        "text_phrase" => {
            // Titles and chapter bullets own the opening/cadence surface and
            // are placed by their own projection. A desert is filled with
            // spoken copy: a plain phrase or a scene bullet.
            if !matches!(role(item), "" | "scene_bullet") {
                return false;
            }
        }
        "quote" => {}
        _ => return false,
    }
    if !word_timing_anchored(item) {
        return false;
    }
    if request.canvas.maps_only {
        return false;
    }
    !category_disabled(request, item)
}

/// A promoted card must sit entirely inside the gap and must not leave a
/// sliver shorter than the cooldown on either side.
fn fits_gap(item: &PlanItemV2, gap_start_us: i64, gap_end_us: i64, cooldown_us: i64) -> bool {
    let (start_us, end_us) = item.window_us();
    if end_us <= start_us || start_us < gap_start_us || end_us > gap_end_us {
        return false;
    }
    let before = start_us - gap_start_us;
    let after = gap_end_us - end_us;
    (before == 0 || before >= cooldown_us) && (after == 0 || after >= cooldown_us)
}

/// Runs the gap pass. Returns the admitted set, the candidates that could not
/// be promoted and the promotion count.
///
/// The pass is deterministic: the largest gap is filled first, ties break on
/// the earlier start, and within a gap the highest-priority candidate wins,
/// then the earliest, then the id. Every promotion removes that candidate's
/// admission decision, so the decision log and the shortfall report describe
/// the same final plan.
pub(super) fn close_gaps(
    request: &EditorialPlanV2Request,
    mut items: Vec<PlanItemV2>,
    rejected: Vec<PlanItemV2>,
    decisions: &mut Vec<PlanDecisionV2>,
) -> (Vec<PlanItemV2>, Vec<PlanItemV2>, i64) {
    let plan_end_us = plan_extent_us(request);
    let cooldown_us = gap_cooldown_us(request).max(1);
    // The pool holds what MAY be promoted; the caller keeps the full rejected
    // list so the shortfall report still describes every dropped phrase.
    let mut pool: Vec<PlanItemV2> = rejected
        .iter()
        .filter(|item| is_gap_promotable(request, item))
        .cloned()
        .collect();
    let mut promoted_ids: BTreeSet<String> = BTreeSet::new();
    let mut promoted = 0_i64;
    while (promoted as usize) < MAX_GAP_PROMOTIONS {
        let covered = covered_intervals(&items, plan_end_us);
        let mut open = uncovered_intervals(&covered, plan_end_us);
        // Largest first, then earliest: a 54 s hole is worth more than three
        // 26 s ones, and the order must not depend on hash iteration.
        open.sort_by(|left, right| {
            (right.1 - right.0)
                .cmp(&(left.1 - left.0))
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut promoted_this_round = false;
        for (gap_start_us, gap_end_us) in open {
            let mut best: Option<usize> = None;
            for (index, candidate) in pool.iter().enumerate() {
                if !fits_gap(candidate, gap_start_us, gap_end_us, cooldown_us) {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some(current) => {
                        let (candidate_start, _) = candidate.window_us();
                        let (current_start, _) = pool[current].window_us();
                        let candidate_key = (
                            -candidate_priority(candidate),
                            candidate_start,
                            candidate.id.as_str(),
                        );
                        let current_key = (
                            -candidate_priority(&pool[current]),
                            current_start,
                            pool[current].id.as_str(),
                        );
                        candidate_key < current_key
                    }
                };
                if better {
                    best = Some(index);
                }
            }
            let Some(index) = best else {
                continue;
            };
            let candidate = pool.remove(index);
            decisions.retain(|decision| decision.candidate_id != candidate.id);
            promoted_ids.insert(candidate.id.clone());
            items.push(candidate);
            promoted += 1;
            promoted_this_round = true;
            break;
        }
        if !promoted_this_round {
            break;
        }
    }
    if promoted > 0 {
        // Keep the sealed plan in render order after an insertion.
        items.sort_by(|left, right| {
            left.start_us
                .cmp(&right.start_us)
                .then_with(|| left.id.cmp(&right.id))
        });
    }
    let unpromoted: Vec<PlanItemV2> = rejected
        .into_iter()
        .filter(|item| !promoted_ids.contains(&item.id))
        .collect();
    (items, unpromoted, promoted)
}

/// Distinct visible phrase texts, counted the same way the Go budget measures
/// materialization (a fused chain contributes one identity per segment).
pub(super) fn materialized_phrase_count(items: &[PlanItemV2]) -> i64 {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for item in items {
        if item.kind == "text_phrase" {
            insert_phrase(&mut seen, &item.text);
        }
        if !matches!(item.kind.as_str(), "phrase_chain" | "mixed_chain") {
            continue;
        }
        let Some(segments) = item
            .params
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|params| params.get("chain_segments"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for segment in segments {
            if item.kind == "mixed_chain"
                && segment.get("kind").and_then(Value::as_str) != Some("text_phrase")
            {
                continue;
            }
            if let Some(text) = segment.get("text").and_then(Value::as_str) {
                insert_phrase(&mut seen, text);
            }
        }
    }
    seen.len() as i64
}

fn insert_phrase(seen: &mut BTreeSet<String>, text: &str) {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if !normalized.is_empty() {
        seen.insert(normalized);
    }
}

/// Measures the sealed plan. This is the single coverage owner: the Go gate
/// consumes the numbers instead of re-deriving them with a different notion of
/// which item counts.
pub(super) fn measure_coverage(
    request: &EditorialPlanV2Request,
    fixed_phrase_budget: i64,
    items: &[PlanItemV2],
    gap_promotions: i64,
) -> CoverageV2 {
    let plan_end_us = plan_extent_us(request);
    let covered = covered_intervals(items, plan_end_us);
    let covered_us: i64 = covered
        .iter()
        .map(|(start_us, end_us)| end_us - start_us)
        .sum();
    let empty = |(start_us, end_us): (i64, i64)| end_us.saturating_sub(start_us) > 0;
    let mut gaps: Vec<CoverageGapV2> = uncovered_intervals(&covered, plan_end_us)
        .into_iter()
        .filter(|gap| empty(*gap))
        .map(|(start_us, end_us)| CoverageGapV2 {
            start_us,
            end_us,
            duration_us: end_us - start_us,
        })
        .collect();
    gaps.sort_by(|left, right| {
        right
            .duration_us
            .cmp(&left.duration_us)
            .then_with(|| left.start_us.cmp(&right.start_us))
    });
    let threshold_us = max_gap_us(request);
    let max_gap_us = gaps.first().map(|gap| gap.duration_us).unwrap_or(0);
    let coverage_percent = if plan_end_us > 0 {
        covered_us as f64 / plan_end_us as f64 * 100.0
    } else {
        0.0
    };
    CoverageV2 {
        plan_us: plan_end_us,
        covered_us,
        coverage_percent,
        target_percent: coverage_target_percent(request),
        max_gap_us,
        threshold_us,
        gaps_over_threshold: gaps
            .iter()
            .filter(|gap| gap.duration_us > threshold_us)
            .count() as i64,
        gaps,
        phrase_target: phrase_budget(request, fixed_phrase_budget),
        phrase_materialized: materialized_phrase_count(items),
        phrase_per_minute: request.canvas.phrase_per_minute,
        gap_promotions,
    }
}

/// Maps a decision reason onto the phrase-lane vocabulary the ticket names.
fn phrase_reason(reason_code: &str) -> &'static str {
    match reason_code {
        "PER_SCENE_LIMIT" => "scene_cap",
        "OVERLAP_LOST" | "OCCLUDED_BY_ENTITY_CARD" | "CLIPPED_OUT" => "window_collision",
        "DUPLICATE_NAMED_PHRASE" => "deduped",
        "CATEGORY_DISABLED" => "category_disabled",
        "MAPS_ONLY" => "maps_only",
        "UNANCHORED_IN_TRANSCRIPT" => "unanchored_in_transcript",
        _ => "budget_exhausted",
    }
}

/// Builds one drop record per rejected phrase candidate. A candidate that
/// reached admission and was then dropped by layout keeps that terminal reason;
/// a candidate that never had certified word timing is reported as
/// `unanchored_in_transcript`, because no pacing pass could place it without
/// guessing. The list is sorted so a replay of the same request publishes an
/// identical document.
pub(super) fn build_phrase_shortfall(
    request: &EditorialPlanV2Request,
    fixed_phrase_budget: i64,
    decisions: &[PlanDecisionV2],
    unpromoted: &[PlanItemV2],
    items: &[PlanItemV2],
) -> PhraseShortfallV2 {
    let mut drops: Vec<PhraseDropV2> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for decision in decisions {
        if !matches!(decision.kind.as_str(), "text_phrase" | "quote") {
            continue;
        }
        if !seen.insert(decision.candidate_id.clone()) {
            continue;
        }
        let unpromoted_item = unpromoted
            .iter()
            .find(|item| item.id == decision.candidate_id);
        let unanchored = unpromoted_item
            .map(|item| !word_timing_anchored(item))
            .unwrap_or(false);
        let reason = if unanchored {
            phrase_reason(REASON_UNANCHORED_IN_TRANSCRIPT)
        } else {
            phrase_reason(&decision.reason_code)
        };
        let (text, duration_us) = unpromoted_item
            .map(|item| (item.text.clone(), item.duration_us))
            .unwrap_or_default();
        drops.push(PhraseDropV2 {
            candidate_id: decision.candidate_id.clone(),
            scene_id: decision.scene_id.clone(),
            text,
            reason: reason.to_string(),
            start_us: decision.start_us,
            duration_us,
        });
    }
    // A candidate that the gap pass filtered out before admission ever saw it
    // (for example a phrase that lost its scene) still deserves one record.
    for item in unpromoted {
        if !matches!(item.kind.as_str(), "text_phrase" | "quote") {
            continue;
        }
        if !seen.insert(item.id.clone()) {
            continue;
        }
        drops.push(PhraseDropV2 {
            candidate_id: item.id.clone(),
            scene_id: item.scene_id.clone(),
            text: item.text.clone(),
            reason: if word_timing_anchored(item) {
                "budget_exhausted".to_string()
            } else {
                "unanchored_in_transcript".to_string()
            },
            start_us: item.start_us,
            duration_us: item.duration_us,
        });
    }
    drops.sort_by(|left, right| {
        left.scene_id
            .cmp(&right.scene_id)
            .then_with(|| left.start_us.cmp(&right.start_us))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    let mut by_reason: BTreeMap<String, i64> = BTreeMap::new();
    for drop in &drops {
        *by_reason.entry(drop.reason.clone()).or_default() += 1;
    }
    let materialized = materialized_phrase_count(items);
    let requested = phrase_budget(request, fixed_phrase_budget);
    PhraseShortfallV2 {
        requested,
        materialized,
        dropped: drops.len() as i64,
        by_reason,
        drops,
    }
}
