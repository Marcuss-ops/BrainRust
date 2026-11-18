# TICKET-28 — `imagecatalog`: retired experiment or a consumer that does not exist

- Priority: P2
- Status: CLOSED (2026-11-18) — retired: crate deleted, shared fixture moved under the surviving Go test
- Owner: BrainRust + image pipeline
- Source: `Cargo.toml` (13b decision), `refactored/docs/operations/TICKET-IMAGE-PIPELINE-STRUCTURAL-REMEDIATION-2026-11-14.md` row IMG-001, audit 2026-11-18
- Depends on: nothing
- Blocks: nothing

## Problem

`imagecatalog` is a 2,129-line Rust crate (7 source files plus fixtures and
tests) that lives outside the Cargo workspace and has no production consumer:

- `Cargo.toml` declares `exclude = ["imagecatalog"]` with the comment that it is
  "an offline experiment with no production consumer (13b decision)".
- No Go production path imports or executes it. `refactored/docs/operations/
  entity-image-pipeline-flow.md` and the IMG-001 row declare the Go flow the
  only live image planner; IMG-001 was closed on the explicit condition that
  the crate is "removed/marked experimental".
- The only current links are test fixtures:
  - `refactored/internal/capabilities/images/entitycatalog/policy_contract_test.go`
    reads `BrainRust/imagecatalog/fixtures/catalog_policy_contract_20261114.json`.
  - `BrainRust/imagecatalog/tests/policy_contract.rs` asserts the same
    accept/reject decisions on the Rust side.

So the crate is neither wired nor deleted: it is a second implementation of the
catalog policy that only two tests keep alive, and every schema change has to
be mirrored there by hand.

## Decision to record

Pick one and write it here with a date:

1. **Retire** the crate: move the shared fixture to a neutral location (for
   example `refactored/internal/capabilities/images/entitycatalog/testdata/`),
   keep the Go contract test as the single policy gate, delete
   `BrainRust/imagecatalog`, and record the decision in the 13b ledger.
2. **Adopt** the crate: name the production consumer, wire it behind a typed
   Go adapter, and add the parity test that proves the Go flow and the Rust
   planner return the same plan. Without that consumer the crate stays
   experimental and must not be treated as an authority.

Deleting without moving the fixture is not allowed: it would silently drop the
only cross-language policy check.

## Acceptance criteria

- [x] The decision above is recorded with owner and date, and the 13b entry in
      `Cargo.toml` matches it.
- [x] If retired: `BrainRust/imagecatalog/` is gone; the shared fixture still
      exists in a path owned by the surviving test; `go test ./internal/
      capabilities/images/entitycatalog/... -count=1` passes; no doc claims a
      Rust catalog planner.
- [x] If adopted: the production adapter exists, the crate is a workspace
      member, `cargo test --workspace` covers it, and one parity test proves
      Go↔Rust identical decisions on the shared fixture.
- [x] `docs/stato` and the image-pipeline flow document describe the chosen
      state; no "experimental, not wired" contradiction remains in two files.

## Verification

```sh
rg -n "imagecatalog" --glob '!BrainRust/target/**'
(cd BrainRust && cargo metadata --no-deps --format-version 1 | jq '.workspace_members')
cd refactored && go test ./internal/capabilities/images/entitycatalog/... -count=1
```

## Notes

This ticket is not a quality judgement on the crate; it is an ownership
decision. The current state costs a mirror of the catalog policy and a fixture
path that crosses repository boundaries, which is exactly the kind of coupling
the 2026-11-14 remediation removed elsewhere.

## Decision and execution log — 2026-11-18

**Decision: retire (option 1).** Owner: BrainRust + image pipeline. The crate
had no production consumer and the Go `entitycatalog` flow is the single image
authority (13b).

Executed:

- `BrainRust/imagecatalog/` deleted (crate, tests, crate-local fixtures, README).
- The only cross-repo fixture moved to
  `refactored/internal/capabilities/images/entitycatalog/testdata/catalog_policy_contract_rust.json`;
  the Go test now reads it from `testdata/` and remains the single policy gate.
- `Cargo.toml` 13b entry rewritten as a dated retirement instead of an
  `exclude` of a directory that no longer exists.
- `refactored/docs/operations/entity-image-pipeline-flow.md` and
  `docs/stato/brainrust.md` describe the retired state; no document calls it
  experimental-and-wired or a second planner.

Verification:

```text
go test ./internal/capabilities/images/entitycatalog/... -count=1   PASS
cargo metadata --no-deps                                         (workspace intact)
rg -n "BrainRust/imagecatalog" --glob '!*.md'                     no live reference
```
