# TICKET 39 — wire preflight and the persistent cache to the consumer

- Priority: P2
- Status: CLOSED 2026-11-19 (Rust operation, contract golden, Go caller and
  cache opt-in landed and verified; deployment needs the next BrainRust release
  that carries the operation)
- Source: residuals split out of TICKET-37 (cache) and TICKET-38 (preflight)
  when they closed on 2026-11-19
- Files: `BrainRust/pipelinegen-muscles/src/protocol.rs`,
  `BrainRust/contract/mediaexec.v2/`, `refactored/internal/platform/media/rustexec/`,
  and the PipelineGen startup path that launches `semantic-engine`

## Problem

Both Rust features are implemented, tested and closed, but nothing calls them:

- `pipelinegen_muscles::preflight::check` is a library function; no
  `mediaexec.v2` operation exposes it, so a job cannot ask for it.
- `SEMANTIC_CACHE_DIR` is read by `Worker::from_env`, but PipelineGen starts the
  worker without it, so every batch is memory-only and the disk saving is
  latent.

Closing TICKET-37/38 without this leaves the product work unused. It was split
here because it needs a coordinated Rust+Go change and a golden, which cannot
be completed inside this repository alone.

## Scope

1. **Preflight operation.** In `BrainRust/pipelinegen-muscles/src/protocol.rs`,
   add the `preflight` operation to the request enum and the optional report
   field to `Response` (updating the struct literals in `tests/protocol`).
2. **Contract.** In `BrainRust/contract/mediaexec.v2/`, update
   `request.schema.json` and `response.schema.json`, add a `preflight` golden in
   `goldens/`, then run `validate.py` and `check_consumer_copy.sh` to sync
   `refactored/testdata/mediaexec/v2`.
3. **Go decision.** In `refactored/internal/platform/media/rustexec/`, add the
   caller that invokes `preflight` and translates `PreflightReport.ok` /
   `issues[]` into the Go-owned blocking decision. Log the codes so a blocked
   job is explainable.
4. **Cache opt-in.** Start `semantic-engine` with `SEMANTIC_CACHE_DIR` (and
   optionally `SEMANTIC_CACHE_MAX_BYTES`) for batch and replay workloads;
   unset must keep today's behaviour.
5. **Measure.** Report hit rate and warm/cold wall clock on a real batch. T15 in
   TICKET-37 measured 21.35 s cold vs 8.55 s warm for 100 scenes, with the warm
   run dominated by worker startup (UDPipe model load), so deployment numbers
   matter more than the microbenchmark.
6. **Optional, separate decision.** Lazy UDPipe loading would make warm hits
   cheap for short scripts. It changes worker startup semantics and needs its
   own measurement; do not fold it into the wire change.

## Acceptance criteria

- `mediaexec.v2` accepts `preflight` and returns the structured report; an
  unknown/absent request still behaves exactly as before.
- The golden and the struct literals are updated together and
  `cargo test -p pipelinegen-muscles` is green.
- `python3 contract/mediaexec.v2/validate.py` passes and
  `bash contract/mediaexec.v2/check_consumer_copy.sh ../refactored/testdata/mediaexec/v2`
  reports no drift.
- `cd refactored && go test ./internal/platform/media/rustexec -count=1` shows:
  clean plan proceeds, missing asset blocks with `asset_missing`, dense cue only
  warns, and the Go caller is the component that decides.
- With `SEMANTIC_CACHE_DIR` set, a repeated batch reports `cached: true` from
  the Go side and the second run is measurably faster; without it, unchanged.
- No path in the preflight operation writes to disk (the existing Rust test is
  reused).

## Why it was split out

The Go consumer and its schema live in the PipelineGen repository. Doing only
the Rust half would have published a wire operation no consumer uses, which is
what TICKET-38 deliberately avoided.

## Closure — 2026-11-19

All five scope items landed in the working trees.

### Rust (`BrainRust`)

- `protocol.rs`: `Operation::Preflight` (now 20 operations), optional typed
  `Request.preflight: Option<PreflightRequest>` and
  `Response.preflight: Option<PreflightReport>`. The response field is skipped
  when absent, so every existing response is byte-identical on the wire.
- `preflight_operation.rs`: adapts the pure engine to the envelope. `ok` is
  true whenever the check ran, even when the plan is blocked, so the Go client
  receives the reasons instead of discarding them; the envelope no longer owns
  the blocking decision.
- `Request::validate` rejects `preflight` without its payload; other requests
  are unchanged.

### Contract

