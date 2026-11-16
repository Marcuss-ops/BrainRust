# TICKET 15 — Phrase Engine Go demolition

- Priority: P1
- Status: PARTIAL (Rust selector landed 2026-11-16; Go cutover blocked on TICKET-19)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-14 for the overlay consumer; independent for VidRush
  selection
- Blocks: TICKET-19 (alignment)

## Update 2026-11-16 (partial, verified)

The production seam is ready but the cutover is not complete:

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

No production caller was switched and no Go file was deleted, so the ticket
stays PARTIAL rather than DONE.
