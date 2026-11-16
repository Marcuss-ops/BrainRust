# TICKET 19 — Linguistic and temporal alignment

- Priority: P2
- Status: OPEN
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-15 (phrase selection)
- Blocks: TICKET-18 / TICKET-20

## Why

Go still matches phrases to certified word timing:

- `internal/capabilities/audio/phrase_locator.go` and its tests implement
  `LocatePhrase` / `LocatePhraseTimings`.
- `scripts/phrase_timing.go` (340 LOC) adds a first/last-word endpoint fallback
  and builds the flat phrase/scene timing projections.
- `editorial_overlay.go`, `overlay_entity_cards.go` and
  `chapter_bullet_overlay.go` call the fallback at render-plan preparation.

The Rust worker returns certified word timing, but the alignment decision is
still Go. Ticket 13c closed duration authority, not alignment.

## Target

Rust returns the alignment:

```text
certified word timing + phrases
        -> align_phrases (Rust)
        -> exact word/phrase spans, global and local timing
```

Go projects the result and fails closed when a phrase is absent. No endpoint
fallback, no interpolation, no second word matcher.

## Acceptance

- Rust alignment operation with fixtures for repeated words, missing interior
  words, punctuation and multilingual text.
- Golden parity against the current `LocatePhraseTimings` for every accepted
  phrase; rejected phrases return a typed error.
- `phrase_locator.go` and `locatePhraseTimingWithEndpointFallback` have no
  production caller.
- `go test ./internal/capabilities/audio/... ./internal/capabilities/scripts/...`
  and `make verify-main` pass.

## Risks

- The current fallback accepts endpoint-only matches; parity must preserve the
  accepted set exactly or document a deliberate tightening.
- Global timing is relative to the scene timeline; the Rust response must carry
  both coordinates to avoid a Go re-derivation.