- `request.schema.json`: `preflight` operation, typed `preflight` payload and
  `preflight_asset|cue|overlay` definitions; `allOf` requires the payload when
  the operation is `preflight`.
- `response.schema.json`: optional `preflight_report` / `preflight_issue`
  definitions (`severity` is the stable `error|warning` enum).
- `goldens/preflight.json`: one clean-envelope report carrying a dense-cue
  warning; copied byte-identically to `refactored/testdata/mediaexec/v2/`.

### Go (`refactored`)

- `rustexec/protocol.go`: `OperationPreflight`, typed payload on the request,
  optional report on the response, and payload validation.
- `rustexec/preflight.go`: `Client.Preflight` and the Go-owned
  `PreflightDecision`. `Blocked` is derived from `error`-severity issues; a
  report that says `ok:false` without one is blocked as
  `preflight_report_not_ok` so a drifted worker cannot pass an invalid plan.
  Blocking and warning codes are logged.
- `rustexec/semantic_engine.go`: `NewSemanticEngineExecutor` forwards
  `SEMANTIC_CACHE_DIR` and `SEMANTIC_CACHE_MAX_BYTES` explicitly to the worker;
  unset forwards nothing and keeps the memory-only behaviour. A cache hit is
  decoded from `cached` and logged.
- `rustworker/multiplexed.go`: `NewMultiplexedPersistentRunnerWithEnv` applies
  explicit KEY=VALUE overrides on top of the inherited environment.

## Acceptance evidence (2026-11-19)

```sh
cd BrainRust
cargo fmt -p pipelinegen-muscles -- --check
cargo clippy -p pipelinegen-muscles --all-targets --offline -- -D warnings
cargo test -p pipelinegen-muscles --offline
python3 contract/mediaexec.v2/validate.py
bash contract/mediaexec.v2/check_consumer_copy.sh ../refactored/testdata/mediaexec/v2

cd ../refactored
go test ./internal/platform/rustworker ./internal/platform/media/rustexec -count=1
```

Results:

- `cargo test -p pipelinegen-muscles` — green, including 16 preflight unit
  tests and the 4 real-media integration tests.
- `validate.py` — **4 shared goldens validated**; `check_consumer_copy.sh` —
  **consumer goldens match**.
- `go test ./internal/platform/rustworker` — ok; `rustexec` — ok, **89.4 s**
  for the whole package (preflight protocol/golden/client tests, cache-env and
  cached-log tests included).
- Real worker, `BrainRust/target/debug/pipelinegen-muscles`:
  - a missing asset returns `ok:true` with
    `preflight.issues[0] = {severity:"error", code:"asset_missing"}` and a
    dense cue is a second `warning` issue;
  - `health` still returns the original six-field envelope with **no**
    `preflight` key.
- Persistent cache, `refactored/bin/semantic-engine` with UDPipe models and a
  throwaway directory, same one-scene Italian batch twice:
  - cold **3.64 s**, `cached:false`;
  - warm **1.56 s**, `cached:true`;
  - exactly **one** `manifest-<sha256>.json` entry and no part files.
- Go-side opt-in: `semanticCacheEnvironment()` forwards both variables only
  when set; the runner test proves an explicit override reaches the child and
  that the default runner still inherits; the adapter test proves a
  `cached:true` response is reported.
- Go end-to-end, `TestSemanticEngineRealBinaryServesRepeatedBatchesFromTheCache`
  (real `bin/semantic-engine`, temp cache dir): **PASS in 3.17 s** through
  `NewSemanticEngineExecutor` — cold `cached:false`, repeated `cached:true`, one
  entry. Hit rate on the two-run sample is 1/2.

## Residual

- **Release prerequisite.** The pinned `refactored/bin/pipelinegen-muscles`
  (`v0.3.7`, `git_sha 2ca3603`) predates the `preflight` operation. The caller
  and schema are ready, but a deployment must roll a BrainRust release that
  contains the operation (TICKET-27 / v0.3.8) before the Go path can use it.
  Until then no production path invokes `preflight`, so the mismatch is inert.
- **Concurrent-overlay budget.** Still not part of the request; inventing it in
  Rust would move policy across the boundary. Separate decision.
- **Lazy UDPipe loading.** Warm hits still pay worker startup; separate change,
  not a cache defect.
- **Long-batch hit rate.** The measurement above is one scene on the installed
  release binary. TICKET-37's T15 (100 scenes: 21.35 s cold / 8.55 s warm)
  remains the long-batch reference.
- The changes are in the working trees; publishing `v0.3.8` and pinning it
  remains TICKET-27's external gate.
