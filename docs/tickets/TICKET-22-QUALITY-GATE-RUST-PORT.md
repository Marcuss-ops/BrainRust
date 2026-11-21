# TICKET-22 — Port the editorial quality gate to Rust

- Priority: P2
- Status: DONE (committed 2026-11-18; released in `v0.3.8` on 2026-11-19). Rust
  `quality_gate` is the only production gate: the Go implementation, the
  opt-in flag and the knob are deleted, and the Go↔Rust differential is
  byte-identical on 27 scenarios. The cutover and the Rust goldens are
  committed (see “Repository state”). TICKET-27 is CLOSED: `v0.3.8` is
  published and `make rust-fetch` installs the released binaries.
- Source: follow-up to the 2026-11-16 architecture audit
- Depends on: TICKET-18 (Rust language profiles)
- Blocks: none

## Why

The editorial quality gate (`qualitygate.Evaluate`) is pure, deterministic and
already covered by 28 Go test functions. It owns language match, source
coverage, date preservation, clip binding, unsupported claims, research
candidate coverage, segment narrative and word tolerance. Keeping it in Go
leaves a second owner of language profiles and script QA rules beside
BrainRust.

## Target

Rust owns the evaluator and exposes it as one `quality_gate` operation on the
persistent `semantic-engine` worker. Go keeps only the policy (status, skip and
`QualityGateError`) and the request projection. Since 2026-11-18 the Rust path
is unconditional: there is no flag and no Go implementation.

## Contract

Request: `{ "operation": "quality_gate", "item", "plan", "result" }`.
Response: `{ "ok": true, "quality": { ... }, "reasons": [...] }`.
`ok` is protocol success; `quality.passed` is the gate verdict. A malformed
request returns `{ "ok": false, "error": "..." }`.

## Decision — 2026-11-17 (superseded by the 2026-11-18 cutover)

The port stays, but it is not adopted by default. The chosen path is an
opt-in cutover in the next release, not a deletion: `quality_gate` exists on
`HEAD` (`d3b77d6`) and not in `v0.3.6`, so the Go gate remains the production
default until `v0.3.7` is published and `BRAINRUST_VERSION` is bumped.

2026-11-17 cutover preparation: `GenerationFinalizer` now accepts a
`script.QualityGateEvaluator`; `SemanticEngineAdapter` implements it through
the `quality_gate` NDJSON operation; `VELOX_RUST_QUALITY_GATE=true` wires the
adapter. The flag defaults to `false`. The local `dist-v0.3.7` bundle passed
the clean fixture end-to-end. TICKET-22 is PARTIAL, not DONE, until the
production default is switched, the pin is bumped and the Go implementation
is deleted.

> **Aggiornamento 2026-11-19 — cutover eseguito.** I tre criteri sono chiusi nel
> working tree e committati: `refactored/make/build.mk` pinna `v0.3.7`
> (`2ca3603`), i quattro binari installati hanno hash verificati, il flag
> `VELOX_RUST_QUALITY_GATE` è stato **cancellato** (nessun default da attivare) e
> il package Go `usecase/qualitygate` (10 file, 866 righe) non esiste più. Il
> differenziale Go↔Rust è 27/27 identico. TICKET-22 è quindi **DONE (bound)**:
> l'unico passo residuo è la pubblicazione del tag, tracciata in
> [TICKET-27](TICKET-27-publish-v0.3.7.md); il paragrafo sopra resta come
> fotografia della preparazione del 2026-11-17.

## Acceptance criteria

- [x] `script-qa` implements every rule: language match, source coverage,
      source date preservation, clip coverage, unsupported claims, research
      candidate coverage, segment narrative, target words.
- [x] The complete Go corpus is ported: 28 test functions plus the `TestMain`
      harness, all green in Rust.
- [x] `semantic-engine` serves `quality_gate` over NDJSON; the protocol test
      covers pass, gate failure and rejected request shapes.
- [x] `cargo fmt --all -- --check` passes.
- [x] `cargo clippy -p script-qa -p semantic-engine --all-targets -- -D warnings`
      passes.
