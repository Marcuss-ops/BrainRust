//! Canonical media ranking port (TICKET-16).
//!
//! This is a line-by-line port of the decisions that PipelineGen's
//! `internal/capabilities/mediamemory` package makes for one slot:
//!
//! ```text
//! candidates ─► filter flags ─► 7 mandatory gates ─► ranking input
//!            ─► repetition penalty ─► weighted score + verdict
//!            ─► drop ─► stable sort (score DESC, asset_id ASC)
//!            ─► diversity-aware top-1 pick
//! ```
//!
//! The arithmetic mirrors the Go association order exactly
//! (`semantic*0.30 + exact*0.20 + ... - repetition - rights`) so the same
//! inputs produce the same `f64`, not merely a similar ranking. The Go owner
//! keeps persistence, provider I/O and projection; it does not re-rank.
//!
//! Canonical constants (immutable until a version bump):
//!
//! | seat                | weight |
//! |---------------------|--------|
//! | semantic            | 0.30   |
//! | exact_match         | 0.20   |
//! | visual              | 0.15   |
//! | manual_approval     | 0.15   |
//! | quality             | 0.10   |
//! | historical_success  | 0.05   |
//! | duration_fit        | 0.05   |
//!
//! Verdict bands: `> +0.05` accept, `< 0.0` drop, otherwise downrank.

use serde::{Deserialize, Serialize};

/// Canonical Phase 1.x weights. Mirrors `DefaultRankWeights`/`Weights()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankWeights {
    pub semantic: f64,
    pub exact_match: f64,
    pub visual: f64,
    pub manual_approval: f64,
    pub quality: f64,
    pub historical_success: f64,
    pub duration_fit: f64,
}

pub fn default_weights() -> RankWeights {
    RankWeights {
        semantic: 0.30,
        exact_match: 0.20,
        visual: 0.15,
        manual_approval: 0.15,
        quality: 0.10,
        historical_success: 0.05,
        duration_fit: 0.05,
    }
}

/// Canonical verdict bands. Accept is exclusive, drop is exclusive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerdictThresholds {
    pub accept: f64,
    pub drop: f64,
}

pub fn default_verdict_thresholds() -> VerdictThresholds {
    VerdictThresholds {
        accept: 0.05,
        drop: 0.0,
    }
}

/// Post-rank verdict (closed set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankVerdict {
    Accept,
    Downrank,
    Drop,
}

impl RankVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            RankVerdict::Accept => "accept",
            RankVerdict::Downrank => "downrank",
            RankVerdict::Drop => "drop",
        }
    }
}

/// Canonical seven pre-rank gates (closed vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreRankGate {
    ValidLicense,
    AvailableMedia,
    ValidDuration,
    SupportedFormat,
    CompatibleAspect,
    NoCorruption,
    NoDuplicates,
}

impl PreRankGate {
    pub fn as_str(self) -> &'static str {
        match self {
            PreRankGate::ValidLicense => "valid_license",
            PreRankGate::AvailableMedia => "available_media",
            PreRankGate::ValidDuration => "valid_duration",
            PreRankGate::SupportedFormat => "supported_format",
            PreRankGate::CompatibleAspect => "compatible_aspect",
            PreRankGate::NoCorruption => "no_corruption",
            PreRankGate::NoDuplicates => "no_duplicates",
        }
    }
}

/// Slots the cascade can request. The seven concrete values are the Go
/// `media.SlotKind` set; an unknown value fails closed at the boundary.
///
/// `Any` is the neutral value for a caller that ranks a pool BEFORE a slot is
/// assigned (the Brain orchestrator does exactly that): it disables the
/// aspect gate instead of pretending the pool belongs to one slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    PrimaryVideo,
    SecondaryImage,
    EvidenceOverlay,
    Map,
    Portrait,
    Document,
    Background,
    Any,
}

pub fn parse_slot_kind(value: &str) -> Option<SlotKind> {
    match value {
        "primary_video" => Some(SlotKind::PrimaryVideo),
        "secondary_image" => Some(SlotKind::SecondaryImage),
        "evidence_overlay" => Some(SlotKind::EvidenceOverlay),
        "map" => Some(SlotKind::Map),
        "portrait" => Some(SlotKind::Portrait),
        "document" => Some(SlotKind::Document),
        "background" => Some(SlotKind::Background),
        "any" => Some(SlotKind::Any),
        _ => None,
    }
}

