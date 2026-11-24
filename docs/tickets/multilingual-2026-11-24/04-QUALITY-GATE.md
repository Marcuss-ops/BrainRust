# BR-MULTI-04 — The long-gold scorer does not enforce quality targets

- **Status:** CLOSED (2026-11-24)
- **Priority:** P1
- **Owner:** BrainRust evaluation harness
- **Measured:** 2026-11-24

## Problem

`score_long_gold.py` exits **0** when the corpus is release-sized, even when
the measured quality is far below the proposed targets. The 2026-11-24 run
exits 0 with:

- entity recall 0.758 (target 0.95);
- typed object recall 0.629 (target 0.90);
- surface object recall 0.924 (target 0.90, passes);
- grounding errors 0;
- replay identical.

A green exit code therefore means "the harness ran", not "the quality target
was met". The short-gold scorer already fails on its thresholds; the long-gold
scorer does not.

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

## Acceptance criteria

- `score_long_gold.py --check-thresholds` exits 1 on the 2026-11-24 report and
  prints the entity and typed-object failures.
- Without the flag, the same run still exits 0.
- A synthetic unit test covers pass, each failure and a missing measurement.
- The gate does not claim human certification; it measures extraction targets
  only.

## Verification (2026-11-24)

- `python3 -m py_compile` on the scorer and its test -> RC 0.
- `python3 score_long_gold_test.py` -> **3 tests, OK**.
- `score_long_gold.py --check-thresholds` -> **exit 1**, with
  `GATE-FAIL entity recall: 0.758 < 0.950` and
  `GATE-FAIL typed object recall: 0.629 < 0.900`. The report's
  `quality_gate.pass` is `false`.
- The same command **without** the flag -> **exit 0**, printing
  `quality gate (not enforced): FAIL`; the default contract is unchanged.

## Non-goals

- Making the gate mandatory in CI before the engine meets the targets.
- Lowering thresholds to make the current run pass.
- Adding precision thresholds before the long-gold aggregate exposes
  precision; the short gold already scores precision.
