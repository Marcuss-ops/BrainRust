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
| [BR-LEGACY-01](07-LEGACY-BRANCHES-AND-ARGS.md) | P2 | PARTIAL | Legacy audio/chart/alias branches and eight production `too_many_arguments` (+1 test-only) |
| [BR-DRY-02](08-CLONE-STORM-AND-CORE-DUP.md) | P2 | OPEN | `.clone()` storm and duplicate tokenize/sentence/word primitives |
| [BR-CI-01](09-CI-AUTONOMOUS.md) | P0 | CLOSED | Language-pack gate must not require the Go checkout |
| [BR-CACHE-01](10-UDPIPE-CONTENT-DIGEST.md) | P1 | CLOSED | UDPipe model identity must be content-addressed |
| [BR-PERF-01](11-SCENE-PARALLELISM.md) | P2 | OPEN | Scene analysis is sequential and unmeasured |
| [BR-PUB-01](12-PUBLIC-READINESS.md) | P3 | OPEN | Public presentation, license and reproducible demo |

**Counts:** 6 CLOSED · 2 PARTIAL · 4 OPEN.

The 2026-11-24 external review added BR-CI-01, BR-CACHE-01, BR-PERF-01 and
BR-PUB-01. Independent semantic validation is already tracked by
[TICKET-08](../TICKET-08-editorial-gold-set.md),
[TICKET-10](../TICKET-10-multimodal-pgvector.md) and
[TICKET-26](../TICKET-26-provider-precision-at-10.md); this wave does not
duplicate those tickets.

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

`BrainRust` currently has modified/untracked files from several concurrent
workstreams (lexicon, `semantic-core`, `visualner`, `overlay-compiler` and
docs). This wave must not stage or revert them. Commit only the paths listed in
the ticket.

## Verification (2026-11-24)

| Gate | Result |
| --- | --- |
| `cargo check -p semantic-core -p visualner -p semantic-engine -p pipelinegen-muscles -p overlay-compiler --locked` | RC 0 |
| `cargo test` on the five touched crates (non-doctest) | all code tests passed |
| `cargo test -p visualner italic_and_guillemet` (direct binary) | RC 0, 1 passed |
| `cargo clippy -p semantic-core --lib --locked -- -D warnings` | RC 0 |
| `git diff --check` | RC 0 |
| `rustfmt --check` on the 11 touched files | RC 0 |
| `python3 lexicon/tools/build_language_packs.py --check` in a standalone copy | RC 0; Go snapshot skipped |
| `... --check --check-consumer` missing / present | RC 1 expected / RC 0 |
| `cargo test -p semantic-engine --lib model_profile --locked` | 2 passed |
| direct `semantic_engine-... cache` binary | 29 passed |
| `rustfmt --check` on `semantic-engine/src/{lib.rs,cache/disk.rs}` | RC 0 |

Not claimed: the full `semantic-engine --lib` suite and its clippy did not
finish inside the 600 s shared-target window; the workspace build was compiling
`gliner25-rs`/`ort`. The targeted `model_profile` and cache tests passed from
the built binary. The `overlay-compiler --doc` and full `visualner` gates remain
as before: they must be rerun in closeout when the worktree is quiet.

## Remaining blockers

- **BR-HYG-01 (PARTIAL):** `target/` removal deferred while other profiles are
  compiling; the other artifacts are gone.
- **BR-HOT-01 / BR-DRY-02:** need dedicated equivalence tests or touch files
  already dirty in the concurrent workstream. They stay OPEN with the precise
  next step in each ticket.
- **BR-LEGACY-01 (PARTIAL):** eight production `too_many_arguments` are
  refactored and the legacy branches are documented/tested; the workspace test
  is blocked only by the `semantic-engine` doctest E0463. Re-run when the
  workspace is quiet.
- **BR-PERF-01:** needs a reproducible baseline and byte-identical manifest
  proof before parallelising scenes.
- **BR-PUB-01:** the license choice is an owner decision; the demo and README
  follow it.