/// Operator-curated binding envelope. Mirrors the fields `buildRankingInput`
/// reads; nothing else crosses the boundary.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RankBinding {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub asset_id: String,
    #[serde(default)]
    pub start_ms: i64,
    #[serde(default)]
    pub end_ms: i64,
    #[serde(default)]
    pub approval_status: String,
    #[serde(default)]
    pub manual_score: f64,
    #[serde(default)]
    pub semantic_score: f64,
    #[serde(default)]
    pub quality_score: f64,
    #[serde(default)]
    pub success_score: f64,
}

/// One candidate as the resolver sees it before gating.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RankCandidate {
    #[serde(default)]
    pub candidate_id: String,
    #[serde(default)]
    pub asset_id: String,
    #[serde(default)]
    pub video_id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub candidate_score: f64,
    #[serde(default)]
    pub rights_status: String,
    #[serde(default)]
    pub discovery_status: String,
    #[serde(default)]
    pub materialization_status: String,
    /// BCP-47 language of the candidate. It travels with the pool for audit
    /// and for callers that filter pre-rank; it intentionally does NOT add a
    /// seat, so the canonical weighted formula and its parity fixtures stay
    /// bit-identical. Language scoring lives in `local_score` (the catalog
    /// lexical path), where it is a ±0.10 soft signal.
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub binding: Option<RankBinding>,
    /// Caller-supplied ordering preference, added to the final score AFTER the
    /// verdict is classified. It cannot resurrect a dropped candidate and it
    /// is not part of the breakdown: the canonical seats stay auditable.
    #[serde(default)]
    pub bonus: f64,
}

/// One slot pool to rank.
#[derive(Debug, Clone, Deserialize)]
pub struct RankRequest {
    #[serde(default)]
    pub scene_duration_ms: i64,
    #[serde(default)]
    pub prev_video_id: String,
    /// Scene language (BCP-47). Contract-only: see RankCandidate::language.
    #[serde(default)]
    pub scene_language: String,
    pub slot_kind: String,
    #[serde(default)]
    pub candidates: Vec<RankCandidate>,
}

/// Per-candidate score breakdown: every seat plus both penalties.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ScoreBreakdown {
    pub semantic: f64,
    pub exact_match: f64,
    pub visual: f64,
    pub manual_approval: f64,
    pub quality: f64,
    pub historical_success: f64,
    pub duration_fit: f64,
    pub repetition_penalty: f64,
    pub rights_penalty: f64,
}

/// One surviving candidate in rank order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedCandidate {
    pub asset_id: String,
    pub candidate_id: String,
    pub source_url: String,
    pub provider: String,
    pub media_type: String,
    pub duration_ms: i64,
    pub rights_status: String,
    pub final_score: f64,
    pub verdict: RankVerdict,
    pub reason: String,
    pub breakdown: ScoreBreakdown,
}

/// One dropped candidate with the gate that removed it (`None` when the
/// envelope itself was malformed or the score landed in the drop band).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedDrop {
    pub asset_id: String,
    pub candidate_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    pub reason: String,
}

/// The full ranking outcome for one slot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankOutcome {
    pub ranked: Vec<RankedCandidate>,
    pub drops: Vec<RankedDrop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<RankedCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankError {
    pub message: String,
}

impl RankError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

// ── Canonical math ─────────────────────────────────────────────────

pub fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// Canonical duration fit used by the ranker gates (`ComputeDurationFit`):
/// 1.0 at ratio 1.0, 0.0 at the 0.5 / 2.0 boundaries, linear between,
/// and 0.0 outside the window. Zero/negative durations return 0.0.
pub fn compute_duration_fit(scene_duration_ms: i64, binding_duration_ms: i64) -> f64 {
    if scene_duration_ms <= 0 || binding_duration_ms <= 0 {
        return 0.0;
    }
    let ratio = binding_duration_ms as f64 / scene_duration_ms as f64;
    if ratio < 0.5 || ratio > 2.0 {
        return 0.0;
    }
    if ratio < 1.0 {
        1.0 - (1.0 - ratio) * 2.0
    } else {
        1.0 - (ratio - 1.0) * 2.0
    }
}

