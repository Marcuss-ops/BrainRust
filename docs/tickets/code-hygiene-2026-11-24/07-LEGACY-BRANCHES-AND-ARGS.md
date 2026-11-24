# BR-LEGACY-01 — Legacy audio/chart/alias branches and nine `too_many_arguments`

- **Status:** OPEN — blocked by the worktree rule. Most of the nine
  `too_many_arguments` sites (`scenes.rs`, `editorial.rs`, `cut.rs`, `legal.rs`,
  `groups/charts/series.rs`, `consolidate/recipes.rs`) are in files already
  modified by another profile; the user explicitly asked not to touch them.
  Resume after that workstream lands; `micro.rs` can be the first isolated
  change.
- **Priority:** P2
- **Owner:** `pipelinegen-muscles`, `semantic-engine`, `overlay-compiler`
- **Evidence date:** 2026-11-24

## Legacy branches still live (~30 occurrences)

- `pipelinegen-muscles/src/render_audio_execution/render_context.rs:55-206`:
  legacy `background_music` / `sfx` layer handling.
- `semantic-engine/src/groups/charts/series.rs:4`: legacy chart fallback.
- `overlay-compiler/src/editorial_plan_v2/planner/coalesce.rs:38-218` and
  `planner/presentation.rs:221`: `playfair_display` alias and legacy 400 ms
  handoff.

Each branch needs one of two outcomes: a named owner and removal date, or a
test that proves it is still the production path.

## `too_many_arguments` (9 sites)

At least:

- `overlay-compiler/src/editorial_plan_v2/projection/scenes.rs:111,270`
- `overlay-compiler/src/editorial_plan_v2/editorial.rs:240`
- `pipelinegen-muscles/src/cut.rs:134`
- `semantic-engine/src/legal.rs:358`
- `semantic-engine/src/groups/charts/series.rs:53,257`
- `semantic-engine/src/micro.rs:363`
- `semantic-engine/src/consolidate/recipes.rs:29`

Each is a missing context struct or a function doing two jobs. Replace the
positional parameter bag with a named struct, or split the function.

## Acceptance criteria

- [ ] Every legacy branch above is removed, or carries a ticket link, owner and
      removal date in a comment plus a test.
- [ ] No `#[allow(clippy::too_many_arguments)]` remains in production; each
      site is refactored to a context struct or split.
- [ ] No behavior change in the audio plan, chart series, overlay coalescing or
      presentation handoff; existing tests stay green.
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` is green.
- [ ] `cargo test --workspace --locked` is green.

## Checks

```sh
cd BrainRust
rg -n "allow\(clippy::too_many_arguments\)|background_music|playfair_display|handoff" \
  pipelinegen-muscles/src semantic-engine/src overlay-compiler/src --glob '*.rs'
cargo test -p pipelinegen-muscles -p semantic-engine -p overlay-compiler --locked
```

## Out of scope

- Removing a legacy branch that still has a production consumer without a
  migration path.
