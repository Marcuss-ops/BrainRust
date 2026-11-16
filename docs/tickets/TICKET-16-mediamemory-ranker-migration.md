# TICKET 16 — MediaMemory ranker migration

- Priority: P1
- Status: PARTIAL (Rust ranker, parity fixtures and Go seam landed 2026-11-16; production call sites pending)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: none; coordinate with TICKET-07 manifest reuse
- Blocks: none

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

Rust owns the ranking decision. The Go call sites have **not** been switched.

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

Gate: `cargo test -p mediasampler` (39 lib + 7 worker + 4 parity),
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

Remaining before the ticket can close:

1. Wire `RankCandidates` into `VisualResolver.resolveScene` and
   `mediaMemoryRankerAdapter.Rank` behind a pool-ranker port, so `Filter`,
   `Score`, `buildFilterFlags`, `buildRankingInput`, `sortByFinalScoreDesc`
   and `PickTopFromRose` lose their production callers. The seam is one
   worker call per slot pool, so the resolver consumes the pool answer
   instead of the per-candidate loop.
2. Port the two brain-adapter-only seats (`ComputeDurationFit`,
   `PopulateRightsPenalty`, and the slot-fitness bonus) or fold them into the
   pool request before deleting the helpers.
3. The pinned `bin/mediasampler` release predates `rank_candidates`; the
   cutover needs a release that ships it.
4. Then delete the Go decision helpers and rerun `make verify-main`.

`normalizer.go` is concept-text normalization (NFC, lowercase, whitespace,
punctuation), not a ranking step: it belongs to the linguistic surface
(TICKET-18) and is deliberately not part of this port.

No production caller was switched, so the ticket stays PARTIAL rather than
DONE.