/// Canonical duration fit used by the resolver's ranking input
/// (`durationFitScore`): 1.0 inside ±10%, linear to 0.0 at 0.5 / 2.0, and a
/// neutral 0.5 when neither duration is known.
pub fn duration_fit_score(scene_ms: i64, candidate_ms: i64) -> f64 {
    if scene_ms <= 0 || candidate_ms <= 0 {
        if scene_ms <= 0 && candidate_ms <= 0 {
            return 0.5;
        }
        return 0.0;
    }
    let ratio = candidate_ms as f64 / scene_ms as f64;
    if (0.9..=1.1).contains(&ratio) {
        return 1.0;
    }
    if (0.5..=2.0).contains(&ratio) {
        let mut d = ratio;
        if d > 1.0 {
            d = 2.0 - d;
        }
        return (d - 0.5) / 0.4;
    }
    0.0
}

/// Canonical rights penalty for one status (`PopulateRightsPenalty`):
/// verified → 0.0, unknown/denied/expired → 1.0. An unknown status string
/// fails closed with the full penalty.
pub fn rights_penalty(status: &str) -> f64 {
    match status {
        "verified" => 0.0,
        _ => 1.0,
    }
}

/// The resolver's lighter rights seat (`buildRankingInput`): 0.30 for any
/// non-empty, non-verified status.
pub fn resolver_rights_penalty(status: &str) -> f64 {
    if !status.is_empty() && status != "verified" {
        0.30
    } else {
        0.0
    }
}

/// Candidate video identity with the canonical AssetID fallback.
pub fn candidate_video_id(candidate: &RankCandidate) -> String {
    if !candidate.video_id.is_empty() {
        candidate.video_id.clone()
    } else {
        candidate.asset_id.clone()
    }
}

/// Canonical consecutive-source penalty (`PopulateRepetitionPenalty`): the
/// only component still fed by a live input after the usage log was retired.
pub fn repetition_penalty(candidate: &RankCandidate, prev_video_id: &str) -> f64 {
    let video_id = candidate_video_id(candidate);
    let mut penalty = 0.0;
    if !prev_video_id.is_empty() && !video_id.is_empty() && video_id == prev_video_id {
        penalty += 0.3;
    }
    if penalty > 1.5 {
        penalty = 1.5;
    }
    penalty
}

// ── Gates ──────────────────────────────────────────────────────────

pub fn has_available_media(candidate: &RankCandidate) -> bool {
    matches!(candidate.materialization_status.as_str(), "hot" | "warm")
}

pub fn has_valid_duration(candidate: &RankCandidate) -> bool {
    candidate.duration_ms >= 0
}

/// Supported-format gate. An empty media type is the legacy ambiguous
/// sentinel and is accepted, exactly as in Go.
pub fn has_supported_format(candidate: &RankCandidate) -> bool {
    if candidate.media_type.is_empty() {
        return true;
    }
    matches!(
        candidate.media_type.as_str(),
        "video" | "image" | "audio" | "music"
    )
}

/// A candidate whose envelope cannot be projected is dropped before the
/// gates; the ranker never scores an unidentifiable or unknown-state row.
pub fn media_candidate_is_well_formed(candidate: &RankCandidate) -> bool {
    if candidate.asset_id.is_empty() {
        return false;
    }
    if !matches!(
        candidate.materialization_status.as_str(),
        "cold" | "warm" | "hot" | "failed"
    ) {
        return false;
    }
    matches!(
        candidate.discovery_status.as_str(),
        "queued" | "searched" | "analyzed" | "indexed" | "failed" | "materialized"
    )
}

/// Slot/media-type mismatch. Empty media type is ambiguous and accepted.
pub fn aspect_mismatch_for(slot: SlotKind, media_type: &str) -> bool {
    if media_type.is_empty() {
        return false;
    }
    match slot {
        SlotKind::Any => false,
        SlotKind::PrimaryVideo => media_type != "video",
        SlotKind::SecondaryImage
        | SlotKind::EvidenceOverlay
        | SlotKind::Map
        | SlotKind::Portrait
        | SlotKind::Document
        | SlotKind::Background => media_type != "image",
    }
}

/// The resolver-set gate flags (`buildFilterFlags`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateFlags {
    pub is_duplicate: bool,
    pub missing_rights: bool,
    pub aspect_mismatch: bool,
    pub contaminated: bool,
}

pub fn build_gate_flags(candidate: &RankCandidate, slot: SlotKind) -> GateFlags {
    GateFlags {
        is_duplicate: false,
        missing_rights: !candidate.rights_status.is_empty()
            && candidate.rights_status != "verified",
        aspect_mismatch: aspect_mismatch_for(slot, &candidate.media_type),
        contaminated: candidate.materialization_status == "failed",
    }
}

