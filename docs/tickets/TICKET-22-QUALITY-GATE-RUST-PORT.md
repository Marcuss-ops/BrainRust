# TICKET-22 — Port the editorial quality gate to Rust

- Priority: P2
- Status: DONE (2026-11-17)
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
`QualityGateError`) and the production default; this ticket does not cut over
`generation_finalize.go`.

## Contract

Request: `{ "operation": "quality_gate", "item", "plan", "result" }`.
Response: `{ "ok": true, "quality": { ... }, "reasons": [...] }`.
`ok` is protocol success; `quality.passed` is the gate verdict. A malformed
request returns `{ "ok": false, "error": "..." }`.

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
- Commit state: **uncommitted working tree**. The new crate, the
  `semantic-engine` operation and this ticket are staged nowhere; the Go
  `qualitygate` package is untouched.

## Non-goals

- No change to `generation_finalize.go` production behavior.
- No deletion of the Go `qualitygate` package.
- No changes to SQLite, outbox, Drive, provider or prompt ownership.
