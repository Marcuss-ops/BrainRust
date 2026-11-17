# TICKET 16 — MediaMemory ranker migration

- Priority: P1
- Status: DONE (2026-11-16, verified on the cutover tree)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: none; coordinate with TICKET-07 manifest reuse
- Blocks: none

## Closeout 2026-11-16

- `mediasampler::rank` owns the seven gates, weights, verdict bands,
  repetition/channel penalties and the diversity-aware winner.
- Go `mediamemory` projects one slot pool through `PoolRanker`; `pool_ranker.go`
  has no Go scoring or re-sort, and `PickTopFromRose` /
  `PopulateRepetitionPenalty` have no production caller.
- `rank_parity.rs` reproduces the Go order, scores and winner for every frozen
  fixture; `ranker_rust_fixture_test.go` pins the same contract on the Go side.
- Verified: `go test ./internal/capabilities/mediamemory/...`; `cargo test -p
  mediasampler` (42 + 7 + 4 tests).

## Why

`internal/capabilities/mediamemory/` still contains 29 production files
(5.069 LOC) that decide the final asset order. `PickTopFromRose`
(`ranker_repetition.go:274`) applies repetition penalties, channel recency and
diversity directly in Go; the Rust `mediasampler` does not see the full
candidate set or the usage history.

## Target

`mediasampler` owns ranking:

```text
candidates + usage history + channel policy
        -> rank_media_candidates (one Rust call)
        -> ordered selection + per-candidate score breakdown
```

Go keeps persistence, provider I/O and projection. It does not re-rank, re-gate
or apply a second diversity rule.

## Acceptance

- Rust request/response contract with normalizer, weights, gates and
  repetition penalty moved from `ranker*.go`, `resolver_scoring.go`,
  `normalizer.go`.
- Fixed-candidate parity fixtures: identical order and scores for empty,
  repeated, recency-penalized and cross-channel sets.
- `PickTopFromRose` and the other decision helpers have no production caller.
- `go test ./internal/capabilities/mediamemory/...` and the Rust mediasampler
  suite pass; `make verify-main` on the cutover tree.

## Risks

- The current ranking is deterministic but depends on map iteration order in
  places; the Rust port must define a stable tie-break.
- Usage history is large; send only the fields the ranker reads and keep the
  payload bounded.

## Progress 2026-11-16

Rust owns the ranking decision. The Go call sites were switched in the 2026-11-16 cutover; the notes below record the port before that cutover.

Landed:

- `mediasampler/src/rank.rs` ports the whole decision: gate flags, the seven
  mandatory gates in canonical order, the ranking-input projection, both
  duration-fit curves, the resolver and ranker rights seats, the
  consecutive-source penalty with its 1.5 ceiling, the canonical weighted
  formula (same association order, so the `f64` is bit-identical), the verdict
  bands, stable ordering (FinalScore DESC, AssetID ASC) and the
  diversity-aware pick (delta 0.10).
- `rank_candidates` is a `mediasampler.v1` worker operation; an unknown slot
  fails closed with a typed error.
- `contract/media-ranker/v1/fixtures.json` — 11 cases GENERATED from the Go
  helpers (empty pool, single candidate, repeated source, asset-id tie-break,
  approved vs pending binding seats, all seven gate drops, duration-fit edges,
  cross-channel ties, the drop band, a diversity swap and a diversity stay).
  Both sides assert the same file: `mediasampler/tests/rank_parity.rs` and
  `mediamemory/ranker_rust_fixture_test.go`. Order, scores, breakdowns, drops
  and winners are identical.
- Go seam: `rustexec.MediaRankerAdapter.RankCandidates` sends one
  `rank_candidates` call and fails closed on an unknown slot, a worker error,
  a non-finite score, a broken order, a winner outside the selection, or a
  candidate that is neither ranked nor dropped. 7 unit tests, plus an opt-in
  real-worker test that reproduces the Go 0.625/0.625 diversity swap.

Gate: `cargo test -p mediasampler` (42 lib + 7 worker + 4 parity),
`cargo clippy --workspace --all-targets -- -D warnings` and
`go test ./internal/capabilities/mediamemory/... ./internal/platform/media/rustexec/...`
pass.

Two findings from the port:

- `downrank` is unreachable with the canonical weights. The cheapest
  surviving binding scores 0.275 and the cheapest candidate-only row 0.07,
  both above the 0.05 accept floor, while the 0.30 consecutive penalty jumps
  past the band straight into Drop. The vocabulary stays for a future
  re-weighting; the parity test pins that reasoning.