/// The canonical seven gates in evaluation order.
pub fn run_mandatory_gates(candidate: &RankCandidate, flags: GateFlags) -> Option<PreRankGate> {
    if flags.is_duplicate {
        return Some(PreRankGate::NoDuplicates);
    }
    if flags.missing_rights {
        return Some(PreRankGate::ValidLicense);
    }
    if flags.contaminated {
        return Some(PreRankGate::NoCorruption);
    }
    if !has_available_media(candidate) {
        return Some(PreRankGate::AvailableMedia);
    }
    if !has_valid_duration(candidate) {
        return Some(PreRankGate::ValidDuration);
    }
    if !has_supported_format(candidate) {
        return Some(PreRankGate::SupportedFormat);
    }
    if flags.aspect_mismatch {
        return Some(PreRankGate::CompatibleAspect);
    }
    None
}

// ── Scoring ────────────────────────────────────────────────────────

/// Projects one filtered candidate onto the canonical ranking seats
/// (`buildRankingInput`).
pub fn build_score_breakdown(
    scene_duration_ms: i64,
    candidate: &RankCandidate,
    binding: Option<&RankBinding>,
) -> ScoreBreakdown {
    let mut seats = ScoreBreakdown {
        semantic: 0.0,
        exact_match: 0.0,
        visual: 0.0,
        manual_approval: 0.0,
        quality: 0.5,
        historical_success: 0.4,
        duration_fit: 0.0,
        repetition_penalty: 0.0,
        rights_penalty: 0.0,
    };

    match binding {
        Some(binding) if !binding.asset_id.is_empty() => {
            seats.semantic = clamp01(binding.semantic_score);
            seats.exact_match = 1.0;
            seats.visual = 0.5;
            seats.manual_approval = 0.0;
            if binding.approval_status == "approved" {
                seats.manual_approval = clamp01(binding.manual_score);
            }
            seats.quality = clamp01(binding.quality_score);
            seats.historical_success = clamp01(binding.success_score);
        }
        _ if candidate.candidate_score > 0.0 => {
            seats.semantic = clamp01(candidate.candidate_score);
            seats.exact_match = 0.0;
            seats.visual = 0.0;
            seats.manual_approval = 0.0;
            seats.quality = 0.5;
            seats.historical_success = 0.4;
        }
        _ => {
            seats.semantic = 0.0;
            seats.exact_match = 0.0;
            seats.visual = 0.0;
            seats.manual_approval = 0.0;
            seats.quality = 0.5;
            seats.historical_success = 0.4;
        }
    }

    seats.duration_fit = duration_fit_score(scene_duration_ms, candidate.duration_ms);
    seats.rights_penalty = resolver_rights_penalty(&candidate.rights_status);
    seats
}

/// Canonical weighted formula. Association order mirrors Go so the result is
/// bit-identical, not merely equivalent.
pub fn final_score(seats: &ScoreBreakdown, weights: &RankWeights) -> f64 {
    (seats.semantic * weights.semantic)
        + (seats.exact_match * weights.exact_match)
        + (seats.visual * weights.visual)
        + (seats.manual_approval * weights.manual_approval)
        + (seats.quality * weights.quality)
        + (seats.historical_success * weights.historical_success)
        + (seats.duration_fit * weights.duration_fit)
        - seats.repetition_penalty
        - seats.rights_penalty
}

pub fn classify_score(score: f64, thresholds: VerdictThresholds) -> RankVerdict {
    if score > thresholds.accept {
        RankVerdict::Accept
    } else if score < thresholds.drop {
        RankVerdict::Drop
    } else {
        RankVerdict::Downrank
    }
}

pub fn verdict_reason(verdict: RankVerdict) -> &'static str {
    match verdict {
        RankVerdict::Accept => "score above accept threshold",
        RankVerdict::Downrank => "score within downrank band",
        RankVerdict::Drop => "score below drop threshold (penalty or malformed)",
    }
}

// ── Ordering + diversity pick ──────────────────────────────────────

/// Score for an ordered candidate, kept alongside the video identity the
/// pick step needs. Internal on purpose: it is not part of the wire contract.
#[derive(Debug, Clone)]
struct ScoredCandidate {
    candidate: RankCandidate,
    ranked: RankedCandidate,
}

/// Stable ordering: higher final score first, ties by AssetID ascending.
fn less_ranked(a: &ScoredCandidate, b: &ScoredCandidate) -> bool {
    if a.ranked.final_score != b.ranked.final_score {
        return a.ranked.final_score > b.ranked.final_score;
    }
    a.ranked.asset_id < b.ranked.asset_id
}

