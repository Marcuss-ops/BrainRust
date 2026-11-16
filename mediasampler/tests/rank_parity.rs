//! Cross-repo parity gate for the media ranker (TICKET-16).
//!
//! `contract/media-ranker/v1/fixtures.json` is GENERATED from PipelineGen's
//! `internal/capabilities/mediamemory` package (the Go decision helpers are the
//! oracle) and asserted here. Every fixture fixes candidate order, final
//! scores, verdicts, sizes and the diversity pick, so a drift on either side
//! fails instead of silently changing the winner.
//!
//! Regenerate from the Go side:
//!
//! ```sh
//! cd refactored
//! UPDATE_MEDIA_RANKER_FIXTURES=1 go test ./internal/capabilities/mediamemory \
//!   -run TestMediaRankerRustFixtures -count=1
//! ```

use std::path::PathBuf;

use mediasampler::rank::{rank_candidates, RankRequest, RankVerdict};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FixtureFile {
    schema_version: String,
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    name: String,
    input: RankRequest,
    expected: FixtureExpected,
}

#[derive(Debug, Deserialize)]
struct FixtureExpected {
    ranked: Vec<FixtureRanked>,
    drops: Vec<FixtureDrop>,
    #[serde(default)]
    winner_asset_id: String,
}

#[derive(Debug, Deserialize)]
struct FixtureRanked {
    asset_id: String,
    candidate_id: String,
    source_url: String,
    provider: String,
    media_type: String,
    duration_ms: i64,
    rights_status: String,
    final_score: f64,
    verdict: String,
    reason: String,
    breakdown: FixtureBreakdown,
}

#[derive(Debug, Deserialize)]
struct FixtureBreakdown {
    semantic: f64,
    exact_match: f64,
    visual: f64,
    manual_approval: f64,
    quality: f64,
    historical_success: f64,
    duration_fit: f64,
    repetition_penalty: f64,
    rights_penalty: f64,
}

#[derive(Debug, Deserialize)]
struct FixtureDrop {
    asset_id: String,
    candidate_id: String,
    #[serde(default)]
    gate: String,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("contract")
        .join("media-ranker")
        .join("v1")
        .join("fixtures.json")
}

fn load_fixtures() -> FixtureFile {
    let path = fixture_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("decode {}: {error}", path.display()))
}

#[test]
fn the_fixture_file_is_the_expected_contract_version() {
    let file = load_fixtures();
    assert_eq!(file.schema_version, "media-ranker.v1");
    assert!(
        file.cases.len() >= 11,
        "the parity battery must keep the empty, repeated, tie-break, gate, duration, cross-channel, drop-band and diversity cases (found {})",
        file.cases.len()
    );
}

#[test]
fn every_fixture_reproduces_the_go_order_scores_and_winner() {
    let file = load_fixtures();
    for case in &file.cases {
        let outcome = rank_candidates(&case.input)
            .unwrap_or_else(|error| panic!("case {:?}: {error:?}", case.name));

        assert_eq!(
            outcome.ranked.len(),
            case.expected.ranked.len(),
            "case {:?}: ranked length",
            case.name
        );
        for (index, (got, want)) in outcome
            .ranked
            .iter()
            .zip(case.expected.ranked.iter())
            .enumerate()
        {
            assert_eq!(
                got.asset_id, want.asset_id,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(
                got.candidate_id, want.candidate_id,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(
                got.source_url, want.source_url,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(
                got.provider, want.provider,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(
                got.media_type, want.media_type,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(
                got.duration_ms, want.duration_ms,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(
                got.rights_status, want.rights_status,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(
                got.final_score, want.final_score,
                "case {:?} rank {index}: scores must be identical, not merely ordered",
                case.name
            );
            assert_eq!(
                got.verdict.as_str(),
                want.verdict,
                "case {:?} rank {index}",
                case.name
            );
            assert_eq!(got.reason, want.reason, "case {:?} rank {index}", case.name);

            let seats = (
                got.breakdown.semantic,
                got.breakdown.exact_match,
                got.breakdown.visual,
                got.breakdown.manual_approval,
                got.breakdown.quality,
                got.breakdown.historical_success,
                got.breakdown.duration_fit,
                got.breakdown.repetition_penalty,
                got.breakdown.rights_penalty,
            );
            let expected_seats = (
                want.breakdown.semantic,
                want.breakdown.exact_match,
                want.breakdown.visual,
                want.breakdown.manual_approval,
                want.breakdown.quality,
                want.breakdown.historical_success,
                want.breakdown.duration_fit,
                want.breakdown.repetition_penalty,
                want.breakdown.rights_penalty,
            );
            assert_eq!(
                seats, expected_seats,
                "case {:?} rank {index}: score breakdown",
                case.name
            );
        }

        assert_eq!(
            outcome.drops.len(),
            case.expected.drops.len(),
            "case {:?}: drops",
            case.name
        );
        for (got, want) in outcome.drops.iter().zip(case.expected.drops.iter()) {
            assert_eq!(got.asset_id, want.asset_id, "case {:?} drop", case.name);
            assert_eq!(
                got.candidate_id, want.candidate_id,
                "case {:?} drop",
                case.name
            );
            if !want.gate.is_empty() {
                assert_eq!(
                    got.gate.as_deref(),
                    Some(want.gate.as_str()),
                    "case {:?} drop gate",
                    case.name
                );
            }
        }

        assert_eq!(
            outcome
                .winner
                .as_ref()
                .map(|winner| winner.asset_id.as_str())
                .unwrap_or(""),
            case.expected.winner_asset_id,
            "case {:?}: winner",
            case.name
        );
    }
}

#[test]
fn verdict_vocabulary_matches_the_fixture() {
    let file = load_fixtures();
    let mut seen = std::collections::BTreeSet::new();
    for case in &file.cases {
        for entry in &case.expected.ranked {
            seen.insert(entry.verdict.clone());
        }
    }
    assert!(
        seen.contains("accept"),
        "the fixture battery must keep covering the accept band"
    );
    let drop_case = file
        .cases
        .iter()
        .find(|case| case.name == "drop_band_negative_score")
        .expect("drop-band fixture");
    assert!(
        drop_case.expected.ranked.is_empty() && drop_case.expected.drops.len() == 1,
        "the drop band must drop the candidate, not downrank it"
    );
    assert_eq!(RankVerdict::Drop.as_str(), "drop");
}

/// The downrank band is `0.0 ..= 0.05`. With the canonical weights every
/// surviving candidate clears it: the cheapest binding seat pair is
/// `exact_match*0.20 + visual*0.15 = 0.275`, and the cheapest candidate-only
/// pair is `quality*0.10 + historical_success*0.05 = 0.07`. The consecutive
/// penalty (0.30) jumps straight past the band into Drop, so `downrank` is
/// vocabulary reserved for a re-weighting, not a reachable outcome today.
#[test]
fn the_downrank_band_is_unreachable_with_the_canonical_weights() {
    use mediasampler::rank::{classify_score, default_verdict_thresholds, RankVerdict};

    let thresholds = default_verdict_thresholds();
    let cheapest_binding = 1.0 * 0.20 + 0.5 * 0.15;
    let cheapest_candidate_only = 0.5 * 0.10 + 0.4 * 0.05;
    assert!(cheapest_binding > thresholds.accept);
    assert!(cheapest_candidate_only > thresholds.accept);
    assert_eq!(classify_score(0.04, thresholds), RankVerdict::Downrank);
    assert_eq!(classify_score(-0.01, thresholds), RankVerdict::Drop);
}
