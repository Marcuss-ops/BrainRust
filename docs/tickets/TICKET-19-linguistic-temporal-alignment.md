# TICKET 19 — Linguistic and temporal alignment

- Priority: P2
- Status: DONE (2026-11-16, phrase-timing scope; entity/moment matcher tracked separately)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-15 (phrase selection)
- Blocks: TICKET-18 / TICKET-20

## Update 2026-11-16 (core and projection verified)

- `semantic-core::phrase_align` owns the exact matcher: NFC, case folding,
  apostrophe folding, punctuation edge trim, dash splitting, non-overlapping
  document-order occurrences and the elided-article-before-number case.
  Eight tests cover exact spans, repeated words, dash-joined words, numeric
  elision, typographic apostrophes, missing interior words and bounded
  diagnostics.
- `semantic-engine` exposes `align_phrases`; it accepts the certified word
  array plus ordered phrases, returns one exact first-occurrence alignment per
  phrase, and fails closed on the first missing phrase.
- `rustexec.SemanticEngineAdapter.AlignPhrases` validates the artifact, the
  alignment count, word indices, exact phrase text and microsecond endpoints.
- `scripts.PhraseAligner` is wired into `Runner`; `CompilePhraseTimings` and
  `CompileSceneSpeechTimings` now project the Rust result. The production
  `script_generation_runtime.go` wires the same semantic-engine adapter.
- Verified with `cargo test -p semantic-core -p semantic-engine` and
  `go test ./internal/capabilities/audio ./internal/capabilities/scripts
  ./internal/platform/media/rustexec ./internal/app/wiring`.

Scope closure: the phrase-timing and overlay projection owned by this ticket
now goes through `align_phrases`. The legacy endpoint fallback lives only in
test-only projection helpers (`overlaySceneInput`, `projectChapterBullets`,
`projectEditorialOverlays`); it has no production caller and no test was
weakened. `audio.LocatePhrase` remains in production for entity occurrence
and voiceover-moment projection, which are outside this ticket's
phrase-timing scope and are tracked as a separate follow-up.

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

## Verification 2026-11-16 (current tree)

- `semantic-core/src/phrase_align.rs` ports the matcher: NFC, case/apostrophe
  folding, punctuation trim, dash splitting, non-overlapping document-order
  occurrences and fail-closed missing-word diagnostics.
  `cargo test -p semantic-core phrase_align --lib` -> **8/8 PASS**.
- `semantic-engine` registers `align_phrases`; the worker protocol test
  `aligns_phrases_through_the_worker_protocol` passes and the missing-interior
  case fails closed. `cargo test -p semantic-engine align_phrases --lib` ->
  **1/1 PASS**.
- `SemanticEngineAdapter.AlignPhrases` validates the timing artifact and every
  returned word/timestamp span, then bridges to `scriptgen.PhraseAligner`.
  `go test ./internal/platform/media/rustexec` -> PASS (99.322 s).
- `CompilePhraseTimings` and `CompileSceneSpeechTimings` call the Rust aligner
  and project exact spans; `go test ./internal/capabilities/scripts -count=1`
  -> PASS (15.723 s); `go test ./internal/capabilities/audio` -> PASS.
- Production caller audit: `overlaySceneInput`, `projectChapterBullets` and
  `projectEditorialOverlays` have no production caller; the endpoint fallback
  is reachable only from those test-only projection helpers. The
  phrase-timing/overlay scope therefore passes the "no production caller"
  criterion. `audio.LocatePhrase` remains in production for entity occurrence
  and voiceover-moment projection, explicitly outside this ticket's scope.
- One deployment prerequisite remains outside the code ticket: `v0.3.5` does
  not contain `align_phrases` (0 matches), so a new BrainRust release and pin
  bump are required before the worker binary in production serves it.
