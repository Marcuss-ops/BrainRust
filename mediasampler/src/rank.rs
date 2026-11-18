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
mod tests {
    use super::*;

    fn candidate(asset_id: &str) -> RankCandidate {
        RankCandidate {
            candidate_id: format!("candidate-{asset_id}"),
            asset_id: asset_id.to_string(),
            provider: "artlist".to_string(),
            media_type: "video".to_string(),
            duration_ms: 4_000,
            candidate_score: 0.5,
            rights_status: "verified".to_string(),
            discovery_status: "searched".to_string(),
            materialization_status: "warm".to_string(),
            ..RankCandidate::default()
        }
    }

    fn request(candidates: Vec<RankCandidate>) -> RankRequest {
        RankRequest {
            scene_duration_ms: 4_000,
            prev_video_id: String::new(),
            scene_language: String::new(),
            slot_kind: "primary_video".to_string(),
            candidates,
        }
    }

    #[test]
    fn weights_and_thresholds_are_the_canonical_values() {
        let weights = default_weights();
        assert_eq!(weights.semantic, 0.30);
        assert_eq!(weights.exact_match, 0.20);
        assert_eq!(weights.visual, 0.15);
        assert_eq!(weights.manual_approval, 0.15);
        assert_eq!(weights.quality, 0.10);
        assert_eq!(weights.historical_success, 0.05);
        assert_eq!(weights.duration_fit, 0.05);
        let thresholds = default_verdict_thresholds();
        assert_eq!(thresholds.accept, 0.05);
        assert_eq!(thresholds.drop, 0.0);
    }

    #[test]
    fn verdict_bands_are_exclusive_at_the_boundaries() {
        let thresholds = default_verdict_thresholds();
        assert_eq!(classify_score(0.051, thresholds), RankVerdict::Accept);
        assert_eq!(classify_score(0.05, thresholds), RankVerdict::Downrank);
        assert_eq!(classify_score(0.0, thresholds), RankVerdict::Downrank);
        assert_eq!(classify_score(-0.001, thresholds), RankVerdict::Drop);
    }

    #[test]
    fn gates_evaluate_in_the_canonical_order() {
        let mut c = candidate("a1");
        c.materialization_status = "failed".to_string();
        let flags = build_gate_flags(&c, SlotKind::PrimaryVideo);
        assert_eq!(
            run_mandatory_gates(&c, flags),
            Some(PreRankGate::NoCorruption),
            "corruption precedes availability"
        );

        c.materialization_status = "cold".to_string();
        let flags = build_gate_flags(&c, SlotKind::PrimaryVideo);
        assert_eq!(
            run_mandatory_gates(&c, flags),
            Some(PreRankGate::AvailableMedia)
        );

        let mut c = candidate("a1");
        c.rights_status = "unknown".to_string();
        let flags = build_gate_flags(&c, SlotKind::PrimaryVideo);
        assert_eq!(
            run_mandatory_gates(&c, flags),
            Some(PreRankGate::ValidLicense),
            "rights precede availability"
        );

        let mut c = candidate("a1");
        c.media_type = "raw".to_string();
        let flags = build_gate_flags(&c, SlotKind::PrimaryVideo);
        assert_eq!(
            run_mandatory_gates(&c, flags),
            Some(PreRankGate::SupportedFormat)
        );

        let c = candidate("a1");
        let flags = build_gate_flags(&c, SlotKind::SecondaryImage);
        assert_eq!(
            run_mandatory_gates(&c, flags),
            Some(PreRankGate::CompatibleAspect)
        );
    }

    #[test]
    fn empty_media_type_and_negative_duration_follow_the_go_sentinels() {
        let mut c = candidate("a1");
        c.media_type.clear();
        assert!(has_supported_format(&c));
        c.duration_ms = -1;
        assert!(!has_valid_duration(&c));
    }

    #[test]
    fn malformed_envelopes_are_recorded_as_drops_without_a_gate() {
        let mut malformed = candidate("a1");
        malformed.asset_id.clear();
        let outcome = rank_candidates(&request(vec![malformed])).expect("slot is valid");
        assert!(outcome.ranked.is_empty());
        assert_eq!(outcome.drops.len(), 1);
        assert_eq!(outcome.drops[0].gate, None);
        assert!(outcome.winner.is_none());
    }

    #[test]
    fn an_empty_pool_ranks_nothing_and_returns_no_winner() {
        let outcome = rank_candidates(&request(Vec::new())).expect("slot is valid");
        assert!(outcome.ranked.is_empty());
        assert!(outcome.drops.is_empty());
        assert!(outcome.winner.is_none());
    }

    #[test]
    fn an_unknown_slot_fails_closed() {
        let mut req = request(vec![candidate("a1")]);
        req.slot_kind = "b_roll".to_string();
        let error = rank_candidates(&req).expect_err("unknown slot must fail");
        assert!(error.message.contains("unknown slot_kind"));
    }

