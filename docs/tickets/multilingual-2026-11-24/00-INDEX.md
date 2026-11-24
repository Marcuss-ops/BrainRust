# Multilingual editorial quality — 2026-11-24

First real run of the long-form editorial gold after the multilingual corpus
review. This wave separates measured engine gaps from corpus/provenance gaps;
it does not claim that the 17-language target is met.

## Findings

| ID | Priority | Status | Finding |
| --- | --- | --- | --- |
| [BR-MULTI-01](01-LONG-GOLD-ORG-RECALL.md) | P1 | OPEN | Long-form entity recall is 75.8%, driven by 29/30 ORG misses |
| [BR-MULTI-02](02-CORPUS-COVERAGE-AND-PROVENANCE.md) | P0 | BLOCKED | The linked 17-language/51-script corpus is incomplete and cannot be scored |
| [BR-MULTI-03](03-TYPED-OBJECTS-INFLECTION.md) | P1 | OPEN | Typed visual-object recall is 62.9%; surface recall is 92.4% |
| [BR-MULTI-04](04-QUALITY-GATE.md) | P1 | CLOSED | The long-gold scorer exits 0 with 75.8% entity recall |

**Counts:** 1 CLOSED · 2 OPEN · 1 BLOCKED.

## Evidence

- Worker: `target/release/semantic-engine` (built 2026-11-24 08:00 UTC).
- Models: `VISUALNER_MODEL_DIR=$HOME/.local/share/visualner/models`;
  health reports 13 installed UDPipe languages, 0 missing, 0 failed.
- GLiNER: not installed (`ner.mode=auto`, `model_installed=false`).
- ICU: `RUST_ICU_MAJOR_VERSION_NUMBER=70`.
- Short gold: `score_gold.py` -> recall **1.000** (98/98), precision
  **0.991** (112/113), noise leaks **0**, exit 0.
- Long gold: `score_long_gold.py` -> entity recall **0.758** (94/124),
  typed object recall **0.629** (66/105), surface object recall **0.924**
  (97/105), grounding errors **0**, replay identical, exit 0.
- Reports: `/tmp/brainrust_short_gold_20261124.json`,
  `/tmp/brainrust_long_gold_20261124.json`.
- The previous committed report
  `semantic-engine/fixtures/gold_multilingual/reports/long_gold_report_2026-11-24.json`
  carries the same entity and typed-object numbers, so the result is
  reproducible across runs.

## Reproduction

```sh
cd BrainRust
export VISUALNER_MODEL_DIR="$HOME/.local/share/visualner/models"
export RUST_ICU_MAJOR_VERSION_NUMBER=70

python3 semantic-engine/fixtures/gold_multilingual/score_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/brainrust_short_gold_20261124.json

python3 semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/brainrust_long_gold_20261124.json

# Expected exit 1 until BR-MULTI-01 and BR-MULTI-03 close.
python3 semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/brainrust_long_gold_gate_20261124.json \
  --check-thresholds

python3 semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold_test.py
```

The gate reports `quality_gate.pass=false` and exits 1 for entity recall and
typed-object recall; grounding and replay pass. Without `--check-thresholds`
the same run exits 0, preserving the existing harness contract.

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

## Non-goals

- No new visual recipes (`before_after`, `causal_diagram`, `process_steps`,
  `visual_narrative_state.v1`) in this wave.
- No 100-language expansion.
- No weakening of the named-entity or object contracts to raise the score.
