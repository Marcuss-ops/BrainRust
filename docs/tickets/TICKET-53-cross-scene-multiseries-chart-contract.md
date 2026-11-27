# TICKET-53 — Plan-contract fields for cross-scene and multi-series charts

- Status: PROPOSED (2026-11-27)
- Priority: P1
- Depends on: TICKET-41, TICKET-42, TICKET-52
- Unblocks: TICKET-52 items 4 (cross-scene charts) and 5 (multi-series charts)
- Scope: contract and validation design. No renderer implementation.

## 1. Why the current contract cannot express these charts

The current chart lane is single-scene and single-series by construction:

| Layer | Current field | Limit |
|---|---|---|
| semantic manifest | `ChartItem.start_byte/end_byte` | offsets are relative to the proposal's one `scene_id` |
| semantic manifest | `ChartMetadata.metric/unit/dimension/period` | one metric, one unit, one period for the whole candidate |
| semantic manifest | `ChartMetadata.series_type` | `time` or `category`, but no series identity |
| production candidate | `ProductionCandidate.scene_id` + one `evidence` | one anchor span; `scene_ids` exists but charts only fill one id |
| plan request | `RawSceneV2.chart_values: Vec<String>` + `chart_layout` | values are bare strings, one list per scene, no label/value/period/anchor |
| plan response | `PlanItemV2.params.chart_card/chart_bar` + `chart_group_id` | grouping is opaque; no series id, no per-point source |
| renderer | no capability descriptor | the planner cannot know whether a layout is certified |

Adding more values to `chart_values` cannot fix this: it loses the series a
value belongs to, the scene it came from, and the exact source span. A
cross-scene chart needs one timing anchor plus many provenance anchors; a
multi-series chart needs one timing window plus a series identity per point.

## 2. Design rules

1. **Additive, not a schema break.** Every new field is optional or defaults to
   the legacy meaning. Frozen `editorial_plan.v2` requests and responses must
   serialize byte-identically when the new fields are absent.
2. **One timing authority.** The producer supplies the source timing of each
   point; the planner owns the chart's placement window inside the anchor
   scene. The renderer never re-derives timing.
3. **Grounding is per point, not per candidate.** Every point names its scene
   and exact `[start_byte, end_byte)`; `text` must equal that slice.
4. **Series identity is explicit.** No inference from array position.
5. **Layouts are capability-gated.** A cross-scene or multi-series layout is
   emitted only when the request declares a certified capability for it.
6. **Fail closed on malformed charts, fail soft on uncertified layouts.**
   A bad chart object rejects the request; a layout the renderer did not
   certify is skipped with a decision reason, like any other unavailable lane.
7. **No invented values, labels, periods or missing points.** A gap stays a
   gap; a series with fewer points stays shorter.

## 3. Semantic manifest fields

File: `semantic-core/src/manifest/entities.rs`.

```rust
/// One comparable numeric value inside a chart proposal.
pub struct ChartItem {
    // existing: label, text, kind, value, start_byte, end_byte, qualifier,
    //           period, lower, upper, range_id

    /// Scene that owns `start_byte/end_byte`. Empty means the candidate's
    /// anchor scene, so every pre-existing item keeps its meaning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,

    /// Stable id of the series this point belongs to inside the candidate.
    /// Empty means the candidate's only series (legacy single-series).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
}

/// Series identity and scale shared by its points.
pub struct ChartSeries {
    pub id: String,                    // "series_revenue_alfa"
    pub label: String,                 // grounded display label
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,     // canonical id when the subject is an entity
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimension: Option<String>,
    /// "left" (default) or "right" for a certified dual-axis layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
}

pub struct ChartMetadata {
    // existing: metric, unit, dimension, period, series_type

    /// Present only for multi-series charts. Empty for single-series, so the
    /// legacy metadata stays unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<ChartSeries>,

    /// X keys shared by every series, in display order. Empty means the
    /// legacy time/category derivation from the items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub x_keys: Vec<String>,
}
```

`ChartItem.scene_id` scopes the byte offsets. When it is absent, the offsets are
relative to `ProductionCandidate.scene_id`; when present, they are relative to
that scene's text. `end_byte` is exclusive.

File: `semantic-core/src/manifest.rs`.