- [x] `cargo test -p script-qa -p semantic-engine` passes.
- [x] `go test ./internal/capabilities/scripts/usecase/qualitygate -count=1`
      still passes.
- [x] `GenerationFinalizer` accepts an injectable `QualityGateEvaluator`;
      the default remains the Go gate when no evaluator is wired.
- [x] `SemanticEngineAdapter` implements `quality_gate` with a typed
      `*QualityGateError` on a failing verdict and a fail-closed error on a
      protocol failure.
- [x] `VELOX_RUST_QUALITY_GATE` is an opt-in config flag (default `false`)
      wired by `wire_script_usecases.go`.
- [x] Unit tests cover the request projection, typed failure, protocol failure
      and injected-finalizer path; the real local `v0.3.7` binary passes the
      clean fixture.
- [x] Switch the production default to Rust, bump the BrainRust pin and delete
      the Go gate (2026-11-18; the flag was removed outright, see the cutover
      section). Publication of `v0.3.7` remains TICKET-27.
- [x] The docs record the exact commands, counts and commit state.

## Cutover — 2026-11-18

The Rust gate is now the only implementation. The flag was deleted instead of
flipped, so no deployment order can leave production calling an operation the
installed worker does not serve.

- **Parity corpus.** 27 scenarios cover all eight rules (language match whole
  text and per scene, source coverage per policy, source dates, clip coverage,
  unsupported claims, research candidates, segment narrative, target words)
  plus both pre-metric hard stops. The canonical Go verdicts were frozen to
  [`script-qa/tests/fixtures/quality_gate/goldens.json`](../../script-qa/tests/fixtures/quality_gate/goldens.json)
  before deletion and are enforced by
  [`script-qa/tests/quality_gate_goldens.rs`](../../script-qa/tests/quality_gate_goldens.rs)
  (3 tests: verdict reproduction, pass/fail reason invariants, unique names).
- **Differential against the real release.**
  `VELOX_RUST_SEMANTIC_ENGINE_PATH=…/dist-v0.3.7/semantic-engine go test
  ./internal/platform/media/rustexec -run TestQualityGateGoRustParity` —
  **27/27 PASS**, metrics and reason strings identical. The differential test
  was removed with the Go gate; the frozen goldens keep enforcing the behavior.
- **Pin.** `BRAINRUST_VERSION ?= v0.3.7`; `bin/BRAINRUST_RELEASE.json` =
  `v0.3.7`, `git_sha 2ca3603`, ICU 72; all four binary SHA-256 digests match the
  release manifest. `TestSemanticEngineRealBinaryEvaluatesQualityGate` passes
  against the installed pin (no skip).
- **Go deletion.** `internal/capabilities/scripts/usecase/qualitygate/`
  (10 production files, 866 lines), `VELOX_RUST_QUALITY_GATE`, the config field,
  the knob entry and the generated artifacts are gone. `go build ./...` and
  `go vet` on the touched packages are green; the finalizer fails closed with
  `quality gate evaluator is not configured` when the port is missing, covered
  by `TestGenerationFinalizerFailsClosedWithoutQualityGate`.
- **Publication.** `git ls-remote --tags origin v0.3.7` returns nothing on
  2026-11-18: the tag and bundle exist only locally. TICKET-27 tracks the
  publish; until then an offline install uses
  `make rust-fetch RUST_ARTIFACTS_DIR=<BrainRust>/dist-v0.3.7`.

## Verification — 2026-11-17 (opt-in era)

Run on the `BrainRust` working tree; the Go gate ran on `refactored`. No Go
production path was changed.

- `cargo fmt --all -- --check` — **rc 0**.
- `cargo clippy -p script-qa -p semantic-engine --all-targets -- -D warnings` —
  **rc 0**.
- `cargo test -p script-qa` — **28 passed, 0 failed** (parity corpus);
  doc-tests 0.
