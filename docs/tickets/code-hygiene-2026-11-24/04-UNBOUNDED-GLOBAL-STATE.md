# BR-STATE-01 — Unbounded `LATEST` map and unvalidated `schema` field

- **Status:** CLOSED (2026-11-24) — `LATEST` is capped at 4096 entries with
  oldest-generation eviction; `schema` is validated against
  `relative_time.v1` and the `dead_code` allow is gone.
- **Priority:** P1
- **Owner:** `pipelinegen-muscles`, `semantic-core`
- **Evidence date:** 2026-11-24

## Problem 1 — `LATEST` grows for the process lifetime

`pipelinegen-muscles/src/artifact.rs:48-51` keeps a process-global
`HashMap<String, u64>` of the latest published generation per final path. The
map never removes a key, so a long-lived worker that renders many distinct
output paths grows without bound. The map is correctness-relevant: it prevents
a slow older attempt from replacing a newer completed file.

## Problem 2 — `schema` is parsed and never validated

`semantic-core/src/dates/relative.rs:25-28` declares:

```rust
struct RelativeLexicon {
    #[allow(dead_code)]
    schema: String,
    languages: BTreeMap<String, LanguageRules>,
}
```

The embedded `lexicon/relative_time.json` starts with
`"schema": "relative_time.v1"`, but no code checks it. A malformed or migrated
table would deserialize into the old shape and silently run with wrong rules.

## Acceptance criteria

- [ ] `LATEST` is bounded by a named constant (e.g. `LATEST_CAP = 4096`), and
      eviction removes the oldest generations first.
- [ ] A test inserts more than the cap and asserts the map length and that the
      oldest path is gone.
- [ ] The stale-generation guard still rejects an older generation in the
      existing concurrent-publish test.
- [ ] `RelativeLexicon.schema` is validated against `relative_time.v1`; a
      wrong schema makes `lexicon()` return `None` (fail closed).
- [ ] The `#[allow(dead_code)]` on `schema` is gone.
- [ ] A test pins the embedded schema and the fail-closed behavior.
- [ ] `cargo test -p pipelinegen-muscles -p semantic-core` is green.

## Checks

```sh
cd BrainRust
rg -n "LATEST|LATEST_CAP|allow\(dead_code\)" pipelinegen-muscles/src/artifact.rs \
  semantic-core/src/dates/relative.rs
cargo test -p pipelinegen-muscles artifact --locked
cargo test -p semantic-core dates --locked
cargo clippy -p pipelinegen-muscles -p semantic-core --all-targets -- -D warnings
```

## Evidence (2026-11-24)

- `pipelinegen-muscles/src/artifact.rs`: `LATEST_CAP = 4096` and
  `remember_latest` evict the oldest generations; `latest_published_map_is_bounded`
  inserts 4224 entries and asserts the cap and the eviction.
- `semantic-core/src/dates/relative.rs`: `RelativeLexicon.schema` is validated
  against `relative_time.v1`; a wrong schema makes `lexicon()` return `None`.
  `embedded_relative_lexicon_has_the_expected_schema` passes.
- `cargo test -p pipelinegen-muscles artifact --locked` → passed.
- `cargo test -p semantic-core dates --locked` → passed.
- `cargo check -p pipelinegen-muscles -p semantic-core` → RC 0.

## Out of scope

- Replacing `LATEST` with a cross-process registry: the map is deliberately
  process-local; the worker owns the final output path.
