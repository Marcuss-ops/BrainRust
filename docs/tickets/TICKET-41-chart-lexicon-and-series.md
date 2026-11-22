# TICKET-41 — Chart lexicons in `lexicon/`, N-point cross-sentence series & chart metadata

- Priority: P1
- Status: DONE (2026-11-22)
- Depends on: TICKET-30
- Files: `semantic-engine/src/chart_rules.rs`,
  `semantic-engine/src/groups/charts.rs`,
  `semantic-core/src/manifest.rs`,
  `semantic-engine/src/proposals.rs`,
  `semantic-engine/src/consolidate/producers.rs`,
  `lexicon/` (new chart cue/unit pack or extension of `units` + `build_language_packs.py`),
  `contract/` & Go consumer (`refactored/internal/platform/media/rustexec/`)

## Problem

1. **Hardcoded multilingual rules in Rust (`it`/`en`-skewed):**
   `semantic-engine/src/chart_rules.rs` and `semantic-engine/src/groups/charts.rs`
   embed language tables directly in Rust instead of `lexicon/`, violating the
   workspace invariant (*"Data, not code, per language"*):
   - `HARD_VETOES` (`chart_rules.rs:15-77`)
   - `qualified_numeric_span` approximators, units and rate cues (`chart_rules.rs:113-312`)
   - `has_series_context` cues (`chart_rules.rs:317-396`, e.g. `esportazion`, `fatturato`, `richieste`/`approvat`/`respint`, `stesso mercato`, `compreso tra`)
   - `chart_unit` currencies, length units and `known_units` (`chart_rules.rs:425-590`)
   - `same_metric_across_semicolon` stems (`chart_rules.rs:612-704`)
   - `same_series_metric` (`charts.rs:643-664`, only 13 Italian/English stems)
   - `chart_subject_label` stopwords and trailing verb cutoffs (`charts.rs:512-590`)
   Non-Italian/non-English scripts miss chart extraction when predicates or
   units are outside those narrow hardcoded lists.

2. **Cross-sentence series capped at 2-point pairs:**
   `cross_sentence_series` (`charts.rs:202-378`) compares pairs `(left, right)`
   within `right.sentence_index - left.sentence_index <= 3` and emits a
   2-item chart per pair. A 3–6 sentence annual progression (e.g. 2021, 2022,
   2023, 2024) either fragments into overlapping pairs or drops intermediate
   points, and relies on `same_series_metric` rather than `Quantity` predicates
   and units from TICKET-30.

3. **No chart-level metadata or `ChartQualifier` for downstream renderers (`ChrononTemplate` mismatch):**
   `ChartItem` (`semantic-core/src/manifest.rs:274-284`) only carries `label`,
   `text`, `kind`, `value`, `start_byte`, `end_byte`. Neither `VisualProposal`
   nor `ProductionCandidate` exposes structured `metric`, `unit`, `period`, or
   `series_type` (`time` vs `category`), and `ChartItem` does not expose the
   structured `qualifier` (`Exact`, `Approximate`, `GreaterThan`, `Forecast`,
   `Hypothetical`, `Range`, `Missing`) or `period`/`lower`/`upper` already
   expected by `ChrononTemplate/include/chronontemplate/charts/ChartsPack.hpp`.
   Downstream planners/renderers must therefore re-parse raw evidence text for
   titles, axis units and qualifiers.

## Scope & Design

1. **Move chart vocabularies into `lexicon/`:**
   - Extract `HARD_VETOES`, rate cues, series context cues/predicates, and
     subject-label stopword/verb cutoffs into versioned lexicon JSON checked by
     `lexicon/tools/build_language_packs.py --check` and fingerprinted into
     `semantic_core::content_fingerprint`.
   - Reuse `lexicon/units/*.json` (from TICKET-30) inside `chart_unit` and
     `qualified_numeric_span` instead of maintaining a duplicate `known_units`
     array in Rust.

2. **N-point cross-sentence series (`2..=6` points):**
   - Group consecutive same-unit, same-metric `ChartPoint`/`Quantity` items
     within a bounded window (e.g. gap ≤ 3 sentences between adjacent points,
     up to 6 items total) into one ordered `VisualCandidate` instead of emitting
     isolated pairs.
   - Match series metric via grounded `Quantity.predicate` + `Quantity.unit`
     first, keeping `same_series_metric` as a compatibility fallback via the
     lexicon table.

3. **Additive `ChartMetadata` & `ChartQualifier` on proposals/candidates:**
   - Add `#[serde(default, skip_serializing_if = "Option::is_none")] pub chart_meta: Option<ChartMetadata>`
     on `VisualProposal` and `ProductionCandidate` (`metric`, `unit`,
     `dimension`, `period`, `series_type`: `"time" | "category"`).
   - Add additive optional `qualifier` (`exact | approximate | greater_than | forecast | hypothetical | range | missing`),
     `period`, `lower`, `upper` on `ChartItem` (bridged from `Quantity.qualifier`
     / `range_start`+`range_end` / `TemporalFact`) matching `ChartsPack.hpp`,
     while keeping all existing required `ChartItem` fields unchanged.

## Acceptance criteria

- [x] All chart-specific word lists (`HARD_VETOES`, rate cues, series cues,
      subject label stopwords/verbs, series metric stems) live in `lexicon/` and
      pass `build_language_packs.py --check`.
- [x] A 3–5 sentence annual series produces one N-point chart candidate
      (`2..=6` items) in all 11 supported languages (`de`, `en`, `es`, `fr`,
      `id`, `it`, `nl`, `pl`, `pt`, `ru`, `tr`), with byte-exact verbatim item
      spans and scene-spanning evidence.
- [x] `chart_meta` is populated whenever `unit` / `dimension` / `predicate` /
      `series_type` are grounded; absent fields stay `None` (never guessed).
- [x] Existing chart regression suites (`chart_acceptance_36`,
      `chart_bare_cue`, `chart_multilingual`, `charts_numeric`) pass with 0
      regressions and 0 false positives on existing negative sets.
- [x] `AnalyzerVersions` bumped; Go consumer schema/golden updated in sync.

## Non-goals

- Currency conversion or inflation adjustment across periods.
- Changing Chronon rendering templates or Go admission budgets.

## Verification — 2026-11-22

- `lexicon/chart_cues.json` owns every chart cue table and `build_language_packs.py --check` passes.
- `semantic-engine/tests/chart_series_metadata.rs` proves one N-point candidate (4 points, 6-point cap) in all 11 languages with byte-exact item spans and scene-spanning evidence.
- `ChartMetadata` is populated from grounded quantities; `ChartItem` now carries additive `qualifier`, `period`, `lower`, `upper` and `range_id`.
- Chart regressions (`chart_acceptance_36`, `chart_bare_cue`, `chart_multilingual`, `charts_numeric`) pass; `AnalyzerVersions.quantities` is `typed_quantities.v3.chart_meta_series`.