```rust
/// One verbatim evidence span in one source scene.
pub struct SceneEvidence {
    pub scene_id: String,
    pub evidence: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

pub struct ProductionCandidate {
    // existing fields...

    /// Every source span backing a multi-scene candidate, in timeline order.
    /// Empty for single-scene candidates, whose `scene_id`/`evidence` pair
    /// remains authoritative.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_evidence: Vec<SceneEvidence>,
}
```

For a cross-scene candidate:

- `scene_id` is the **anchor** scene: the scene that owns the card's text and
  timing window.
- `scene_ids` lists every contributing scene once, in timeline order.
- `scene_evidence` has exactly one entry per id in `scene_ids`, and the anchor
  entry must equal `(scene_id, evidence, start_byte, end_byte)`.
- `chart_items[*].scene_id` is one of `scene_ids`; absent means the anchor.
- `chart_meta.series` is empty only when the chart is single-series; otherwise
  it has >= 2 entries and every nonzero `chart_items[*].series_id` resolves.

## 4. Plan request fields

Files: `overlay-compiler/src/editorial_plan_v2/request/types.rs` and
`request/types/raw.rs`.

### 4.1 Version switch

```rust
pub struct EditorialPlanV2Request {
    // existing fields...

    /// 0 (default) keeps the per-scene `chart_values` / `chart_layout` lane.
    /// 1 makes `charts` the only structured chart source and forbids the
    /// legacy arrays, so one candidate cannot be read twice.
    #[serde(default)]
    pub chart_contract_version: i64,

    /// Structured charts, including cross-scene and multi-series. Empty is
    /// valid and means "no structured chart".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub charts: Vec<ChartPlanV2>,

    /// Certified renderer capabilities keyed by layout id. Empty means only
    /// the pre-existing single-series layouts may be selected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chart_layouts: BTreeMap<String, ChartLayoutCapabilityV2>,
}
```

`chart_layouts` is deliberately at request level, not inside `canvas`: it is a
renderer certification fact, like the motion pools, and must not be inferred by
the planner.

### 4.2 Chart object

```rust
pub struct ChartPlanV2 {
    /// Stable id; equals the production candidate `dedup_key`.
    pub id: String,

    /// "chart" or "delta_comparison".
    pub recipe: String,

    /// Must be a key in `chart_layouts`.
    pub layout: String,

    /// Scene that owns the card and the placement window.
    pub anchor_scene_id: String,

    /// Every contributing scene, in timeline order, without duplicates.
    /// Length >= 2 for a cross-scene chart.
    pub scene_ids: Vec<String>,

    /// Absolute placement window `[start_us, end_us)` in plan time. Must lie
    /// inside the anchor scene's `[timeline_start_us, timeline_start_us +
    /// duration_us)` and be at least `ChartMinWindowUS` long.
    pub window_us: [i64; 2],

    /// One verbatim source span per contributing scene.
    pub evidence: Vec<ChartEvidenceV2>,

    /// Series in display order. Length >= 2 for a multi-series layout.
    pub series: Vec<ChartSeriesV2>,

    /// All points of all series. Every point is owned by exactly one series.
    pub points: Vec<ChartPointV2>,

    pub axes: ChartAxesV2,
}

pub struct ChartEvidenceV2 {
    pub scene_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,          // == scene.text[start..end]
}

pub struct ChartSeriesV2 {
    pub id: String,            // unique inside the chart
    pub label: String,         // grounded, never generated
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entity_id: String,     // canonical id when grounded
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub metric: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dimension: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub axis: String,          // "" | "left" | "right"

    /// Indices into `ChartPlanV2.points`, in x order. Indices are unique
    /// across all series: one point belongs to exactly one series.
    pub points: Vec<usize>,
}

pub struct ChartPointV2 {
    pub scene_id: String,
    pub start_byte: usize,
    pub end_byte: usize,

    /// Verbatim numeric surface, e.g. "95 milioni".
    pub text: String,

    /// Normalized x key shared with the other series ("2023", "Q2-2024",
    /// "north"). Must match one of `axes.x_keys`.
    pub x_key: String,

    /// Human label for that x key ("2023", "Nord").
    pub x_label: String,

    pub value: f64,
    pub kind: String,          // NUMBER | PERCENT | MONEY

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub qualifier: String,     // exact | approximate | forecast | ...
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub period: String,        // normalized period when grounded
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,

