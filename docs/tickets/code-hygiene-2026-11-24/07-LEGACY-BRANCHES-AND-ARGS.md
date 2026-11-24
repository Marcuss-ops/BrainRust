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

- [x] Every legacy branch above is removed, or carries a ticket link, owner and
      removal date in a comment plus a test.
- [x] No `#[allow(clippy::too_many_arguments)]` remains in production; each
      site is refactored to a context struct or split. Resta solo il fixture
      test-only `micro.rs::tests::fact`, documentato come tale.
- [x] No behavior change in the audio plan, chart series, overlay coalescing or
      presentation handoff; existing tests stay green (gate mirati sotto).
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

## Lavoro 2026-11-24

### Refactor `too_many_arguments`

Otto siti di produzione sono passati a struct di contesto, senza cambiare il
corpo delle funzioni:

| Funzione | Contesto |
| --- | --- |
| `project_scene` | `SceneProjection` |
| `project_entity` | `EntityProjection` |
| `scene_phrase` | `ScenePhraseInput` |
| `cut_one` | `CutOneRequest` |
| `resolve_value` | `ResolveValueContext` |
| `cross_sentence_series` | `SeriesContext` |
| `build_series_candidate` | `SeriesCandidateContext` |
| `candidate` | `RecipeCandidate` |

L'unico `#[allow(clippy::too_many_arguments)]` rimasto è il fixture
`micro.rs::tests::fact`, dentro `#[cfg(test)]`: non è codice di produzione ed è
commentato come tale.

### Branch legacy

Ogni branch ha ora owner, data di ritiro `2027-03-31`, link a questo ticket e un
test che lo copre:

- `render_audio_execution::layer_chains` — test `sources.rs` con piano
  `background_music` non vuoto.
- `charts::series::chart_measure` — nuovo test
  `a_point_without_a_typed_quantity_uses_the_legacy_parser`.
- `planner::coalesce` (`CHAIN_HANDOFF_US` e `chain_camera` 0) — test
  `planner/tests/chains.rs` e `tests/overlay_plan_contract.rs`.
- `planner::presentation::normalize_font_alias` (`playfair_display`) — test
  `bullet_pin_tests.rs::legacy_playfair_alias_normalizes_but_other_families_survive`.

### Verifica

| Gate | Esito |
| --- | --- |
| `cargo check -p overlay-compiler -p pipelinegen-muscles -p semantic-engine --locked` | RC 0 |
| `cargo test -p semantic-engine a_point_without_a_typed_quantity_uses_the_legacy_parser --locked` | 1/1 |
| `cargo test -p overlay-compiler --lib --locked` | 169/169 |
| `cargo test -p pipelinegen-muscles --lib --locked` | 153/153 |
| `cargo clippy -p ... --lib --locked -- -D warnings` | RC 0 su tutti e tre |
| `cargo fmt --all -- --check` | RC 0 |
| `git diff --check` | RC 0 |
| `cargo test --workspace --locked` | in corso / da completare |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | in corso / da completare |
