# TICKET-47 — Multi-recipe scenes, outlet coverage, and the VLM identity gate

- Priority: P2
- Status: DONE (2026-11-22)
- Depends on: TICKET-46
- Files:
  - `overlay-compiler/src/editorial_plan_v2/request/types.rs`
  - `overlay-compiler/src/editorial_plan_v2/projection.rs`
  - `overlay-compiler/src/editorial_plan_v2/planner/visual_recipes.rs`
  - `refactored/internal/capabilities/scripts/overlay_plan_raw.go`
  - `semantic-core/src/identity.rs`
  - `lexicon/entity_type_terms.json`

## 1. One recipe per scene

`RawSceneV2` / `SceneAnnotationsV2` carry a single `visual_recipe`. TICKET-46
resolved the collision by forwarding the highest-priority candidate, so a scene
that states both a role and a KPI shows only one of them. The engine already
emits both candidates.

Acceptance:

- Add one additive field
  `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub visual_recipes: Vec<VisualRecipeSpecV2>`
  (`{ recipe, payload }` for now; timing still comes from annotations/entities).
- Keep the single legacy field accepted and consumed as the first element, so
  frozen requests and the existing fixture stay byte-identical.
- Go forwards every supported candidate of the scene, ordered by priority, into
  the list; it still never interprets a payload.
- The planner applies each recipe once, with the same no-duplicate rules
  (`counter_odometer` must not enrich the same number twice; two role cards for
  one person must collapse).
- A frozen-fixture test proves that an absent list serializes exactly as before.

## 2. Outlet coverage

`MEDIA_OUTLET` matches the *full* surface exactly. In Italian news the article
is often dropped ("Repubblica ha pubblicato…") and web mentions arrive as
domains (`notizie.it`, `ilfattoquotidiano.it`), so a real share of sources is
still typed ORG or CONCEPT.

Acceptance:

- Measure the miss rate on the existing 2026-11-22 job scripts
  (`ops/jobs/augias_aftermath_20261122/`) before changing the matcher: count
  mentions that name a known outlet but are not typed `MEDIA_OUTLET`.
- Only then choose one of: an inflected/articled alias table per outlet, or a
  domain normalizer in the extractor. Do not add suffix matching: `"stampa"`,
  `"repubblica"`, `"tempo"`, `"sabah"` are ordinary words.
- Positive/negative tests in all 11 languages, including the bare-article case
  and a domain-vs-prose case.

## Resolution — 2026-11-22

### Measurement before the change

The six `ops/jobs/augias_aftermath_20261122/` directories are YouTube extraction
jobs: they contain no script or semantic manifest, so they cannot measure
recipes. The measurement used the request/manifest pairs already archived under
`ops/jobs/**/*.json`: **1,173 unique scenes**. Current engine output:

| Recipe | Scenes |
| --- | ---: |
| `quote_attribution` | 83 |
| `role_lower_third` | 64 |
| `counter_odometer` | 53 |
| `source_highlight` | 4 |
| `relationship_graph` | 3 |
| `lifespan_timeline` | 1 |
| `choropleth_value_map` | 0 |
| `route_map` | 0 |

Five scenes carry two or more recipes, so the single field was losing real
information, not just theoretical capacity.

Outlet surfaces in the same corpus: **21**, all non-`MEDIA_OUTLET` — 10 ORG
(`ansa` without the article) and 11 VISUAL_CONCEPT (`stampa`, `zeit`, `monde`,
`mundo`, `welt`, `libération`). No domain mention appeared in that corpus, so
the domain path is exercised by tests, not by this sample.

### 1. Multi-recipe scenes

`visual_recipes: [{recipe, payload}]` is additive; the legacy `visual_recipe`
stays authoritative as the first element and an empty list is omitted, so the
frozen request bytes do not change. Both `annotations` and `raw` accept up to
eight recipes, each validated by the shared closed check. The planner applies
every recipe once: the counter skips a metric already carrying two rows, quote
skips a line already ending in an attribution (`\n— `), role dedupes per person,
source/route payloads are idempotent, and the legacy+first-list duplicate is not
applied twice. Go forwards every supported candidate ordered by priority and
rank, without interpreting the payload. Evidence: 16 planner tests, the
`overlay_plan_contract` fixture `visual_recipes.request.json` scene `s7` now
carries a two-recipe list, and the wire tests in `overlay_plan_raw_test.go`.

### 2. Outlet coverage

Decision: **domain normalizer + only unambiguous article-dropped aliases**.
No suffix matching. Domains live in the lexicon as
`media_outlet_domains` (11 languages, curated hosts such as `ansa.it`,
`notizie.it`, `fanpage.it`) and are matched whole, case-insensitively and
boundary-checked, so `redazione@ansa.it` and `example.com` never qualify.
Aliases were added only where the dropped form is distinctive (`fatto
quotidiano`, `sole 24 ore`, `guardian`, `new york times`, `figaro`,
`telegraaf`, `volkskrant`); ambiguous heads (`repubblica`, `stampa`, `zeit`,
`monde`, `welt`, `mundo`) still require the article. Tests cover all 11
language tables, domain-vs-prose, the email boundary and the bare-article
negatives.

### 3. VLM identity gate

Decision: **keep the narrower policy**. `role_of` now returns `NamedEntity` for
`MEDIA_OUTLET` (it is an organization for identity and image binding) and
continues to return `VisualConcept` for the TICKET-43 additive types. The
comment on the function points at `entity_capabilities`' `named_kind` and the
test `named_and_generic_kinds_get_the_right_role` asserts the difference for
all ten additive types, so reading the SSOT flag later cannot silently create
name cards for animals or diseases. Entity-linker tests pass unchanged.
