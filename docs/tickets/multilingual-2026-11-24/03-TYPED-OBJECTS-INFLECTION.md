# BR-MULTI-03 — Typed visual objects lose inflection and multiword types

- **Status:** CLOSED (enforced gate target; 0.95 surface aspiration remains)
- **Priority:** P1
- **Owner:** BrainRust visual extraction
- **Measured:** 2026-11-24, long-form editorial gold

## Final result

The enforced object gate passes:

- typed object recall: **95/105 = 0.905** (target 0.90);
- surface object recall: **98/105 = 0.933** (gate target 0.90);
- grounding errors: **0**.

The ticket's stricter **0.95** surface-object aspiration is **not** met. That
is a documented residual, not a reason to lower or relabel the gate.

| Language | Typed object recall | Surface object recall |
| --- | --- | --- |
| de | 8/10 = 0.800 | 9/10 = 0.900 |
| en | 10/10 = 1.000 | 10/10 = 1.000 |
| es | 10/10 = 1.000 | 10/10 = 1.000 |
| fr | 9/10 = 0.900 | 10/10 = 1.000 |
| id | 9/10 = 0.900 | 9/10 = 0.900 |
| it | 13/15 = 0.867 | 14/15 = 0.933 |
| nl | 9/10 = 0.900 | 9/10 = 0.900 |
| pl | 9/10 = 0.900 | 9/10 = 0.900 |
| pt | 9/10 = 0.900 | 9/10 = 0.900 |
| ru | 9/10 = 0.900 | 9/10 = 0.900 |
| **overall** | **95/105 = 0.905** | **98/105 = 0.933** |

## What was fixed

- Lemma-aware object lookup, so inflected surfaces (`kable`, `кабели`,
  `câbles`, `cabos`) reach the canonical object term.
- Multiword object head typing for Italian technical phrases.
- Additional object terms for helmets, cables, scaffolding, ladders,
  cameras, reports and related terms.
- Generic visual nouns (`raport`, `laporan`, `отчёт`, `stellingen`) keep their
  VISUAL_CONCEPT type; the scorer is not weakened to count them as OBJECT.
- The gold harness requests `entity_safety_cap=64` so the object/entity slice
  is not truncated by the default top-N; the object thresholds are unchanged.

## Residual

- Surface recall is 98/105 (0.933), below the ticket's 0.95 aspiration.
- Italian typed recall is 13/15 (0.867); the overall typed gate still passes.
- The two Russian named-entity misses in `ru_stadion_long_01` are tracked in
  BR-MULTI-01 and are not hidden in this object score.

## Acceptance criteria

- Typed object recall **≥ 0.90** overall: **met (0.905)**.
- Surface object recall **≥ 0.90** (enforced gate): **met (0.933)**.
- Surface object recall **≥ 0.95** (ticket aspiration): **not met; recorded**.
- Grounding errors **0**: **met**.
- Per-language Polish and Russian typed recall above 0.80: **met (0.900 each)**.
- No common nouns added merely to raise the score: **met**.

## Non-goals

- Adding every noun in the corpus to `object_vocabulary.json`.
- Replacing the object contract with an LLM.
- Changing the gold annotations without human review.
