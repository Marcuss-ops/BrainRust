# BR-CI-01 — The language-pack gate must not require the Go checkout

- **Status:** CLOSED (2026-11-24)
- **Priority:** P0
- **Owner:** `lexicon/tools`, `.github/workflows/ci.yml`
- **Evidence date:** 2026-11-24

## Problem

`lexicon/tools/build_language_packs.py` puts the Go consumer snapshot

```
../refactored/internal/capabilities/linguistics/profiles.generated.json
```

into the same `files` map as the Rust-owned packs. `--check` therefore reports
that path as stale when BrainRust is checked out on its own, which is exactly
what GitHub Actions does. The CI job fails before `cargo fmt`, Clippy and the
Rust tests run.

Locally the same command passes only because the adjacent PipelineGen checkout
exists and its snapshot happens to be current. That makes the gate
environment-dependent and hides the real contract.

## Acceptance criteria

- [x] `--check` validates only files owned by the BrainRust checkout; it passes
      in a copy with no sibling `refactored/` directory.
- [x] `--check-consumer` additionally validates the adjacent Go snapshot and
      fails with an actionable message when the snapshot is missing or stale.
- [x] Write mode never creates a new `../refactored/` tree in a standalone
      checkout; it reports that the consumer snapshot was skipped.
- [x] CI keeps using `--check`; the cross-repo PipelineGen job uses
      `--check-consumer` where the Go checkout exists.
- [x] Verification covers three cases: standalone `--check` passes, adjacent
      `--check-consumer` passes, missing/stale consumer fails only when
      `--check-consumer` is requested.

## Verification (2026-11-24)

| Case | Command | Result |
| --- | --- | --- |
| Python syntax | `python3 -m py_compile lexicon/tools/build_language_packs.py` | RC 0 |
| Local standalone mode | `python3 lexicon/tools/build_language_packs.py --check` | RC 0; `Go snapshot skipped` |
| Local consumer mode | `python3 lexicon/tools/build_language_packs.py --check --check-consumer` | RC 0; `Go snapshot up to date` |
| Standalone copy | same `--check` in a temp copy with no sibling `refactored/` | RC 0 |
| Missing consumer | standalone copy + `--check-consumer` | RC 1 with the expected path |
| Present consumer | copy the current Go snapshot into the temp tree + `--check-consumer` | RC 0 |

The CI workflow still calls `--check`; its comment now names the cross-repo
`--check-consumer` mode. The local Go snapshot was current before the test.

## Non-goals

- Removing the Go snapshot from the generator: it is still the cross-repo
  contract.
- Making BrainRust own the Go consumer build.
