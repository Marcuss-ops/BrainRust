# BrainRust code hygiene — wave 2026-11-24

**Origin:** the 2026-11-24 audit of the BrainRust workspace (8 Rust crates,
241 production `.rs` files, ~99k LOC). This wave takes only findings that are
still present on `main` at `116c8d4` plus the current working tree.

**Rule:** one ticket per problem class; every closure needs the command, the
exit code and the artifact that proves it. A finding that needs a human label,
a GPU or a live service is declared `BLOCKED` or `OPEN`, never `PASS`.

## Tickets

| ID | Priority | Status | Title |
| --- | --- | --- | --- |
| [BR-HYG-01](01-DEAD-TOOLS-AND-WORKTREE.md) | P2 | PARTIAL | Dead `tools/`, imagecatalog residue and abandoned worktree artifacts |
| [BR-DRY-01](02-EXACT-CODE-CLONES.md) | P1 | CLOSED | Exact clones: `is_combining_mark`, `sentence_index_at`, `lowercase_hex` |
| [BR-DATE-01](03-MULTILINGUAL-DATE-SSOT.md) | P1 | CLOSED | Italian-only month tables in `overlay-compiler`; gate exception documented |
| [BR-STATE-01](04-UNBOUNDED-GLOBAL-STATE.md) | P1 | CLOSED | Unbounded `LATEST` map and unvalidated `schema` field |
| [BR-ERR-01](05-IGNORED-ERRORS-AND-PANICS.md) | P1 | CLOSED | Discarded `Result`s, production `unwrap`/`expect` and one `panic!` |
| [BR-HOT-01](06-COMPLEXITY-HOTSPOTS.md) | P1 | OPEN | Eight complexity hotspots (CC 40–65, 114–311 LOC) |
| [BR-LEGACY-01](07-LEGACY-BRANCHES-AND-ARGS.md) | P2 | OPEN | Legacy audio/chart/alias branches and nine `too_many_arguments` |
| [BR-DRY-02](08-CLONE-STORM-AND-CORE-DUP.md) | P2 | OPEN | `.clone()` storm and duplicate tokenize/sentence/word primitives |

**Counts:** 4 CLOSED · 1 PARTIAL · 3 OPEN.

## Execution order

1. **Zero-risk cleanup:** BR-HYG-01.
2. **Exact clones and shared primitives:** BR-DRY-01.
3. **Date SSOT:** BR-DATE-01.
4. **Global state:** BR-STATE-01.
5. **Error triage:** BR-ERR-01, one sub-check at a time.
6. **Hotspots and legacy:** BR-HOT-01, BR-LEGACY-01, BR-DRY-02.

## Common gates

```sh
cd BrainRust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

For a ticket that touches only one crate, the minimum is that crate's tests
plus `cargo check --workspace`. The full workspace suite is required before the
wave is declared CLOSED; if the shared `target/` is in use by another
workstream, record the blocked gate instead of claiming a pass.

## Worktree rule

`BrainRust` currently has 14 modified/untracked files from another workstream
(`overlay-compiler/src/editorial_plan_v2/planner/*`, `visualner/src/extractor/verb*`).
This wave must not stage or revert them. Commit only the paths listed in the
ticket.

## Verification (2026-11-24)

| Gate | Result |
| --- | --- |
| `cargo check -p semantic-core -p visualner -p semantic-engine -p pipelinegen-muscles -p overlay-compiler --locked` | RC 0 |
| `cargo test` on the five touched crates (non-doctest) | all code tests passed |
| `cargo test -p visualner italic_and_guillemet` (direct binary) | RC 0, 1 passed |
| `cargo clippy -p semantic-core --lib --locked -- -D warnings` | RC 0 |
| `git diff --check` | RC 0 |
| `rustfmt --check` on the 11 touched files | RC 0 |


Not claimed: the `overlay-compiler --doc` doctest failed with `E0463`
(`can't find crate for serde/semantic_core`) while another profile was building
the same target; the full `visualner` suite exceeded the 600 s timeout under the
shared lock. Both gates must be rerun in closeout when the worktree is quiet.

## Remaining blockers

- **BR-HYG-01:** `target/` removal deferred while other profiles are compiling.
- **BR-HOT-01 / BR-LEGACY-01 / BR-DRY-02:** need dedicated equivalence tests or
  touch files already dirty in the concurrent workstream. They stay OPEN with
  the precise next step in each ticket.
