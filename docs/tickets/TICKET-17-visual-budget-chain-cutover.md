# TICKET 17 — Visual budget and chain coalescing cutover

- Priority: P1
- Status: DONE (2026-11-16)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-14 (single-call overlay planner)
- Blocks: none (TICKET-13 and 13d closed 2026-11-16); independent after TICKET-14

## Why

The Go overlay package is still 84 production files / 15.269 LOC. Two
decision clusters are not part of the current `editorial_plan.v2` port:

- **Visual budget** — `internal/capabilities/overlays/visual_budget.go`,
  `visual_budget_admission.go`, `visual_budget_entity_cards.go`,
  `visual_budget_phrase.go` decide admission, group caps and lane balance.
- **Chain coalescing** — `internal/capabilities/overlays/model/chain_coalesce.go`
  (732 LOC) fuses nearby overlays into `phrase_chain` / `image_chain` /
  `mixed_chain` / `map_chain` items with their own timing and handoff.

If these stay in Go after TICKET-14, the "single call" returns a plan that Go
still edits.

## Update 2026-11-16 (verified)

The only production chain call site — the Go seal's
`CoalesceCaptionedEntityImageCards` — has been removed; the captioned-card
fusion now runs in the Rust planner. The general `CoalesceChainedOverlays`
policy is also ported to Rust (`coalesce_chained_overlays` + `fuse_chain`) and
covered by the `general_chain_*` unit tests: gap clamp, segment ceiling, image
partition, 400 ms handoff, 2.5 s hold, 7.5 s entity-card gap, scene-end guard,
portrait attachment, and short-phrase/date/chart/map exclusions. The Go
coalescer (`CoalesceChainedOverlays*`, `CoalesceCaptionedEntityImageCards`,
`fuseChain`, timing helper) and its tests were deleted; `model/chain_coalesce.go`
now only owns the wire segment type, decoder/rebase and fail-closed validation,
and the WP7 guard rejects a Go producer reintroduction. `admitOverlayItems` and
the visual-budget functions have no production caller after the single-call
cutover. The formal acceptance is closed; the legacy visual-budget helpers
remain production-dead and guarded.

Golden coverage is now checked in as the seventh fixture,
`general_chain_rendering`. It seals one `phrase_chain`, one `image_chain` and
one `mixed_chain`, each with two segments, a 400 ms handoff and preserved
segment assets; its two map items stay standalone. An eighth fixture,
`chain_collision_resolution`, puts two phrase candidates under a map so both
are shifted to the map end before fusion, then seals a `phrase_chain` starting
at that resolved boundary with a 400 ms segment handoff. The producer-side
`map_chain` arm has been retired: `chain_family` no longer classifies maps and
`chainable` rejects them because a map owns the full georeferenced canvas, pin
timing and LOD assets. Go's wire validator still accepts an explicitly authored
`map_chain` for a geographic sequence, but the Rust compiler never emits one.
`general_chain_keeps_maps_and_chart_groups_standalone` locks the exclusion.

## Closure 2026-11-16 (verified)

The production admission is the Rust `admit` pass, applied after ranking and
before chain fusion and seal. No production Go file calls
`ApplyEditorialOverlayBudget*`, `ApplyPhraseOverlayBudget*`,
`LimitOverlayItemsPerScene` or `admitOverlayItems`; the WP7 guard pins those
symbols. The legacy Go `BuildPlan` and visual-budget helpers remain compiled
only as a test oracle for chart/bullet certification; they are production-dead
and guarded, not a second production authority.

Evidence on this tree: eight fixtures / 30 contract tests, real-worker 8/8,
the deletion guard, and `make verify-main` PASS. The local untracked fragment
cache was disabled with `VELOX_FINAL_JOB_FRAGMENT_CACHE_DIR=off` for the gate
because its foreign avcC corpus entry is outside this cutover; the corpus probe
is designed to skip when the cache is disabled.

## Target

Fold both clusters into `compile_editorial_plan` v2:

- request carries the full candidate set, budgets, group limits and chain
  policy;
- response carries final items, chain segments, placements and intake;
- Go applies the response and seals; no budget or chain recomputation.

## Acceptance

- Rust planner ports budget admission in its real order (after ranking, before
  seal); chain coalescing is already ported and the Go producer is deleted.
- Golden parity for phrase, image and mixed general chains, including 400 ms
  handoff gaps, asset preservation and map-collision resolution before fusion;
  maps, charts and dates stay standalone by policy.
- `visual_budget*` has no production caller; `model/chain_coalesce.go` is
  wire-only (segment type, decoder/rebase and validation).
- One `compile_editorial_plan` request per plan, counter test green.
- `make verify-main` on the cutover tree.

## Risks

- Chain fusion changes the number of overlay items and the render key; a
  partial port changes the video even when the text is identical.
- Budget admission is order-sensitive; apply it before chain fusion exactly as
  the Go planner does.
