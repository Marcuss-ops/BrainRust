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
