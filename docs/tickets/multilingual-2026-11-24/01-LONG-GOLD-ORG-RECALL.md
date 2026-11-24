# BR-MULTI-01 — Long-form multilingual ORG recall

- **Status:** CLOSED (2026-11-24)
- **Priority:** P1
- **Owner:** BrainRust semantic extraction
- **Measured:** 2026-11-24, release worker, 10 languages × 2 long scripts

## Final result

The long-form editorial gold scores **122/124 named entities (98.4%)**, above
the **95%** target. The short gold also passes at recall 1.000 and precision
0.991. Two Russian entities remain in `ru_stadion_long_01`; they are a
documented residual, not a gate failure.

| Language | Entity recall | Surface object recall |
| --- | --- | --- |
| de | 12/12 = 1.000 | 9/10 = 0.900 |
| en | 12/12 = 1.000 | 10/10 = 1.000 |
| es | 12/12 = 1.000 | 10/10 = 1.000 |
| fr | 12/12 = 1.000 | 10/10 = 1.000 |
| id | 12/12 = 1.000 | 9/10 = 0.900 |
| it | 16/16 = 1.000 | 14/15 = 0.933 |
| nl | 12/12 = 1.000 | 9/10 = 0.900 |
| pl | 12/12 = 1.000 | 9/10 = 0.900 |
| pt | 12/12 = 1.000 | 9/10 = 0.900 |
| ru | 10/12 = 0.833 | 9/10 = 0.900 |
| **overall** | **122/124 = 0.984** | **98/105 = 0.933** |

## Residual

`ru_stadion_long_01` still misses:

- `Стадион Северный` (FACILITY): the extractor emits the prepositional
  `Стадионе Северном` and a separate `Северный` LOCATION;
- `Вода Север` (ORG): the extractor emits `Компания Вода` as ORG and
  `Вода Север` as PERSON.

The scene's contextual query is consequently 0/1. Overall contextual-query
recall is 19/20. The gate target is met and the residual is reported rather
than hidden by a scorer change.

## What was fixed

- Added the generic office/studio markers and the lowercase-marker path.
- Added `generic_company_nouns` so `empresa`, `société`, `perusahaan`,
  `компания`, `firma` and related heads introduce a name without becoming
  part of it.
- Added the subject-verb path (`nsubj` plus a language-specific
  estimate/expect/claim verb), including the real UDPipe lemmas `raam`,
  `menperkirakan` and `оценивать`.
- Allowed a capitalized identity after an organization marker even when the
  tagger calls it a stopword or ADP (`бюро Заря`, `biuro Wiślan`).
- Promoted a PERSON run to FACILITY when it contains a curated facility head
  and a capitalized companion (`Central Stadium`, `Stadion Północny`,
  `Стадион Северный`).
- The gold harness requests `entity_safety_cap=64` so its 124-entity slice is
  not truncated by the extractor's default top-N. Thresholds are unchanged.

The fix is data/tests only; no language-specific branch was added to the
planner.

## Acceptance criteria

- Long-gold entity recall **≥ 0.95** overall: **met (0.984)**.
- Short-gold precision **≥ 0.98**: **met (0.991)**; no noise leaks.
- Grounding errors **0**; replay byte-identical: **met**.
- Per-language recall reported: **met**; Russian is 10/12 and is explicitly
  recorded.
- No language-specific planner branch: **met**.

## Non-goals

- Certifying the 19 draft scripts without human review.
- Claiming a 17-language result from the 10-language corpus.
- Adding the four profile-only languages (`ko`, `ar`, `hi`, `th`).