    #[test]
    fn equal_scores_break_ties_by_asset_id_ascending() {
        let outcome = rank_candidates(&request(vec![
            candidate("b-asset"),
            candidate("a-asset"),
            candidate("c-asset"),
        ]))
        .expect("slot is valid");
        let order: Vec<&str> = outcome
            .ranked
            .iter()
            .map(|entry| entry.asset_id.as_str())
            .collect();
        assert_eq!(order, vec!["a-asset", "b-asset", "c-asset"]);
        assert_eq!(
            outcome.winner.as_ref().map(|w| w.asset_id.as_str()),
            Some("a-asset")
        );
    }

    #[test]
    fn the_consecutive_source_penalty_is_applied_and_never_exceeds_the_ceiling() {
        let c = candidate("a1");
        assert_eq!(repetition_penalty(&c, "a1"), 0.3);
        assert_eq!(repetition_penalty(&c, "other"), 0.0);
        assert_eq!(repetition_penalty(&c, ""), 0.0);

        let mut clip = candidate("a1");
        clip.video_id = "video-7".to_string();
        assert_eq!(repetition_penalty(&clip, "video-7"), 0.3);
        assert_eq!(
            repetition_penalty(&clip, "a1"),
            0.0,
            "video id wins over asset id"
        );
    }

    /// A strong approved binding scores 0.925; the consecutive-source
    /// penalty (0.30) brings it to 0.625, where a competitive alternative can
    /// displace it only through the diversity knob.
    fn strong_binding(asset_id: &str) -> RankCandidate {
        let mut c = candidate(asset_id);
        c.binding = Some(RankBinding {
            id: format!("binding-{asset_id}"),
            asset_id: asset_id.to_string(),
            approval_status: "approved".to_string(),
            manual_score: 1.0,
            semantic_score: 1.0,
            quality_score: 1.0,
            success_score: 1.0,
            ..RankBinding::default()
        });
        c
    }

    /// Equal final score (0.625) without the approval seat, so the pair is an
    /// exact tie on score and the AssetID tie-break decides the order.
    fn tied_alternative(asset_id: &str) -> RankCandidate {
        let mut c = candidate(asset_id);
        c.binding = Some(RankBinding {
            id: format!("binding-{asset_id}"),
            asset_id: asset_id.to_string(),
            semantic_score: 1.0,
            quality_score: 0.0,
            success_score: 0.0,
            ..RankBinding::default()
        });
        c
    }

    #[test]
    fn diversity_swaps_a_repeated_top_one_for_a_close_alternative() {
        let mut req = request(vec![strong_binding("a1"), tied_alternative("a2")]);
        req.prev_video_id = "a1".to_string();
        let outcome = rank_candidates(&req).expect("slot is valid");
        assert_eq!(
            outcome.ranked[0].asset_id, "a1",
            "the ordered selection keeps the score order and the asset-id tie-break"
        );
        assert_eq!(
            outcome.winner.as_ref().map(|w| w.asset_id.as_str()),
            Some("a2"),
            "a1 repeats the prior source and a2 is within the delta"
        );
    }

    #[test]
    fn diversity_keeps_the_top_one_when_no_alternative_is_close() {
        let mut far = tied_alternative("a2");
        if let Some(binding) = far.binding.as_mut() {
            binding.semantic_score = 0.0;
        }
        let mut req = request(vec![strong_binding("a1"), far]);
        req.prev_video_id = "a1".to_string();
        let outcome = rank_candidates(&req).expect("slot is valid");
        assert_eq!(
            outcome.winner.as_ref().map(|w| w.asset_id.as_str()),
            Some("a1"),
            "a 0.30 gap is outside the 0.10 diversity delta"
        );
    }

    #[test]
    fn binding_seats_flow_and_unapproved_manual_scores_are_zeroed() {
        let mut approved = candidate("a1");
        approved.binding = Some(RankBinding {
            id: "b1".to_string(),
            asset_id: "a1".to_string(),
            approval_status: "approved".to_string(),
            manual_score: 1.0,
            semantic_score: 0.8,
            quality_score: 0.7,
            success_score: 0.6,
            ..RankBinding::default()
        });
        let mut pending = candidate("a2");
        pending.binding = Some(RankBinding {
            id: "b2".to_string(),
            asset_id: "a2".to_string(),
            approval_status: "pending".to_string(),
            manual_score: 1.0,
            ..RankBinding::default()
        });

        let outcome = rank_candidates(&request(vec![approved, pending])).expect("slot is valid");
        let approved = outcome
            .ranked
            .iter()
            .find(|entry| entry.asset_id == "a1")
            .expect("approved candidate");
        assert_eq!(approved.breakdown.exact_match, 1.0);
        assert_eq!(approved.breakdown.visual, 0.5);
        assert_eq!(approved.breakdown.manual_approval, 1.0);

        let pending = outcome
            .ranked
            .iter()
            .find(|entry| entry.asset_id == "a2")
            .expect("pending candidate");
        assert_eq!(
            pending.breakdown.manual_approval, 0.0,
            "an unapproved binding cannot smuggle its manual score in"
        );
    }

