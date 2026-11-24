# Multilingual editorial quality — 2026-11-24

Final run of the long-form editorial gold after the multilingual corpus review.
The 10-language gate passes; the linked 17-language corpus remains unavailable,
so no 17-language claim is made.

## Findings

| ID | Priority | Status | Finding |
| --- | --- | --- | --- |
| [BR-MULTI-01](01-LONG-GOLD-ORG-RECALL.md) | P1 | CLOSED | Long-form entity recall is 122/124 = 98.4%; two Russian entities remain |
| [BR-MULTI-02](02-CORPUS-COVERAGE-AND-PROVENANCE.md) | P0 | BLOCKED | The linked 17-language/51-script corpus is incomplete and cannot be scored |
| [BR-MULTI-03](03-TYPED-OBJECTS-INFLECTION.md) | P1 | CLOSED (gate) | Typed object recall is 95/105 = 90.5%; surface recall is 98/105 = 93.3% |
| [BR-MULTI-04](04-QUALITY-GATE.md) | P1 | CLOSED | The long-gold gate now exits 0 with `quality_gate.pass=true` |

**Counts:** 3 CLOSED · 1 BLOCKED. BR-MULTI-03 is closed for the enforced
gate target; its stricter 0.95 surface-object aspiration is recorded as a
residual, not silently reclassified as met.

## Evidence

- Worker: `target/release/semantic-engine` (release, 2026-11-24).
- Models: `VISUALNER_MODEL_DIR=$HOME/.local/share/visualner/models`;
  health reports 13 installed UDPipe languages, 0 missing, 0 failed.
- GLiNER: not installed (`ner.mode=auto`, `model_installed=false`).
- ICU: **72**, the canonical `ICU_MAJOR` from `.cargo/config.toml`.
- Short gold: `score_gold.py` -> recall **1.000** (98/98), precision
  **0.991** (112/113), noise leaks **0**, exit 0.
- Long gold: `score_long_gold.py` -> entity recall **0.984** (122/124),
  typed object recall **0.905** (95/105), surface object recall **0.933**
  (98/105), contextual queries **19/20**, grounding errors **0**, replay
  identical. The enforced gate exits **0**.
- Reports: `/tmp/brainrust_short_gold_final_20261124.json`,
  `/tmp/brainrust_long_gold_final_20261124.json`.
- The committed long report
  `semantic-engine/fixtures/gold_multilingual/reports/long_gold_report_2026-11-24.json`
  is regenerated from the final run.
- Residual: `ru_stadion_long_01` misses `Стадион Северный` and
  `Вода Север`; the scene's contextual query is also 0/1. This does not
  block the overall gate and is recorded in BR-MULTI-01.

## Reproduction

```sh
cd BrainRust
export VISUALNER_MODEL_DIR="$HOME/.local/share/visualner/models"
export RUST_ICU_MAJOR_VERSION_NUMBER=72

python3 semantic-engine/fixtures/gold_multilingual/score_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/brainrust_short_gold_final_20261124.json

python3 semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/brainrust_long_gold_final_20261124.json \
  --check-thresholds

python3 semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold_test.py
```

The gate report is `quality_gate.pass=true` with no failures. Without
`--check-thresholds` the same run still exits 0, preserving the harness
contract.

## What is not claimed

- The short gold passes, but it is a 20-script named-entity/noise suite, not
  an editorial quality certification.
- The long gold's 19 draft scripts are `assistant_draft_pending_human_review`;
  certification remains `blocked_pending_human_review`.
- The Drive folder linked on 2026-11-24 contains 10 Italian `.txt` files and a
  README, not the promised `gold.json`, `requests.ndjson` or scorer. See
  BR-MULTI-02.
- The repository has no 17-language long gold. Its long gold covers 10
  languages; `language_profiles.json` declares 13 treebanks and four
  profile-only languages (`ko`, `ar`, `hi`, `th`).
- The 0.95 surface-object aspiration in BR-MULTI-03 is not met; the enforced
  gate target is 0.90 and it passes.

## Non-goals

- No new visual recipes (`before_after`, `causal_diagram`, `process_steps`,
  `visual_narrative_state.v1`) in this wave.
- No 100-language expansion.
- No weakening of the named-entity or object contracts to raise the score.
