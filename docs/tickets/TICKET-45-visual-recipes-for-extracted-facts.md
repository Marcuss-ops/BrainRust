# TICKET-45 — Visual recipes & layout hints for already-extracted semantic facts

- Priority: P2
- Status: DONE (2026-11-22)
- Depends on: TICKET-30, TICKET-32, TICKET-33, TICKET-42
- Files: `semantic-engine/src/proposals.rs`,
  `semantic-engine/src/groups.rs`,
  `semantic-engine/src/consolidate/producers.rs`,
  `semantic-core/src/manifest.rs`,
  `overlay-compiler/src/editorial_plan_v2/` (`request/`, `projection.rs`, `planner/`),
  `contract/`

## Problem

`semantic-engine` already extracts rich structured facts that currently have
no dedicated visual recipe (`ProductionCandidate.recipe` / `suggested_visual`)
or `overlay-compiler` planner lane:

| Already extracted in BrainRust | Current visual projection | Missing visual recipe / hint (matching `ChrononTemplate` where present) |
|---|---|---|
| `PersonRelation` / `AssertedRelation` (`family_relations`, `semantic-engine/src/roles.rs`) | None (entities/cards only) | `investigation_board` / `relationship_graph` (subjects + edges; `InvestigationBoardPack.hpp` already renders pinned cards + red string `connections`) |
| `PersonRole` (`role`, `organization`, `date`) | Generic `entity_card` / `lower_third` without structured role slot | `role_lower_third` (person name + grounded role/org/tenure) |
| `Route` waypoints + `place_index_v1.json` (195 countries w/ coords) | Single `map` pin | `route_map` (ordered waypoints + distance/legs) and `choropleth_value_map` (country + `Quantity`) |
| `PersonAgeFact` / birth→death `TemporalFact` pairs | Generic `timeline` or `date_card` | `lifespan_timeline` (birth → death / tenure span) |
| Single high-salience `Quantity` (not part of a ≥2-value chart) | Plain `number` text overlay or dropped | `counter_odometer` (single KPI value + unit + subject label) |

Currently `overlay-compiler/src/editorial_plan_v2` handles `text_phrase`,
`number`, `entity_card`, `entity_image`, `map`, `mixed_chain`, `image_chain`,
`phrase_chain`, `phrase`, and `LIGHT_LEAK_MOTIONS` (`presentation.rs`), while
`semantic-engine` emits `entity_search`, `entity_card`, `multiple_entities`,
`timeline`, `fact_card`, `chart`, `legal_outcome_card`,
`investigation_status_card`, `info_card`, `date_card`, and `statement_card`.
Because the missing recipes are never projected into `ProductionCandidate` or
`editorial_plan_v2`, Go and Chronon cannot schedule or animate them.

## Scope & Design

Respecting the BrainRust authority boundary (BrainRust emits grounded proposals
and layout hints; Go owns scheduling/assets; Chronon owns rendering/animation):

1. **Additive `ProductionCandidate` recipes & `suggested_visual` hints:**
   - `investigation_board` / `relationship_graph` backed by grounded `PersonRelation` / `AssertedRelation` edges (maps directly to `InvestigationBoardPack.hpp` `items` + `connections`).
   - `role_lower_third` backed by `PersonRole` (same-sentence person + role + optional org/date).
   - `route_map` backed by `Route` (≥2 ordered grounded `Place` waypoints with coordinates).
   - `choropleth_value_map` backed by ≥2 country/location-bound `Quantity` items resolved against `place_index_v1.json`.
   - `lifespan_timeline` when one grounded person has ≥2 dated life/tenure events (e.g. birth/appointment/death).
   - `counter_odometer` for a single high-salience asserted `Quantity` (currency/percent/population/count) with grounded unit and subject, when not already claimed by a chart.

2. **`overlay-compiler` (`editorial_plan.v2`) additive lane:**
   - Extend `editorial_plan_v2` request/projection/planner to preserve and
     budget-admit the new recipe payloads without breaking existing frozen
     `overlay_plan_contract` goldens (additive `#[serde(default)]` fields).

## Acceptance criteria

- [x] Each new recipe emits only from grounded facts with verbatim `evidence`
      and exact `[start_byte, end_byte)` spans; missing slots fail closed (no
      recipe emitted).
- [x] Consolidation deduplicates overlapping facts (`covered_fact_ids`) so a
      `choropleth_value_map` or `counter_odometer` does not duplicate a chart or
      plain `fact_card` for the same sentence span unless permitted by budget.
- [x] Existing `overlay_plan_contract` frozen fixtures remain byte-identical
      when new optional inputs are absent.
- [x] Unit + protocol tests cover positive and negative cases for all 6 recipes;
      `cargo fmt`, `clippy --all-targets -- -D warnings`, and Go contract sync
      pass.

## Non-goals

- Implementing Chronon shaders/keyframe animations (tracked separately in
  `Chronon3d` / `ChrononMotion3D`).
- Designing Go editorial pacing or UI controls (tracked in `refactored`).

## Verification — 2026-11-22

- `consolidate/recipes.rs` emits `relationship_graph`, `role_lower_third`, `route_map`, `choropleth_value_map`, `lifespan_timeline` and `counter_odometer` only from grounded facts, with verbatim `evidence`, exact spans, typed `visual_payload` and non-empty `covered_fact_ids`.
- `semantic-engine/tests/visual_recipes_45.rs` covers positive and missing-slot negatives for all six recipes, chart/counter dedupe, and neutral text.
- `SceneAnnotationsV2`/`RawSceneV2` carry additive `visual_recipe`/`visual_payload`; unknown recipes and non-object payloads fail closed.
- Frozen `overlay_plan_contract` fixtures remain byte-identical when the new optional inputs are absent.