    #[test]
    fn duration_fit_curves_match_the_two_go_variants() {
        assert_eq!(duration_fit_score(0, 0), 0.5);
        assert_eq!(duration_fit_score(0, 4_000), 0.0);
        assert_eq!(duration_fit_score(4_000, 4_000), 1.0);
        assert_eq!(duration_fit_score(4_000, 3_600), 1.0);
        assert_eq!(duration_fit_score(4_000, 2_000), 0.0);
        assert!((duration_fit_score(4_000, 2_800) - 0.5).abs() < 1e-12);

        assert_eq!(compute_duration_fit(0, 0), 0.0);
        assert_eq!(compute_duration_fit(4_000, 4_000), 1.0);
        assert_eq!(compute_duration_fit(4_000, 2_000), 0.0);
        assert!((compute_duration_fit(4_000, 3_000) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn rights_penalties_match_both_go_seats() {
        assert_eq!(rights_penalty("verified"), 0.0);
        for status in ["unknown", "denied", "expired", ""] {
            assert_eq!(rights_penalty(status), 1.0, "{status}");
        }
        assert_eq!(resolver_rights_penalty("verified"), 0.0);
        assert_eq!(resolver_rights_penalty(""), 0.0);
        assert_eq!(resolver_rights_penalty("expired"), 0.30);
    }

    #[test]
    fn a_negative_score_is_dropped_with_the_canonical_reason() {
        // Weak unapproved binding (0.275) minus the 0.30 consecutive-source
        // penalty lands below zero, so the verdict drops it. Rights stay empty
        // so the license gate does not pre-empt the score path.
        let mut c = candidate("a1");
        c.rights_status.clear();
        c.duration_ms = 100;
        c.binding = Some(RankBinding {
            id: "binding-a1".to_string(),
            asset_id: "a1".to_string(),
            semantic_score: 0.0,
            quality_score: 0.0,
            success_score: 0.0,
            ..RankBinding::default()
        });
        let mut req = request(vec![c]);
        req.prev_video_id = "a1".to_string();
        let outcome = rank_candidates(&req).expect("slot is valid");
        assert!(outcome.ranked.is_empty(), "{outcome:?}");
        assert_eq!(outcome.drops.len(), 1);
        assert_eq!(
            outcome.drops[0].reason,
            "score below drop threshold (penalty or malformed)"
        );
    }

    #[test]
    fn caller_bonus_orders_candidates_without_touching_the_verdict() {
        let mut strong = candidate("strong");
        strong.candidate_score = 0.9;
        let mut preferred = candidate("preferred");
        preferred.candidate_score = 0.2;
        preferred.bonus = 1.0;

        let outcome = rank_candidates(&request(vec![strong, preferred])).expect("slot is valid");

        assert_eq!(outcome.ranked[0].asset_id, "preferred");
        assert_eq!(outcome.ranked[0].verdict, RankVerdict::Accept);
        assert_eq!(
            outcome.ranked[0].breakdown.semantic, 0.2,
            "the bonus must not leak into the auditable breakdown"
        );
    }

    #[test]
    fn caller_bonus_cannot_rescue_a_gate_drop() {
        let mut denies = candidate("denied");
        denies.rights_status = "denied".to_string();
        denies.bonus = 5.0;

        let outcome = rank_candidates(&request(vec![denies])).expect("slot is valid");

        assert!(outcome.ranked.is_empty(), "{outcome:?}");
        assert_eq!(outcome.drops[0].gate.as_deref(), Some("valid_license"));
    }

    #[test]
    fn language_travels_with_the_pool_without_changing_the_score() {
        let base = rank_candidates(&request(vec![candidate("a")])).expect("slot is valid");

        let mut same = candidate("a");
        same.language = "it".to_string();
        let mut same_request = request(vec![same]);
        same_request.scene_language = "it".to_string();
        let same_outcome = rank_candidates(&same_request).expect("slot is valid");

        let mut cross = candidate("a");
        cross.language = "en".to_string();
        let mut cross_request = request(vec![cross]);
        cross_request.scene_language = "it".to_string();
        let cross_outcome = rank_candidates(&cross_request).expect("slot is valid");

        assert_eq!(
            same_outcome.ranked[0].final_score,
            base.ranked[0].final_score
        );
        assert_eq!(
            cross_outcome.ranked[0].final_score,
            base.ranked[0].final_score
        );
        assert_eq!(same_outcome.ranked[0].breakdown, base.ranked[0].breakdown);
        assert_eq!(cross_outcome.ranked[0].breakdown, base.ranked[0].breakdown);
    }

    #[test]
    fn any_slot_disables_the_aspect_gate_only() {
        let mut image = candidate("still");
        image.media_type = "image".to_string();

        let mut any = request(vec![image.clone()]);
        any.slot_kind = "any".to_string();
        let accepted = rank_candidates(&any).expect("any is a valid slot");
        assert_eq!(accepted.ranked.len(), 1, "{accepted:?}");

        let primary = rank_candidates(&request(vec![image])).expect("slot is valid");
        assert!(primary.ranked.is_empty(), "{primary:?}");
        assert_eq!(primary.drops[0].gate.as_deref(), Some("compatible_aspect"));
    }
}
