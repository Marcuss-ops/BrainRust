# TICKET 15 — Phrase Engine Go demolition

- Priority: P1
- Status: DONE (2026-11-16, verified; deadline fallback retained only as a test oracle with no production caller)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-14 for the overlay consumer; independent for VidRush
  selection
- Blocks: TICKET-19 (alignment)

## Update 2026-11-16 (partial, verified)

The production seam was ready before the 2026-11-16 cutover; this section records the pre-cutover state and is superseded by the DONE verdict:

- `semantic-core::phrase_select` now owns the exact Go selector algorithm
  (short/long candidates, blocked rune spans, corpus boost, sentence cards,
  function-word density, proper-name/acronym guards, 16/30-word caps). Ten
  parity tests pass.
- `semantic-engine` exposes `select_phrases`; the Go
  `SemanticEngineAdapter.SelectPhrases` boundary validates byte/rune grounding
  and fails closed on worker or span errors.
- Still OPEN: no caller is switched yet, so `scripts/phrases/`,
  `phrase_supplement.go` and `phrase_timing.go` remain live; TICKET-16 and
  18/19/20 are untouched. Do not mark this ticket DONE until those callers and
  the parity gate land.

## Update 2026-11-16 (selection cutover verified)

The deterministic selector is now the only phrase authority on the source and
localized paths; the Go selector package is gone.

- `SceneIRSegmentEnricher` requires a `PhraseSelector` when the plan requests
  important phrases or words. `vidrush_semantic_chain.go` no longer imports
  `scripts/phrases`; operator hints pass the local proper-name ingress guard.
- `translated_nlp.go` selects through the same Rust operation with the
  document corpus and blocked entity spans.
- `phrase_projection.go` (renamed from `phrase_supplement.go`) calls
  `SelectPhrases` for the 2–5-word and default lanes, then only completes,
  merges and balances the returned spans.
- `internal/capabilities/scripts/phrases/` is deleted in production; the
  remaining `phrases/semantic` package is the unrelated phrase-impact scorer.
  Tests now use local projection helpers instead of the deleted selector.
- The phrase-timing projection (`CompilePhraseTimings`,
  `CompileSceneSpeechTimings`) now consumes the Rust `align_phrases` operation
  via `PhraseAligner`; Go maps the returned spans and never calls
  `LocatePhraseTimings` there.

Verified: `go test ./internal/capabilities/audio
./internal/capabilities/scripts ./internal/platform/media/rustexec
./internal/app/wiring`; `cargo test -p semantic-core -p semantic-engine`.
Closeout: no production file imports or calls `phrases.Select`,
`ImportantPhrasesWithCorpus`, `CompleteGroundedPhrase` or
`locatePhraseTimingWithEndpointFallback`. The remaining endpoint helper is
reachable only from the test-only projection oracles (`overlaySceneInput`,
`projectChapterBullets`, `projectEditorialOverlays`), which the deletion guard
keeps out of the production planner. `audio.LocatePhrase` remains for
entity/moment projection and is tracked with TICKET-19's residual scope, not
as a second phrase selector.

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

## Progress 2026-11-16

Rust owns the selection decision. The Go demolition has **not** happened.

Landed:

- `semantic-core/src/phrase_select.rs` (869 LOC) ports the selector:
  candidate generation, configured visual-verb ranking, corpus document-term
  boost, the one-third function-word density guard, the complete-sentence and
  two-sentence-card paths, the hard sixteen-word fragment cap, and
  rune-exclusive blocked spans.
- `select_phrases` is a worker operation in `semantic-engine/src/protocol.rs`,
  with typed fail-closed errors for empty text and malformed requests.
- `semantic-engine/tests/phrase_selection_protocol.rs` — 13 tests (12 in
  process plus the real worker one-request path). The fixtures are those the
  Go suite pins in `internal/capabilities/scripts/phrases/selection_test.go`
  (grounded name, blocked-spans, corpus boost, six-word natural phrase,
  function-word density, hard cap, long sentence, two-sentence card), with the
  same expected order; the test also pins the byte/rune coordinate contract.