/// Canonical diversity-aware top-1 pick (`PickTopFromRose`). The input is
/// already sorted; a non-consecutive alternative within
/// `DIVERSE_SCORE_DELTA` displaces a top-1 that repeats `prev_video_id`.
pub const DIVERSE_SCORE_DELTA: f64 = 0.10;

fn pick_top(rose: &[ScoredCandidate], prev_video_id: &str) -> Option<usize> {
    if rose.is_empty() {
        return None;
    }
    if prev_video_id.is_empty() {
        return Some(0);
    }
    let top_video_id = candidate_video_id(&rose[0].candidate);
    if top_video_id.is_empty() || top_video_id != prev_video_id {
        return Some(0);
    }
    for (index, alt) in rose.iter().enumerate().skip(1) {
        let alt_video_id = candidate_video_id(&alt.candidate);
        if alt_video_id == prev_video_id || alt_video_id.is_empty() {
            continue;
        }
        if rose[0].ranked.final_score - alt.ranked.final_score <= DIVERSE_SCORE_DELTA {
            return Some(index);
        }
    }
    Some(0)
}

// ── Entry point ────────────────────────────────────────────────────

/// Ranks one slot pool. Fails closed only on an unknown slot; every
/// per-candidate problem is a recorded drop, never a silent re-rank.
pub fn rank_candidates(request: &RankRequest) -> Result<RankOutcome, RankError> {
    let slot = parse_slot_kind(&request.slot_kind).ok_or_else(|| {
        RankError::new(format!(
            "unknown slot_kind {:?}: expected primary_video, secondary_image, evidence_overlay, map, portrait, document, background or any",
            request.slot_kind
        ))
    })?;

    let weights = default_weights();
    let thresholds = default_verdict_thresholds();

    let mut drops = Vec::new();
    let mut ranked: Vec<ScoredCandidate> = Vec::new();

    for candidate in &request.candidates {
        if !media_candidate_is_well_formed(candidate) {
            drops.push(RankedDrop {
                asset_id: candidate.asset_id.clone(),
                candidate_id: candidate.candidate_id.clone(),
                gate: None,
                reason: "malformed candidate envelope".to_string(),
            });
            continue;
        }
        let flags = build_gate_flags(candidate, slot);
        if let Some(gate) = run_mandatory_gates(candidate, flags) {
            drops.push(RankedDrop {
                asset_id: candidate.asset_id.clone(),
                candidate_id: candidate.candidate_id.clone(),
                gate: Some(gate.as_str().to_string()),
                reason: format!("failed the {} gate", gate.as_str()),
            });
            continue;
        }

        let mut seats = build_score_breakdown(
            request.scene_duration_ms,
            candidate,
            candidate.binding.as_ref(),
        );
        seats.repetition_penalty = repetition_penalty(candidate, &request.prev_video_id);

        let score = final_score(&seats, &weights);
        let verdict = classify_score(score, thresholds);
        if verdict == RankVerdict::Drop {
            drops.push(RankedDrop {
                asset_id: candidate.asset_id.clone(),
                candidate_id: candidate.candidate_id.clone(),
                gate: None,
                reason: verdict_reason(verdict).to_string(),
            });
            continue;
        }
        ranked.push(ScoredCandidate {
            candidate: candidate.clone(),
            ranked: RankedCandidate {
                asset_id: candidate.asset_id.clone(),
                candidate_id: candidate.candidate_id.clone(),
                source_url: candidate.source_url.clone(),
                provider: candidate.provider.clone(),
                media_type: candidate.media_type.clone(),
                duration_ms: candidate.duration_ms,
                rights_status: candidate.rights_status.clone(),
                // The verdict is decided on the canonical score; the bonus is an
                // ordering preference layered on top of it, never a rescue.
                final_score: score + candidate.bonus,
                verdict,
                reason: verdict_reason(verdict).to_string(),
                breakdown: seats,
            },
        });
    }

    ranked.sort_by(|a, b| {
        if less_ranked(a, b) {
            std::cmp::Ordering::Less
        } else if less_ranked(b, a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });

    let winner = pick_top(&ranked, &request.prev_video_id)
        .and_then(|index| ranked.get(index))
        .map(|scored| scored.ranked.clone());

    Ok(RankOutcome {
        ranked: ranked.into_iter().map(|scored| scored.ranked).collect(),
        drops,
        winner,
    })
}

#[cfg(test)]
#[path = "rank_tests.rs"]
mod tests;
