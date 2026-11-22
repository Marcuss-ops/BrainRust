# TICKET-46 — End-to-end visual recipes wiring, `MEDIA_OUTLET`, and anti-boilerplate cleanup

- Priority: P1
- Status: DONE (2026-11-22)
- Depends on: TICKET-43, TICKET-45
- Follow-ups: [TICKET-47](TICKET-47-multi-recipe-scenes-and-outlet-coverage.md), [TICKET-48](TICKET-48-unrendered-recipe-layouts.md)
- Files:
  - `semantic-core/src/entity_capabilities.rs`
  - `semantic-core/src/entity_type_terms.rs`
  - `lexicon/entity_type_terms.json`, `lexicon/tools/build_language_packs.py`
  - `visualner/src/analysis.rs`, `visualner/src/extractor/values.rs`
  - `semantic-engine/src/consolidate/recipes.rs`, `semantic-engine/tests/visual_recipes_46.rs`
  - `overlay-compiler/src/editorial_plan_v2/planner/visual_recipes.rs`
  - `overlay-compiler/src/editorial_plan_v2/request/validation.rs`
  - `contracts/overlay-v2/fixtures/brainrust/visual_recipes.request.json`
  - `refactored/internal/kernel/entitytype/entitytype.go`
  - `refactored/internal/kernel/overlayplan/v2.go`
  - `refactored/internal/capabilities/scripts/overlay_plan_raw.go`
  - `refactored/internal/platform/media/rustexec/semantic_engine_*`

## Problem

TICKET-45 emitted six grounded recipes, but the chain stopped before the
renderer: `RawSceneV2` carried no recipe, the Go producer never forwarded the
payload, the planner validated-and-ignored it, and RenderingGen had no idea the
strings existed. TICKET-43 added ten entity types that four of the five
taxonomy copies knew about and two did not. The result was correct data that
never reached a screen.

## What landed

### WS1 — one taxonomy table, `MEDIA_OUTLET` as data

- `semantic_core::entity_capabilities::entity_capabilities(kind)` is now the
  single membership table (`visual_subject`, `named_subject`,
  `co_mention_subject`, `named_kind`, `visual_query`, `bullet_signal`,
  `media_outlet`). It replaces the five duplicated `matches!` lists:
  `bullets::signal_kind_for_entity_type`, VisualNER's co-mention filter, the
  identity registry's named kinds, `groups::{is_visual_entity,
  is_named_subject}` and `candidates/contextual/helpers::is_visual_query_entity`.
  The capability sets are pinned by a test, so widening one of them is a
  deliberate, visible change.
- `MEDIA_OUTLET` is a typed vocabulary entry for all 11 languages. Outlet names
  are full proper names, so they are matched **exactly** (lowercased,
  punctuation-trimmed) before the institutional marker. The head-noun matcher
  never returns this type: `"Repubblica"` alone stays ORG, `"la Repubblica"` is
  the paper.
- Go folds `MEDIA_OUTLET` onto `Org`, so identity, catalog and image binding stay
  deterministic. The editorial distinction travels in the recipe payload, not in
  a new canonical family.

### WS2 — two recipes with no second NLP pass

- `quote_attribution`: joins the quotes `quote_candidates` already grounds with
  a same-sentence PERSON/ORG identity (nearest first, people before
  organizations) and carries the speaker role when `person_roles` has it. An
  unattributed quote emits nothing.
- `source_highlight`: joins a `MEDIA_OUTLET` mention with a same-sentence quote,
  or with the phrase that carries an existing attribution cue
  (`lexicon/discourse_cues.json::attribution`). No cue, no ordinary
  organization, no recipe.
- Both reuse `candidate(...)`, so span grounding and the `visual_recipe.v1`
  schema stay the same as TICKET-45.

### WS3 — table-driven lowering, no new `ItemKind`

`planner/visual_recipes.rs` lowers a recipe onto templates RenderingGen already
certifies:

| Recipe | Effect | Template |
| --- | --- | --- |
| `counter_odometer` | enriches the existing number item (value line + subject line), seeded `METRIC_MOTIONS`, 110 px no-shrink | `METRIC_STAT_CARD` |
| `role_lower_third` | the one new item; timing from the canonical entity occurrence | `LOWER_THIRD` |
| `quote_attribution` | appends `— Author, role` to the existing quote | `QUOTE` |
| `source_highlight` | pins `doc_04_archival_source_highlight` on the phrase/quote that names the outlet | `IMPORTANT_PHRASE` / `QUOTE` |
| `route_map` | reorders the scene's existing map items to waypoint order | `MAP` |

A recipe that cannot find its grounded item is a no-op, never a duplicate card.
`quote_attribution` and `source_highlight` are now in the validation whitelist.
The cross-repo protocol fixture gained one scene per new recipe, so the
contract corpus exercises both.

### WS4 — Go is a pass-through

- `overlayplan.RawScene` carries `visual_recipe` / `visual_payload`
  (`omitempty`; frozen requests stay byte-identical).
- `buildRawOverlayScenes` forwards the highest-priority supported recipe of each
  scene. Recipes the planner does not lower (relationship graph, choropleth,
  lifespan) are not shipped at all.
- `rustexec` decodes `visual_payload` into
  `SemanticProductionCandidate.VisualPayload` and copies it verbatim.
- `SemanticOverlayResolver` gained the upper-case `LOWER_THIRD` spelling.

## Verification (2026-11-22)

- `cargo test -p semantic-core --lib entity_capabilities entity_type_terms` — 8 passed.
- `cargo test -p visualner --lib media_outlets` — 1 passed.
- `cargo test -p semantic-engine --test visual_recipes_46` — 6 passed.
- `cargo test -p overlay-compiler --lib visual_recipes` — 12 passed.
- `cargo test -p overlay-compiler --test overlay_plan_contract the_cross_repo_protocol_fixtures` — 1 passed (8 recipes).
- `go test ./internal/capabilities/scripts ./internal/kernel/entitytype ./internal/platform/media/rustexec ./internal/kernel/overlayplan ./internal/capabilities/overlays` — all pass.
- `go build` on the same packages — clean.

## Not in scope (ticketed)

- More than one recipe per scene: the wire still carries one, and Go picks by
  priority. See TICKET-47.
- Bare outlet names, outlet domains and the VLM identity gate. See TICKET-47.
- Layouts for `relationship_graph`, `choropleth_value_map`, `lifespan_timeline`:
  still validated and intentionally inert. See TICKET-48.
