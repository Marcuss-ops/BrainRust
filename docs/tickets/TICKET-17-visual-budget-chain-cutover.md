# TICKET 17 — Visual budget and chain coalescing cutover

- Priority: P1
- Status: OPEN
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-14 (single-call overlay planner)
- Blocks: TICKET-13 final closure with 14

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

## Target

Fold both clusters into `compile_editorial_plan` v2:

- request carries the full candidate set, budgets, group limits and chain
  policy;
- response carries final items, chain segments, placements and intake;
- Go applies the response and seals; no budget or chain recomputation.

## Acceptance

- Rust planner ports budget admission and chain coalescing in their real order
  (after ranking, before seal).
- Golden parity for phrase, image, mixed and map chains, including handoff
  gaps and collision resolution.
- `visual_budget*` and `model/chain_coalesce*.go` have no production caller.
- One `compile_editorial_plan` request per plan, counter test green.
- `make verify-main` on the cutover tree.

## Risks

- Chain fusion changes the number of overlay items and the render key; a
  partial port changes the video even when the text is identical.
- Budget admission is order-sensitive; apply it before chain fusion exactly as
  the Go planner does.
