# TICKET-42 — Dynamic chart layouts, `QuantityComparison` delta bridges & ranking bars

- Priority: P1
- Status: OPEN (created 2026-11-22)
- Depends on: TICKET-30, TICKET-41
- Files: `semantic-engine/src/groups/charts.rs`,
  `semantic-engine/src/chart_rules.rs`,
  `semantic-engine/src/groups.rs`,
  `semantic-engine/src/consolidate/producers.rs`,
  `overlay-compiler/src/editorial_plan_v2/planner/mod.rs`

## Problem

1. **Layout hardcoded to `bar_comparison` despite 9 `ChrononTemplate` chart recipes:**
   Both single-sentence (`charts.rs:137`) and cross-sentence (`charts.rs:370`)
   chart emission hardcode `suggested_layout: Some("bar_comparison")`. Yet
   `ChrononTemplate/include/chronontemplate/charts/ChartsPack.hpp` already
   implements 9 semantic `ChartStyle` recipes (`dual_comparison_bars`,
   `timeline_growth`, `ranked_bars`, `scenario_split`,
   `bounded_range_comparison`, `qualified_growth`, `missing_data_timeline`,
   `segmented_total_bar`, `before_after_metric`) whose docstring explicitly
   states *"BrainRust owns eligibility and normalization decisions"*. BrainRust
   already detects years, shares, approximators, scenarios and ranges, yet
   collapses everything into `"bar_comparison"`.

2. **`QuantityComparison` (TICKET-30) is unused by chart proposals:**
   `SceneManifest.quantity_comparisons` extracts `ratio`, `difference`, and
   `change` (e.g. `+12%`, `il doppio rispetto al 2018`, `triplicato`), but
   `charts.rs` only bridges `Quantity` points (`charts.rs:158-197`) and ignores
   `QuantityComparison`. Before/after or growth comparisons do not produce a
   structured delta/ratio visual proposal.

3. **Ranked lists (Top-N) are vetoed with no visual fallback:**
   `chart_rules.rs:322-335` explicitly vetoes `top `, `rank`, `classifica`,
   `ranking` in `has_series_context` so rank numbers aren't misread as chart
   values. While that prevents wrong bars (tested in `charts_numeric`), valid
   ranked metric lists (e.g. Top 5 countries by GDP/revenue) currently get no
   chart/ranking layout hint at all, and `overlay-compiler` caps
   `count_chart_placements` to 2..6 values (`planner/mod.rs:449-453`).

## Scope & Design

1. **Deterministic `suggested_layout` selection aligned to `ChartsPack.hpp` (keeping `"bar_comparison"` as legacy fallback):**
   - `timeline_growth`: when `chart_items.len() >= 2` and labels are
     monotonically ordered 4-digit years / normalized dates for the same
     metric (`qualified_growth` when any item carries an approximator/bound
     qualifier; `missing_data_timeline` when an explicit missing period is
     grounded).
   - `segmented_total_bar`: when `kind == "PERCENT"`, `2..=6` items represent
     parts of a share/quota with explicit share/total context (`sum <= 100.5`).
   - `scenario_split`: when explicit hypothetical/scenario cues (`scenario`,
     `ipotesi`, `forecast`) split observations.
   - `bounded_range_comparison`: when `Quantity` pairs share `range_id`
     (`range_start` / `range_end`, e.g. `"tra X e Y"`).
   - `dual_comparison_bars` / `bar_comparison`: default for multi-category
     comparisons.

2. **Bridge `QuantityComparison` (`change` / `ratio` / `difference`) to `before_after_metric`:**
   - Emit a grounded `before_after_metric` visual candidate when a
     `QuantityComparison` links two grounded `Quantity` items (or one quantity
     + explicit delta surface) with exact byte spans and compatible
     dimension/unit.

3. **Ranking bars (`ranked_bars`):**
   - When a `SemanticGroup` of `group_type == "ranking"` has `2..=10` members
     whose text carries grounded same-unit `Quantity` values (distinct from the
     rank index itself, respecting the `charts_numeric` Top-10 rank-veto test),
     emit a `ranked_bars` candidate without breaking the rank-index exclusion.

## Acceptance criteria

- [ ] Annual series emit `timeline_growth` (or `qualified_growth`), share
      percentages emit `segmented_total_bar`, scenario comparisons emit
      `scenario_split`, and range comparisons emit `bounded_range_comparison`;
      ordinary comparisons map cleanly without breaking Go consumers.
- [ ] Grounded `QuantityComparison` (`ratio`, `difference`, `change`) emits
      a `before_after_metric` proposal with verbatim evidence and byte spans;
      incompatible units/currencies still emit 0 proposals.
- [ ] Existing tests (`chart_acceptance_36`, `charts_numeric`,
      `chart_multilingual`, `chart_bare_cue`) continue to pass without weakening
      Top-10 rank veto assertions.
- [ ] `overlay-compiler` and Go consumer (`refactored`) accept the `ChartsPack`
      layout hints without breaking sealed `chart_placements` invariants.
- [ ] Positive + negative fixture coverage across the 11 languages.

## Non-goals

- Implementing new Chronon shader/geometry primitives (handled in `Chronon3d` /
  `ChrononMotion3D`).
- Inferring missing percentages to force a pie/donut to sum to 100%.
