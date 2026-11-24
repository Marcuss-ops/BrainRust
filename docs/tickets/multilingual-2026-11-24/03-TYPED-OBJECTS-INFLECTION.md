# BR-MULTI-03 — Typed visual objects lose inflection and multiword types

- **Status:** OPEN
- **Priority:** P1
- **Owner:** BrainRust visual extraction
- **Measured:** 2026-11-24, long-form editorial gold

## Problem

Surface detection is close to target, but type assignment is not:

- surface-only object recall: **97/105 = 0.924**;
- typed object recall: **66/105 = 0.629**;
- the gap is **31 objects** that are found by surface but not matched with the
  gold type.

| Language | Typed object recall | Surface object recall |
| --- | --- | --- |
| de | 8/10 = 0.800 | 10/10 = 1.000 |
| en | 7/10 = 0.700 | 10/10 = 1.000 |
| es | 8/10 = 0.800 | 9/10 = 0.900 |
| fr | 7/10 = 0.700 | 10/10 = 1.000 |
| id | 8/10 = 0.800 | 9/10 = 0.900 |
| it | 9/15 = 0.600 | 14/15 = 0.933 |
| nl | 9/10 = 0.900 | 9/10 = 0.900 |
| pl | 2/10 = 0.200 | 9/10 = 0.900 |
| pt | 6/10 = 0.600 | 8/10 = 0.800 |
| ru | 2/10 = 0.200 | 9/10 = 0.900 |
| **overall** | **66/105 = 0.629** | **97/105 = 0.924** |

## Observed misses

- Polish/Russian gold uses dictionary forms, while the text uses inflected
  forms: `siatka` / `siatkę`, `mapa` / `mapę`, `kamera` / `kamerę`,
  `kaski` / `шлемы`, `kable` / `кабели`, `карту`, `камеру`.
- Italian multiword objects are found as surfaces but not typed as `OBJECT`:
  `sensori di umidità`, `pompa mobile`, `pannelli fotovoltaici`,
  `quadro elettrico`, `canaletta`, `cartello temporaneo`.
- Some gold `OBJECT` entries are `VISUAL_CONCEPT` in the engine
  (`raport`, `laporan`, `отчёт`, `stellingen`), which may be a contract or
  annotation mismatch rather than an extraction miss.

The scorer already consults identity dictionary forms for some categories
(`identity_candidates`), but the object path does not preserve enough lemma
information for the inflected Slavic surfaces.

## Proposed implementation

1. Inspect the object identity records for the Polish/Russian misses; add
   canonical object lemmas only when the surface is grounded.
2. Separate `OBJECT` from `VISUAL_CONCEPT` in the gold adapter, and document
   which type the contract expects for reports/documents.
3. Add per-language tests for one inflected object and one multiword Italian
   object; assert the source span is preserved.
4. Re-run the long gold; target typed recall ≥ 0.90 without lowering surface
   recall or grounding.

## Acceptance criteria

- Typed object recall **≥ 0.90** overall.
- Surface object recall **≥ 0.95**; grounding errors stay **0**.
- Per-language report shows Polish and Russian above 0.80.
- No new object vocabulary that types common nouns as visual objects.
- The scorer is not weakened to count a `VISUAL_CONCEPT` as `OBJECT`.

## Non-goals

- Adding every noun in the corpus to `object_vocabulary.json`.
- Replacing the object contract with an LLM.
- Changing the gold annotations without human review.