    /// Absolute source timing. Used for ordering and evidence; the bar's
    /// on-screen window is `ChartPlanV2.window_us`, not this value.
    pub start_us: i64,
    pub duration_us: i64,
}

pub struct ChartAxesV2 {
    /// "time" or "category".
    pub x_kind: String,

    /// Shared x domain, in display order. Every `ChartPointV2.x_key` must be
    /// a member, and every key must have at least one point.
    pub x_keys: Vec<String>,

    pub y_unit: String,
    pub y_dimension: String,

    /// Empty means "derive from the admitted points"; never zero-filled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_max: Option<f64>,

    /// True only when the certified layout normalizes each series to 0..1.
    #[serde(default)]
    pub normalize: bool,
}

pub struct ChartLayoutCapabilityV2 {
    pub max_series: i64,
    pub max_points_per_series: i64,
    pub max_total_points: i64,
    /// "time" or "category".
    pub x_kind: String,
    /// "shared" (one y axis) or "dual" (left/right).
    pub axes: String,
    /// Grouped layouts require every series to carry the same x key set.
    pub requires_aligned_x: bool,
}
```

### 4.3 Example: cross-scene, two series

```json
{
  "chart_contract_version": 1,
  "chart_layouts": {
    "grouped_bar_comparison": {
      "max_series": 2,
      "max_points_per_series": 6,
      "max_total_points": 12,
      "x_kind": "category",
      "axes": "shared",
      "requires_aligned_x": true
    }
  },
  "charts": [
    {
      "id": "chart:revenue:alfa,beta",
      "recipe": "chart",
      "layout": "grouped_bar_comparison",
      "anchor_scene_id": "scene_04",
      "scene_ids": ["scene_02", "scene_04", "scene_07"],
      "window_us": [182000000, 187000000],
      "evidence": [
        {"scene_id": "scene_02", "start_byte": 0, "end_byte": 49, "text": "Nel 2023 Alfa ha registrato 80 milioni di ricavi."},
        {"scene_id": "scene_04", "start_byte": 0, "end_byte": 46, "text": "Nel 2024 Alfa ha raggiunto 95 milioni."},
        {"scene_id": "scene_07", "start_byte": 0, "end_byte": 45, "text": "Nel 2024 Beta ha chiuso a 70 milioni."}
      ],
      "series": [
        {"id": "alfa", "label": "Alfa", "entity_id": "org_01", "metric": "revenue", "unit": "EUR", "dimension": "currency", "points": [0, 1]},
        {"id": "beta", "label": "Beta", "entity_id": "org_02", "metric": "revenue", "unit": "EUR", "dimension": "currency", "points": [2]}
      ],
      "points": [
        {"scene_id": "scene_02", "start_byte": 28, "end_byte": 38, "text": "80 milioni", "x_key": "2023", "x_label": "2023", "value": 80000000, "kind": "MONEY", "period": "2023", "start_us": 1000000, "duration_us": 900000},
        {"scene_id": "scene_04", "start_byte": 29, "end_byte": 39, "text": "95 milioni", "x_key": "2024", "x_label": "2024", "value": 95000000, "kind": "MONEY", "period": "2024", "start_us": 2000000, "duration_us": 900000},
        {"scene_id": "scene_07", "start_byte": 27, "end_byte": 37, "text": "70 milioni", "x_key": "2024", "x_label": "2024", "value": 70000000, "kind": "MONEY", "period": "2024", "start_us": 3000000, "duration_us": 900000}
      ],
      "axes": {
        "x_kind": "category",
        "x_keys": ["2023", "2024"],
        "y_unit": "EUR",
        "y_dimension": "currency",
        "normalize": false
      }
    }
  ]
}
```

## 5. Plan response fields

File: `overlay-compiler/src/editorial_plan_v2/response.rs`.

```rust
pub struct PlanItemV2 {
    // existing fields...

    /// Structured chart role. Absent for every non-chart item and for frozen
    /// legacy chart items, which keep `params.chart_card/chart_bar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<ChartItemV2>,
}

pub struct ChartItemV2 {
    pub chart_id: String,
    pub layout: String,

    /// "card" | "bar" | "axis" | "legend".
    pub role: String,

    /// Empty for the card/axis; required for a bar.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub series_id: String,

    /// Index into `ChartPlanV2.points`; -1 for non-bar roles.
    #[serde(default)]
    pub point_index: i64,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub x_key: String,

