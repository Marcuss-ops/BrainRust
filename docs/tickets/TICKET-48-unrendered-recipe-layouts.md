# TICKET-48 — Decide the fate of the three unrendered recipe layouts

- Priority: P3
- Status: DONE (2026-11-22)
- Depends on: TICKET-46

## Problem

`relationship_graph`, `choropleth_value_map` and `lifespan_timeline` are
grounded, validated and carried by the contract corpus, but no renderer layout
exists for them. After TICKET-46 they are also deliberately **not forwarded** by
the Go producer, because shipping a payload the planner ignores is dead wire
data.

That leaves the three recipes in a defensible but temporary state: valid in the
contract, invisible in production.

## Options

1. **Build the layouts.** They need node/edge geometry (relationship graph),
   polygon fills (choropleth) or a two-marker span (lifespan). None maps onto an
   existing `ItemKind`; each would be a new certified layout with its own
   admission and overlap policy. This is the expensive option and should be
   justified by a measured share of scripts that contain the triggering facts.
2. **Keep them validated-but-inert.** Zero cost, but the contract advertises a
   capability production does not have.
3. **Remove them from the validator.** The recipes would fail closed at the
   contract boundary instead of being silently dropped later.

## Acceptance

- Measure how many of the 2026-11-22 job scripts produce each recipe, using the
  semantic manifest already stored under `ops/jobs/`.
- Pick an option per recipe (they do not have to agree) and record the reason.
- If option 3: update `request/validation.rs`, the protocol fixture values, the
  TICKET-45/46 docs and `docs/architecture/overlay-contract-v2.md` together.
- If option 1: follow the TICKET-46 rule — reuse a certified `ItemKind` if one
  exists; otherwise the new layout needs a RenderingGen certification harness
  before the planner emits it.

## Resolution — 2026-11-22

Measured on the **1,173 unique scenes** archived under `ops/jobs/**/*.json`
(the `augias_aftermath_20261122` directories are extraction-only and carry no
script): `relationship_graph` **3**, `lifespan_timeline` **1**,
`choropleth_value_map` **0**. Even together they are under 0.4% of scenes, and
the graph cases do not establish that node/edge geometry would render well.
The archived `semantic_consolidation` payloads still use the pre-TICKET-45
candidate names, so these are replay counts over the archived scene texts on
this engine revision (same 1,173-scene denominator as TICKET-47), not a count
of stored candidates.

Decision: **option 2, validated-but-inert, for all three.** Building a layout
for a recipe that fires three times in 1,173 scenes would add a new certified
layout, admission/overlap policy and RenderingGen harness for no measured
production demand. Removing them from the validator (option 3) would discard
three grounded, contract-tested facts and change frozen protocol values for no
user-visible gain.

Consequence, recorded so the contract does not overpromise: the three recipes
remain valid at the `overlay-compiler` boundary and keep their protocol
fixtures, while the Go producer still does not forward them (a planner would
ignore the payload). Both halves are pinned: `visual_recipes.request.json`
exercises all eight recipes through validation, and
`layout_only_recipes_stay_validated_but_inert` asserts that the planner neither
adds nor mutates an item for the three (the planner names them in an explicit
drop arm instead of relying on a catch-all). If a future corpus shows meaningful demand, reopening this
ticket means option 1: reuse a certified `ItemKind` if possible, otherwise
certify the new layout first. No new taxonomy was introduced.

## Non-goal

Adding a fourth parallel taxonomy of visual kinds in Go. Any layout that lands
must stay driven by the existing `template_id` registry.
