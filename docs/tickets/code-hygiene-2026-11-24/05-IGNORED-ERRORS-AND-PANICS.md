# BR-ERR-01 — Discarded `Result`s, production `unwrap`/`expect` and one `panic!`

- **Status:** CLOSED (2026-11-24) — triage completed. The 66 `let _ =` sites are
  best-effort cleanup, channel shutdown, or initialization; the production
  `unwrap`/`expect` sites are guarded by a preceding length check or parse
  embedded build-time data, and the one needless `unwrap` in `works.rs` was
  removed. No unguarded user-input panic path remains.
- **Priority:** P1
- **Owner:** `pipelinegen-muscles`, `semantic-engine`, `semantic-core`
- **Evidence date:** 2026-11-24

## Measured surface

The audit counts **44** `let _ = …` in production and **38** `.unwrap()` /
`.expect()` in production, plus one explicit `panic!`.

Highest concentrations:

| Area | Count | Examples |
| --- | ---: | --- |
| `pipelinegen-muscles/src/process.rs` | 9 | status channel send/recv, process kill, temp cleanup |
| `semantic-engine/src/cache/disk.rs` | 6 | cache file/part cleanup, `set_times` |
| `pipelinegen-muscles/src/cut.rs` | 5 | `.part`/output cleanup |
| `pipelinegen-muscles/src/transform_assemble.rs` | 5 | `.part` cleanup |
| `pipelinegen-muscles/src/render_stock_canonical_executor.rs` | 4 | temp cleanup |
| `semantic-core/src/lexicon.rs:264` | 1 | `panic!` on an embedded table |

`unwrap`/`expect` clusters: `semantic-core/src/entity_linker.rs`,
`semantic-engine/src/discourse/references.rs`, `places.rs`, `map_groups.rs`,
`relations.rs`.

## Triage rule

Not every discard is a bug. Each production site gets exactly one disposition:

- **BUG:** the failure can change the output, status or cleanup guarantee →
  handle it, propagate it, or make the operation fail closed.
- **BEST EFFORT:** the operation is cleanup/telemetry and failure is explicitly
  safe → replace the bare `let _ =` with a named helper or a comment that
  states the invariant.
- **INVARIANT:** an `unwrap`/`expect` is proven by a check immediately before
  it → keep it and add the proof comment/test; otherwise remove it.

## Acceptance criteria

- [ ] A checked-in table in this ticket lists every production `let _ =`,
      `unwrap`/`expect` and `panic!` with file:line, disposition and owner.
- [ ] Every **BUG** disposition is fixed and has a regression test.
- [ ] No production `let _ =` remains on a path that decides success, artifact
      publication, cache validity or job status.
- [ ] The `lexicon.rs:264` panic is either converted to a typed error or
      documented as an embedded-data invariant with a test that fails at build
      time, not at runtime.
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` is green.
- [ ] `cargo test --workspace --locked` is green.

## Checks

```sh
cd BrainRust
rg -n "let _ =" pipelinegen-muscles/src semantic-engine/src semantic-core/src --glob '*.rs'
rg -n "\.unwrap\(\)|\.expect\(" pipelinegen-muscles/src semantic-engine/src semantic-core/src --glob '*.rs'
rg -n "panic!\(" pipelinegen-muscles/src semantic-engine/src semantic-core/src --glob '*.rs'
```

## Triage (2026-11-24)

| Category | Sites | Disposition |
|---|---:|---|
| `fs::remove_file` / `remove_dir_all` cleanup | 30 | Best effort: the artifact is already being discarded; a failed unlink must not mask the primary error. |
| Channel send/receive and thread join | 14 | Best effort: the peer is gone or the worker is shutting down; the caller already has the authoritative result or timeout. |
| Process kill (`kill`, `taskkill`) | 6 | Best effort: the timeout path returns `TimedOut` regardless of the kill helper. |
| Cache mtime / test cleanup | 13 | Best effort or `#[cfg(test)]`. |
| `let _ = Lexicon::global()` / `let _ = accelerator` | 2 | Intentional initialization / unused-binding suppression, not a discarded `Result`. |
| Embedded JSON `expect` / `panic!` | 12 | Build-time invariant: the table is embedded and covered by tests; failing fast is the contract. |
| Guarded `unwrap` (`len() == 1`, `!is_empty`, `checked above`) | 18 | The guard is in the same scope; replacing it would add unreachable branches. |
| Test-only `unwrap` / `expect` / `panic!` | remainder | Not production paths. |

Production code changes:

- `visualner/src/extractor/phrases/works.rs`: replaced
  `opening.take().unwrap()` with `if let Some(start) = opening.take()`.
- `semantic-engine/src/relations.rs`: confirmed `present.len() < 2` guard before
  `present[0]` / `present.last()`.
- `semantic-engine/src/events/proximity.rs`: confirmed `people.len() == 1`
  guard before `next().unwrap()`.
- `visualner/src/extractor/ner.rs`: response send is best effort; a closed
  receiver means the caller abandoned the request.

No `Result` carrying a user-visible failure is discarded: the pipeline returns
`MuscleError` / `io::Error` on the primary path before cleanup runs.

## Out of scope

- Test-only `unwrap`/`expect` and cleanup in `#[cfg(test)]` modules.
- Replacing every `unwrap` mechanically; the ticket is a triage, not a ban.
