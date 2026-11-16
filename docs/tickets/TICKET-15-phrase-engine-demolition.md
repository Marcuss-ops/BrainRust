# TICKET 15 — Phrase Engine Go demolition

- Priority: P1
- Status: OPEN
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-14 for the overlay consumer; independent for VidRush
  selection
- Blocks: TICKET-19 (alignment)

## Why

Go still decides which phrases are editorially important and where they sit in
the certified word timing. That is a second authority next to
`semantic-core::phrase_impact`; both can stay green while the selected phrases
diverge.

Measured at `refactored@dc0ca4e3b` (production files, tests excluded):

| Surface | Files | LOC |
|---|---:|---:|
| `internal/capabilities/scripts/phrases/` + `phrase_*.go` | 8 | 1.846 |
| `scripts/phrase_supplement.go` | 1 | 201 |
| `scripts/phrase_timing.go` | 1 | 340 |

Live call sites:

- `runner_overlay_prepare.go:363` — `supplementSourceImportantPhrases`.
- `vidrush_semantic_chain.go` / `vidrush_semantic_ports.go` — `phrases.Select`
  and `phrases.ContainsProperNamePair`.
- `editorial_overlay.go:342`, `overlay_entity_cards.go:460`,
  `chapter_bullet_overlay.go:194` — phrase timing fallback.

## Target

Rust owns the complete phrase decision:

1. `semantic-engine` returns per-scene `important_phrases` (short and long
   pools, explicit operator hints preserved, entity spans blocked) plus the
   certified word/phrase alignment.
2. Go builds the request and projects the returned spans; it does not run a
   selector, a corpus ranker, a minimum-word repair or an endpoint fallback.
3. `scripts/phrases/`, `phrase_supplement.go` and `phrase_timing.go` are
   deleted in the same commit that switches the callers.

## Acceptance

- New Rust contract test with the same short/long fixtures and operator hints.
- Parity test: for fixed scene text + timing, Go projection equals the Rust
  selection byte-for-byte (order included).
- No production caller of `phrases.Select`, `ImportantPhrasesWithCorpus`,
  `CompleteGroundedPhrase` or `locatePhraseTimingWithEndpointFallback`.
- `go test ./internal/capabilities/scripts/...` and
  `cargo test -p semantic-core -p semantic-engine` pass.
- `make verify-main` on the cutover tree.

## Risks

- Explicit operator phrases have contractual priority; the Rust selector must
  carry them before lane balancing.
- Short/long pools have independent budgets; a single flat list changes the
  overlay mix.
- Timing fallback currently tolerates certified endpoint omissions. Rust must
  return a typed failure or the exact endpoints, never interpolate.
