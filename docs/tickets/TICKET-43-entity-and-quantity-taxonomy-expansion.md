# TICKET-43 — Expand visual entity types and `Quantity` dimensions

- Priority: P2
- Status: DONE (2026-11-22)
- Depends on: TICKET-30, TICKET-35
- Files: `visualner/src/`, `semantic-core/src/types.rs`,
  `semantic-core/src/units.rs`, `lexicon/units/`, `lexicon/type_cues.json`,
  `lexicon/object_vocabulary.json`, `semantic-engine/src/quantities.rs`,
  `semantic-engine/src/legal.rs`

## Problem

1. **Coarse entity types for documentary & stock-retrieval queries:**
   `visualner` currently projects a small closed set of non-numeric entity types
   (`PERSON`, `ORG`, `LOCATION`, `FACILITY`, `OBJECT`, `BRAND`, `PRODUCT`,
   `WORK`, `EVENT`, `VISUAL_CONCEPT`). Several visually distinct categories are
   either collapsed into `OBJECT`/`PRODUCT` or omitted as typed entities:
   - Stock-friendly visual subjects: `ANIMAL` (species), `FOOD` (dishes/ingredients),
     `VEHICLE` (aircraft, ship, car/train models), `CELESTIAL` (planets/stars),
     `ARTWORK` (paintings/sculptures distinct from `WORK` books/films).
   - Documentary/editorial subjects: `LAW` (statutes, articles, treaties),
     `COURT_CASE` (trial names, complementing `semantic-engine/src/legal.rs`),
     `NORP` (nationalities, political/religious groups), `DISEASE` (medical
     conditions), `TECHNOLOGY` (software/platforms/protocols).

2. **Missing quantitative dimensions in `lexicon/units/` (`TICKET-30`):**
   `lexicon/units/*.json` currently defines 13 dimensions (`currency`,
   `percent`, `length`, `mass`, `area`, `volume`, `time`, `speed`,
   `temperature`, `count`, `population`, `energy`, `data`). Three common
   documentary numeric forms lack dedicated typed dimensions:
   - `ratio`: `"1 su 3"`, `"1 in 4"`, `"3 volte"`, `"3x"` (distinguishing
     multipliers/ratios from frequency idioms already excluded by TICKET-30).
   - `growth_rate`: `"+12% YoY"`, `"-4,5% annuo"` (signed percentage change tied
     to a period).
   - `rank`: `"1° posto"`, `"3rd place"` (explicit ordinal rank distinct from
     cardinal `count`).

## Scope & Design

1. **Additive entity types / subtypes:**
   - Extend `lexicon/type_cues.json` and `lexicon/object_vocabulary.json` (plus
     optional GLiNER label mapping when `BRAINRUST_NER_MODEL=gliner25` is active)
     to classify `VEHICLE`, `ANIMAL`, `FOOD`, `LAW`, `NORP`, `DISEASE`,
     `TECHNOLOGY` cleanly while preserving existing `OBJECT`/`PRODUCT` fallback
     compatibility for older consumers.
   - Keep `entity_search` query generation verbatim and source-grounded.

2. **Extend `lexicon/units/*.json` & `semantic-engine/src/quantities.rs`:**
   - Add `ratio`, `growth_rate`, and `rank` dimensions across all 11 supported
     languages, regenerated via `build_units.py` / `build_language_packs.py`.
   - Maintain TICKET-30 negative guards (e.g. `"3 volte"` as frequency vs
     multiplier; addresses/IDs/years never become quantities).

## Acceptance criteria

- [x] `lexicon/units/*.json` checks pass and `content_fingerprint` updates.
- [x] `ratio`, `growth_rate`, and `rank` have ≥ 10 positive + ≥ 10 negative
      cases per dimension across the 11 languages, with 0 false positives on
      existing TICKET-30 negatives.
- [x] New entity types have deterministic unit + integration tests (both
      model-free and UDPipe/GLiNER paths where enabled) without regressing the
      existing short/long gold entity recall or precision.
- [x] `AnalyzerVersions` bumped; Go `contract/` & `rustexec` enum decoders
      accept the new types cleanly.

## Non-goals

- Rewriting historical `entity_index_v1.json` IDs.
- Inferring medical/legal truth from extracted `DISEASE` or `LAW` entities.

## Verification — 2026-11-22

- `lexicon/entity_type_terms.json` covers the 10 additive types in all 11 languages; `build_language_packs.py --check` validates it and `content_fingerprint` includes it.
- `semantic-core` unit tests prove exact/head/tail matching in every language; `visualner` integration tests ground 20 cases across languages without changing older OBJECT terms.
- `ratio`, `growth_rate` and `rank` are in `lexicon/units/*.json`; `quantities::tests` covers 11 positives and language-scoped negatives per dimension.
- `AnalyzerVersions.visualner` is `visualner.v2.additive_types`; Go `entitytype` folds the new spellings into existing closed families and its tests pass.
