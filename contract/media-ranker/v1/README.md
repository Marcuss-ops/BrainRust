# Media ranker contract — v1 (TICKET-16)

`fixtures.json` is **generated** from the Go decision helpers in
`refactored/internal/capabilities/mediamemory`, which remain the oracle until
the production call sites move to Rust. Both sides assert the same file:

- Rust: `mediasampler/tests/rank_parity.rs`
  (`cargo test -p mediasampler --test rank_parity`)
- Go: `internal/capabilities/mediamemory/ranker_rust_fixture_test.go`
  (`go test ./internal/capabilities/mediamemory -run TestMediaRankerRustFixtures`)

Regenerate from the `refactored` checkout (its sibling `BrainRust` worktree
must be present):

```sh
cd refactored
UPDATE_MEDIA_RANKER_FIXTURES=1 go test ./internal/capabilities/mediamemory \
  -run TestMediaRankerRustFixtures -count=1
```

## Wire

Operation `rank_candidates` on the `mediasampler.v1` envelope. Scores must
match bit-for-bit across languages, which is why the workspace enables
`serde_json`'s `float_roundtrip` feature: without it the parser lands 1 ULP off
the decimal literal and `0.48500000000000004` becomes `0.485`.

## Cases

Empty pool, single candidate, repeated source (consecutive penalty), asset-id
tie-break, approved vs pending binding seats, the seven gate drops, duration-fit
edges, cross-channel ties (channel identity must not influence the order), the
negative drop band, a diversity swap and a diversity stay.

Every case fixes order, final score, verdict, breakdown, drops and the winner.
`downrank` is intentionally not covered: with the canonical weights the
cheapest surviving binding scores 0.275 and the cheapest candidate-only row
0.07, both above the 0.05 accept floor, while the 0.30 penalty jumps past the
band into Drop.