- `serde_json` without `float_roundtrip` parses 1 ULP off the decimal literal,
  turning `0.48500000000000004` into `0.485`. The workspace dependency now
  enables exact parsing, which any cross-language score equality needs.

## Closed residuals

The earlier blockers were resolved on the cutover tree:

- `RankCandidates` is wired through `internal/app/wiring/media_pool_ranker.go`;
  `PickTopFromRose`, `PopulateRepetitionPenalty`, `buildFilterFlags`,
  `buildRankingInput` and `sortByFinalScoreDesc` have **0 production callers**.
  The resolver projects the Rust pool answer instead of re-scoring it.
- The Go decision helpers remain only as parity/test oracles; no path re-ranks,
  re-gates or applies a second diversity rule.
- `normalizer.go` is concept-text normalization (NFC, lowercase, whitespace,
  punctuation), not a ranking step: it belongs to the linguistic surface
  (TICKET-18) and is deliberately not part of this port.
- `v0.3.5` predated `rank_candidates`; closed 2026-11-17 with release `v0.3.6`
  (tag `527b756`; the release `mediasampler` has SHA-256
  `86756ce95f8c6f90a2df1c8f8f3581a53d807ddcb230b7b414e5d1205404618d`) and the
  pin `BRAINRUST_VERSION ?= v0.3.6` in `refactored@4bde5e66e`. `bin/mediasampler`
  is now that release binary, verified against the production bridge.

## Cutover 2026-11-16 — the production path

- `mediamemory.PoolRanker` (`pool_ranker.go`) is the only ranking seam. The
  resolver builds one pool per slot and maps the answer back; `rankSlotPool`
  fails closed on a missing ranker, a worker error, a key outside the pool or
  survivors without a winner. `layerFromRanked` composes the Layer.
- `WireMediaMemoryResolver` builds the bridge
  (`internal/app/wiring/media_pool_ranker.go`) once and shares it with the
  cascade and the Brain adapter, so both consumers see one authority.
- The Brain ranks before slots are assigned: it sends `slot_kind: "any"` (the
  neutral slot, aspect gate disabled) and its slot-fitness preference as the
  per-candidate `bonus`, which the ranker adds **after** the verdict — it can
  reorder survivors, never rescue a drop. Both are additive wire fields, so the
  fixture contract is unchanged.
- The frozen Go implementation moved into `_test.go` files
  (`ranker_oracle_test.go`, `ranker_oracle_weights_test.go`,
  `ranker_oracle_gates_test.go`, `ranker_oracle_repetition_test.go`,
  `resolver_scoring_oracle_test.go`, `pool_ranker_oracle_test.go`). Production
  cannot link it even by accident; the oracle's `PoolRanker` implementation is
  what the resolver tests inject.
- Pool identity: binding-derived candidates carry no `MediaCandidate.ID`, so the
  caller sends an opaque, unique pool key (`<ordinal>|<candidate_id>`) and the
  ranker echoes it. The key is never parsed; `asset_id` stays the real one.
- End-to-end against the rebuilt worker:
  `VELOX_RUST_MEDIASAMPLER_PATH=$PWD/bin/mediasampler go test ./internal/app/wiring -run TestMediaPoolRankerBridge_RealWorker`
  -> PASS (approved binding wins, `0.01` bonus breaks a tie, denied rights
  dropped with `valid_license`).

## Verification 2026-11-16 (current tree)

- `mediasampler::rank`, the `rank_candidates` operation, the 11-case parity
  fixture contract and the bonus/neutral-slot additions are in this tree.
- `cargo test -p mediasampler` -> **42 lib + 7 worker + 4 parity PASS**.
- `go vet ./internal/app/wiring/... ./internal/capabilities/mediamemory/...
  ./internal/platform/media/rustexec/...` -> clean; `go build ./...` -> clean.
- `go test ./internal/capabilities/mediamemory/... ./internal/capabilities/brain/...
  ./internal/platform/media/rustexec/...` -> PASS (mediamemory 0.059 s,
  rustexec 86.500 s).
- `go test ./internal/app/wiring` -> PASS except
  `TestFinalJobCertifiedFragmentsShareOneDecoderConfiguration`, which fails on
  committed `data/final-job-fragments` avcC bytes (pre-existing, no file in
  this change touches it).
- Production caller check: `PickTopFromRose`, `PopulateRepetitionPenalty`,
  `buildFilterFlags`, `buildRankingInput` and `sortByFinalScoreDesc` return 0
  matches outside `_test.go`.
- Residual external only, closed 2026-11-17: release `v0.3.6` contains
  `rank_candidates` and the pin is `BRAINRUST_VERSION ?= v0.3.6`.
