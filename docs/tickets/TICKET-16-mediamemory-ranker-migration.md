# TICKET 16 — MediaMemory ranker migration

- Priority: P1
- Status: OPEN
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