    /// Verbatim display text of the source value.
    pub display_value: String,
    pub value: f64,

    /// Provenance. Timing is the item's own `start_us/duration_us`.
    pub source: ChartSourceV2,
}

pub struct ChartSourceV2 {
    pub scene_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
}
```

Rules for lowering:

- One `card` item carries the chart title and the full placement window; it is
  anchored to `anchor_scene_id`.
- One `bar` item per point carries `role="bar"`, its `series_id`, `point_index`,
  `display_value` and `value`. All bars share the card's window.
- `params.chart_card` / `params.chart_bar` / `params.chart_group_id` are still
  written (`chart_group_id = chart_id`) so the existing Go finalizer, dedupe and
  coalesce logic keep working without a second code path.
- `kind` stays `text_phrase`; the renderer must not need a new primitive kind.
- `chart_cards` in `PlanMetrics` counts `role="card"` items; `chart_placements`
  counts distinct `chart_id` values admitted, not bars.

## 6. Invariants and validation matrix

### Request boundary (fail closed)

| Check | Rejection |
|---|---|
| `chart_contract_version` not 0 or 1 | invalid version |
| version 1 and any scene has `raw.chart_values` or `raw.chart_layout` | mixed chart sources |
| duplicate `ChartPlanV2.id` | duplicate chart id |
| `anchor_scene_id` not in `scene_ids` or unknown scene | bad anchor |
| `scene_ids` not unique / not in request scene order | bad scene order |
| `evidence.len() != scene_ids.len()` | missing evidence |
| evidence text not equal to its scene slice | ungrounded evidence |
| point scene not in `scene_ids` | orphan point |
| point text not equal to its scene slice | ungrounded point |
| point byte span outside the scene text | bad span |
| non-finite `value`, `y_min`, `y_max` or `range` | non-finite number |
| duplicate `(scene_id, start_byte, end_byte)` across points | reused point |
| duplicate series id, empty series, duplicate point index | bad series ownership |
| point index out of range / point not owned by a series | bad point reference |
| series unit/dimension/metric differ inside one series | incompatible series |
| `x_key` not in `axes.x_keys`, or an x key with no point | unaligned x domain |
| grouped layout with `requires_aligned_x` and unequal x key sets | unaligned series |
| series count or point count over the layout capability | over capability |
| `layout` not in `chart_layouts` | uncertified layout |
| window outside the anchor scene or shorter than `ChartMinWindowUS` | bad window |
| cross-scene chart with `< 2` scenes | not cross-scene |
| multi-series layout with `< 2` series | not multi-series |

### Planner (fail soft)

- A chart whose layout is absent from `chart_layouts` is skipped and reported
  as `uncertified_chart_layout`; it must not fall back to `bar_comparison`,
  because that would silently drop a series.
- A chart that collides with an admitted chart in the anchor scene loses by
  priority, exactly like the current chart lane.
- A chart that cannot fit its window loses; it is never stretched into another
  scene, because the card text belongs to the anchor scene.

### Response boundary

- Every `chart_id` group has exactly one `card`; every non-card item has a
  valid `series_id` and `point_index`.
- The number of `bar` items equals the number of points.
- Every `source` is a verbatim slice of its scene; the item window equals the
  card's window.
- No two admitted chart groups share the anchor scene window.

## 7. Compatibility and migration

| Phase | Producer | Planner | Renderer |
|---|---|---|---|
| 0 (today) | `scene.raw.chart_values` | legacy `chart_numbers` lane | single-series bars |
| 1 | adds `ChartItem.scene_id/series_id`, `ChartMetadata.series`, `ProductionCandidate.scene_evidence` | ignores them | unchanged |
| 2 | emits `chart_contract_version=1` + `charts`, stops filling `chart_values` | builds `ChartItemV2`, validates, lowers | reads typed `chart`, falls back to params |
| 3 | — | selects cross-scene/multi-series only from `chart_layouts` | certifies layouts and publishes capabilities |

Rollback: set `chart_contract_version=0`, drop `charts` and
`chart_layouts`; the legacy bytes are unchanged. No schema-version bump is
required because every new field defaults to the legacy meaning. Bump to
`editorial_plan.v3` only if a future field changes the meaning of an existing
one (this design does not).

## 8. Implementation map

| Layer | File | Change |
|---|---|---|
| semantic types | `semantic-core/src/manifest/entities.rs` | `ChartItem.scene_id/series_id`, `ChartSeries`, `ChartMetadata.series/x_keys` |
| semantic types | `semantic-core/src/manifest.rs` | `ProductionCandidate.scene_evidence` |
| engine | `semantic-engine/src/groups/charts.rs`, `groups/charts/series.rs`, `groups/charts/metadata.rs` | fill series id/label and per-item scene; add the cross-scene builder that groups by `(entity_id, metric, unit)` |
| consolidation | `semantic-engine/src/consolidate/producers.rs` | preserve all scene ids/evidence; never truncate a multi-scene candidate to its anchor |
| request | `overlay-compiler/src/editorial_plan_v2/request/types.rs` | `chart_contract_version`, `charts`, `chart_layouts` and the structs above |
| validation | `overlay-compiler/src/editorial_plan_v2/request/validation.rs` | the matrix in section 6 |
| projection | `overlay-compiler/src/editorial_plan_v2/projection.rs` | version 0 keeps `apply_chart_values`; version 1 projects `charts` |
| planner | `overlay-compiler/src/editorial_plan_v2/planner/candidates/editorial.rs` | emit typed `ChartItemV2` plus legacy params; enforce capability |
| response | `overlay-compiler/src/editorial_plan_v2/response.rs` | `PlanItemV2.chart`, `ChartItemV2`, `ChartSourceV2` |
| Go wire | `refactored/internal/kernel/overlayplan/v2.go` | mirror the new request/response fields |
| Go producer | `refactored/internal/capabilities/scripts/overlayplan/overlay_plan_raw.go` | build `charts` from production candidates; stop writing `chart_values` under version 1 |
| Go decode | `refactored/internal/platform/media/rustexec/semantic_engine_*.go` | decode/forward `charts`, `chart_layouts`, `PlanItemV2.chart` |
| Go finalizer | `refactored/internal/capabilities/scripts/overlayplan/overlay_plan_finalize.go` | validate group completeness; keep `chart_group_id` compatibility |
| renderer | Chronon/ChrononTemplate (external) | implement and certify `cross_scene_timeline`, `grouped_bar_comparison`, `multi_line_series` |

The algorithmic work that is not in the contract is the cross-scene grouping
itself: values from different scenes form one series only when their canonical
subject, metric and unit match, and their `x_key` values are strictly ordered.
That is a producer concern; the contract only makes the result expressible and
validatable.

## 9. Initial certified limits

Proposed first release, deliberately conservative:

| Layout | Series | Points/series | Total | Axes | X |
|---|---|---|---|---|---|
| `cross_scene_timeline` | 1 | 2..12 | 12 | shared | time |
| `grouped_bar_comparison` | 2 | 2..6 | 12 | shared | category |
| `multi_line_series` | 2..3 | 2..12 | 12 | shared | time or category |

`> 6` points per series remains TICKET-52 item 12 and is not certified here.
Dual-axis layouts are deferred: `axis` is reserved but `axes` must be `shared`
until a dual-axis capability is certified.

## 10. Tests and fixtures

1. **Request validation unit tests** (`overlay-compiler`, request tests):
   one valid cross-scene chart, one valid two-series chart, and one rejection
   per row of the matrix.
2. **Planner golden**: two companies across three scenes, grouped bars; assert
   two series, three x keys, one card, one bar per point, all bars share the
   card window, and `chart_placements == 1`.
3. **Cross-scene single series**: one company across three scenes; assert the
   anchor scene owns the card and all three provenance spans survive.
4. **Backward compatibility**: a frozen version-0 request and response must be
   byte-identical before and after the change.
5. **Go round-trip**: producer → request JSON → Rust plan → Go decode; assert
   `charts` and `PlanItemV2.chart` survive and `chart_values` is empty under
   version 1.
6. **Renderer conformance** (external, blocked here): golden frames for each
   certified layout and a negative test that an uncertified layout is never
   emitted.

## 11. What this design does not unblock

- Rendering the new layouts still needs Chronon/ChrononTemplate work and a
  certified capability descriptor; the planner will refuse to emit a layout
  that is not declared.
- Editorial interest scoring and >6-point series remain independent blockers
  (TICKET-52 items 10 and 12).
- Progressive reveal still needs word-level attribute timing, which is not
  part of the chart contract.
