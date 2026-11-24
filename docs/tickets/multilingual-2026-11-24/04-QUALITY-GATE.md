# BR-MULTI-04 — The long-gold scorer does not enforce quality targets

- **Status:** CLOSED (2026-11-24)
- **Priority:** P1
- **Owner:** BrainRust evaluation harness
- **Measured:** 2026-11-24

## Problem

`score_long_gold.py` exits **0** when the corpus is release-sized, even when
the measured quality is far below the proposed targets. The initial
2026-11-24 run exited 0 with entity recall 0.758, typed object recall 0.629,
surface object recall 0.924, grounding 0 and replay identical. A green exit
code therefore meant "the harness ran", not "the quality target was met".

## Fix

Add an opt-in `--check-thresholds` mode:

- entity recall ≥ 0.95;
- surface object recall ≥ 0.90;
- typed object recall ≥ 0.90;
- grounding errors == 0;
- replay byte-identical.

The default remains unchanged so existing consumers do not break. The report
gets an additive `quality_gate` block with `enforced`, `pass`, `thresholds`
and `failures`. With the flag, a failing target exits 1; a non-release-sized
corpus still exits 2.

The long-gold request now sets `entity_safety_cap=64`. The default extractor
cap is a safety bound, not a quality threshold; 64 lets the 124-entity slice
reach the scorer without top-N truncation. The gate thresholds are unchanged,
and the short gold remains on its own scorer.

## Final verification (2026-11-24)

- `python3 -m py_compile` on the scorer and its test -> RC 0.
- `python3 score_long_gold_test.py` -> **3 tests, OK**.
- `score_long_gold.py --check-thresholds` -> **exit 0**;
  `quality_gate.pass=true`, `failures=[]`.
- Final measurements: entity **122/124 = 0.984**, typed objects
  **95/105 = 0.905**, surface objects **98/105 = 0.933**, grounding **0**,
  replay identical, contextual queries **19/20**.
- Short gold -> **PASS**, recall **1.000** (98/98), precision **0.991**
  (112/113), leaks **0**.
- The run used ICU **72**, the canonical `ICU_MAJOR` in `.cargo/config.toml`.
- Report: `/tmp/brainrust_long_gold_final_20261124.json`; the committed
  `reports/long_gold_report_2026-11-24.json` is regenerated from this run.

The historical failing run remains part of the ticket history; the current
gate passes because the extraction gaps were fixed, not because a threshold
was lowered.

## Acceptance criteria

- `score_long_gold.py --check-thresholds` exits 0 on the final report and
  reports `quality_gate.pass=true`: **met**.
- Without the flag, the same run still exits 0: **met**.
- A synthetic unit test covers pass, each failure and a missing measurement:
  **met (3 tests)**.
- The gate does not claim human certification; it measures extraction targets
  only: **met**.

## Non-goals

- Making the gate mandatory in CI before the engine meets the targets.
- Lowering thresholds to make the current run pass.
- Adding precision thresholds before the long-gold aggregate exposes
  precision; the short gold already scores precision.