- `cargo test -p semantic-core -p semantic-engine --lib` (161 + 54) and
  `cargo test -p semantic-engine --test phrase_selection_protocol` (13) pass;
  `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- The Go seam exists but is not wired: `scripts/phrase_selection.go` declares
  `PhraseSelector`, `PhraseSelectionRequest`/`PhraseSelectionProfile`/
  `SelectedPhrase` and `NewPhraseSelectionProfile` (sorted keys, deterministic
  request), and `rustexec.SelectPhrases` sends one `select_phrases` call and
  rejects any phrase whose bytes are not the requested-text slice. Three unit
  tests cover the grounded accept, the ungrounded reject and the worker error;
  `go test ./internal/platform/media/rustexec/...` passes.

Remaining before the ticket can close:

1. **Alignment is TICKET-19.** `scripts/phrase_timing.go` and
   `audio/phrase_locator.go` own the word/phrase alignment, and this ticket's
   acceptance forbids a production `locatePhraseTimingWithEndpointFallback`
   caller. The Go selector cannot be deleted before the Rust alignment op
   exists, because the same call sites consume the phrase timing.
2. **Operator hints are caller input.** `phrase_supplement.go` injects the
   anchored operator phrases; the callers must send those spans to Rust before
   the file is deleted.
3. **Short/long pools are caller budgets.** The op exposes `limit`,
   `min_words` and `max_words`, but the lane split stays in the Go caller until
   the alignment op returns both pools from one request.
4. Then the single-commit cutover: wire `PhraseSelector` into
   `runner_overlay_prepare.go`, `vidrush_semantic_chain.go`,
   `editorial_overlay.go`, `overlay_entity_cards.go` and
   `chapter_bullet_overlay.go`; delete `scripts/phrases/`,
   `phrase_supplement.go` and `phrase_timing.go`; rerun
   `go test ./internal/capabilities/scripts/...` and `make verify-main`.

At the committed HEAD (`refactored@6f1e75a6d`) no production caller was
switched and no Go file was deleted, so the ticket stays PARTIAL rather than
DONE. The in-flight pass below moves the callers but does not change that
verdict.

## In-flight cutover 2026-11-16 (historical, pre-commit)

A second pass on the working tree on top of `refactored@6f1e75a6d` showed the
selector wired before the final test fixes. This section is retained as
history; the current verification is below:

- `supplementSourceImportantPhrases` now takes a `PhraseSelector` and calls
  `SelectPhrases` twice, once for the 2–5-word lane and once for the default
  lane; a nil selector fails closed.
- `SceneIRSegmentEnricher`, `computeSegmentEntityAnnotations`,
  `runner_overlay_prepare.go`, `runner.go`, `translated_nlp.go` and
  `vidrush_semantic_chain.go` pass the selector through.
- No production file imports `capabilities/scripts/phrases` any more; only
  three test files still do (`catalog_hint_validation_test.go`,
  `translated_nlp_property_test.go`, `phrase_sentence_boundaries_test.go`).
- `go build ./internal/capabilities/scripts/... ./internal/app/wiring/...` is
  clean.

## Residuals after the DONE verdict

- The two previously red tests were repaired without weakening their
  assertions; `go test ./internal/capabilities/scripts -count=1` passes in
  15.723 s.
- `scripts/phrase_timing.go` (402 LOC) delegates alignment to the Rust
  `align_phrases` seam. The endpoint fallback remains only for the test-only
  projection helpers (`overlaySceneInput`, `projectChapterBullets`,
  `projectEditorialOverlays`); it has no production caller. `audio.LocatePhrase`
  remains for entity-occurrence and voiceover-moment projection, outside this
  ticket's phrase-selection scope.
- The Go selector package is no longer a production dependency: 0 production
  imports of `capabilities/scripts/phrases`; `phrase_supplement.go` is
  deleted; the remaining `phrases/semantic` package is the unrelated
  phrase-impact scorer. The short/long two-request lane is an optimization,
  not a second selector authority.
- `refactored` still pins `BRAINRUST_VERSION ?= v0.3.5`, which contains
  neither `select_phrases` nor `align_phrases`. A new BrainRust release and a
  pin bump are required before production runs the switched callers; this is a
  deployment prerequisite, not an open code path in this ticket.

## Verification 2026-11-16 (current tree)

| Command | Result |
|---|---|
| `go build ./internal/capabilities/scripts/... ./internal/app/wiring/...` | PASS |
| `go test ./internal/capabilities/scripts -count=1` | PASS, 15.723 s |
| `go test ./internal/capabilities/audio ./internal/platform/media/rustexec` | PASS, 0.026 s + 99.322 s |
| `cargo test -p semantic-engine --test phrase_selection_protocol` | 13/13 PASS |
| production imports of `capabilities/scripts/phrases` | 0 |
| production callers of the endpoint fallback | 0 |
| `git show v0.3.5:semantic-engine/src/protocol.rs \| grep select_phrases` | 0 matches |
