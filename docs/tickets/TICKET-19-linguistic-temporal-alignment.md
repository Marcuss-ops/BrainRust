# TICKET 19 — Linguistic and temporal alignment

- Priority: P2
- Status: DONE (2026-11-16 phrase timing; entity/moment follow-up closed 2026-11-17)
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
test-only projection helpers (`overlaySceneInput`, `projectEditorialOverlays`);
it has no production caller and no test was weakened. `projectChapterBullets`
was deleted on 2026-11-16, so it can no longer reach the fallback. The entity-occurrence and voiceover-moment follow-up is closed as well: on
2026-11-17 `audio/phrase_locator.go` no longer exists
(`refactored@8f159549b` deleted the last Go word matcher), `audio.PhraseAligner`
is the single production seam, and the voiceover and entity paths build one
`align_phrases` request per segment.

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
- Production caller audit: `overlaySceneInput` and
  `projectEditorialOverlays` have no production caller; `projectChapterBullets`
  was deleted on 2026-11-16. The endpoint fallback is reachable only from the
  surviving test-only projection helpers. The
  phrase-timing/overlay scope therefore passes the "no production caller"
  criterion. The entity/moment follow-up is closed on 2026-11-17:
  `audio.LocatePhrase` and `audio/phrase_locator.go` no longer exist, and no
  production file references them.
- One deployment prerequisite sat outside the code ticket at the time (`v0.3.5`
  had 0 matches for `align_phrases`); closed 2026-11-17 by release `v0.3.6` and
  the `BRAINRUST_VERSION ?= v0.3.6` pin bump.

## Verification 2026-11-17 (follow-up closed)

Run on `BrainRust@527b756` and `refactored@eb65a6a30`.

| Comando | Esito |
|---|---|
| `cargo test -p semantic-core --lib` | **184 passed** |
| `cargo test -p semantic-engine align_phrases --lib` | **3 passed** |
| `cargo test -p semantic-engine --test phrase_selection_protocol --test language_profile_protocol --test localized_identity_protocol` | **13 + 4 + 5 passed** (il worker reale risponde a una richiesta `select_phrases`) |
| `cargo test -p mediasampler --test rank_parity` | **4 passed** (ordine, punteggi e vincitore dei fixture Go) |
| `grep -rn "LocatePhrase\|locatePhraseTimingWithEndpointFallback" --include=*.go internal/ cmd/` sui file non-test | **0 match**: il matcher Go non è più in produzione |
| `go build ./...` e `go vet ./...` | rc 0 |
| `go test ./internal/app/knobs/... ./internal/capabilities/overlays -count=1` | PASS (0,024 s + 11,931 s) |
| `make verify-architecture` | PASS (C2-C 0/0, C2-E 0/0) |

**Deploy chiuso.** La release `v0.3.6` è pubblicata il 2026-11-17 sul tag
`527b756` (run CI `44032711209`, esito `success`, otto asset ICU 72) e contiene
le cinque operazioni: nei binari scaricati dalla release il `semantic-engine`
espone `select_phrases`, `align_phrases`, `lexicon_profile` e
`localize_identities`, il `mediasampler` espone `rank_candidates`, e i loro
SHA-256 coincidono con `BRAINRUST_RELEASE.json`. `refactored@4bde5e66e` pinna
`BRAINRUST_VERSION ?= v0.3.6` e `bin/` contiene i binari della release
(`git_sha=527b756`). Nessun prerequisito di deploy resta aperto.
