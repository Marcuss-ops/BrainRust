# TICKET-22 — Port the editorial quality gate to Rust

- Priority: P2
- Status: PARTIAL (2026-11-17) — Rust evaluator ported and the opt-in cutover
  is implemented behind `VELOX_RUST_QUALITY_GATE`; the Go gate remains the
  production default until `v0.3.7` is published and the pin is bumped
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
persistent `semantic-engine` worker. Go keeps the policy (status, skip and
`QualityGateError`) and the production default. The cutover is opt-in behind
`VELOX_RUST_QUALITY_GATE`; it is not the default path in this change.

## Contract

Request: `{ "operation": "quality_gate", "item", "plan", "result" }`.
Response: `{ "ok": true, "quality": { ... }, "reasons": [...] }`.
`ok` is protocol success; `quality.passed` is the gate verdict. A malformed
request returns `{ "ok": false, "error": "..." }`.

## Decision — 2026-11-17

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
- [ ] Switch the production default to Rust, bump the BrainRust pin and delete
      the Go gate after `v0.3.7` is published.
- [x] The docs record the exact commands, counts and commit state.

## Verification — 2026-11-17

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

## Non-goals

- No change to the production default in this change; `VELOX_RUST_QUALITY_GATE`
  defaults to `false`.
- No deletion of the Go `qualitygate` package until the release is published
  and the pin is bumped.
- No changes to SQLite, outbox, Drive, provider or prompt ownership.