- `cargo test -p semantic-engine --test quality_gate_protocol` — **3 passed**
  (clean pass, empty-text failure, malformed request).
- `cargo test -p script-qa -p semantic-engine` — **29 suites, 240 passed,
  0 failed**; the run includes the 28 parity tests and 3 protocol tests.
- `go test ./internal/capabilities/scripts/usecase/qualitygate -count=1` —
  **ok, 0.022 s**.
- `git diff --check` in the root and in `BrainRust` — **rc 0**.
- Commit state: `d3b77d6` (local). The earlier "uncommitted working tree"
  note is superseded by that commit; the tag `v0.3.6` predates it.

### Opt-in cutover verification — 2026-11-17

Local bundle: `BrainRust/dist-v0.3.7/semantic-engine` (tag `v0.3.7`, commit
`2ca3603`). The clean English fixture returns `quality.passed=true`.

- `VELOX_RUST_SEMANTIC_ENGINE_PATH=$PWD/../BrainRust/dist-v0.3.7/semantic-engine
  go test ./internal/platform/media/rustexec -run
  '^TestSemanticEngineRealBinaryEvaluatesQualityGate$' -count=1` — **PASS**.
- `go test ./internal/platform/media/rustexec -run
  'TestSemanticEngineQualityGate' -count=1` — **ok**.
- `go test ./internal/capabilities/scripts/usecase/gencore -run
  'TestGenerationFinalizer(UsesInjectedQualityGate|InjectedGateFailureIsTyped)'
  -count=1` — **ok**.
- `go test ./internal/app/wiring -run '^$' -count=1` — **ok** (wiring compiles).
- `go vet ./internal/platform/media/rustexec
  ./internal/capabilities/scripts/usecase/gencore ./internal/app/wiring` —
  **rc 0**.

Operator opt-in (default remains Go):

```sh
VELOX_RUST_QUALITY_GATE=true \
VELOX_RUST_SEMANTIC_ENGINE_PATH=/path/to/BrainRust/dist-v0.3.7/semantic-engine \
  <pipelinegen process>
```

## Repository state — 2026-11-18

The cutover is committed in this change (2026-11-18):

- `refactored` tracks the deletion of
  `internal/capabilities/scripts/usecase/qualitygate/` (14 files: 10
  production files, 866 lines, plus tests) and pins
  `BRAINRUST_VERSION ?= v0.3.7`. `bin/` is a local, untracked install; after
  `make rust-fetch RUST_ARTIFACTS_DIR=…/BrainRust/dist-v0.3.7` its manifest is
  `version: v0.3.7`, `git_sha: 2ca3603…`, and all four SHA-256 digests match
  `dist-v0.3.7/sha256sums.txt`.
- `BrainRust` tracks `script-qa/tests/quality_gate_goldens.rs` and
  `script-qa/tests/fixtures/`.
- Gates rerun from this tree on 2026-11-18: `cargo fmt --all -- --check` rc 0;
  `cargo clippy -p script-qa -p semantic-engine --all-targets -- -D warnings`
  rc 0; `cargo test -p script-qa -p semantic-engine` rc 0 (all suites,
  doctests included); `go build ./...` rc 0; `go vet ./...` rc 0;
  `go test ./internal/capabilities/scripts/...` rc 0;
  `TestSemanticEngineRealBinaryEvaluatesQualityGate` PASS on the restored
  release binary.
- The earlier doctest `E0463` and clippy `E0425` were not reproducible; after
  formatting `semantic-core/src/lexicon.rs` and
  `visualner/src/extractor/values.rs`, both gates are green above.

A clean checkout now contains the Go gate removal, the Rust goldens and the
`v0.3.7` pin. Publishing `v0.3.7` remains TICKET-27 because this workspace has
no GitHub credentials.

## Non-goals (pre-cutover, superseded 2026-11-18)

The 2026-11-17 opt-in plan did not change the production default, delete the
Go gate or touch SQLite/outbox/Drive/provider/prompt ownership. The
2026-11-18 cutover supersedes the first two points; the ownership boundary
still holds.
