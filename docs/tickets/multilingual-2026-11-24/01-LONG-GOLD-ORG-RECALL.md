# BR-MULTI-01 — Long-form multilingual ORG recall

- **Status:** OPEN
- **Priority:** P1
- **Owner:** BrainRust semantic extraction
- **Measured:** 2026-11-24, release worker, 10 languages × 2 long scripts

## Problem

The long-form editorial gold scores **94/124 named entities (75.8%)** against
the proposed **95%** target. The short gold passes at 100%, so this is not a
general multilingual regression: it appears when the text uses generic
organization names and contextual company mentions.

| Language | Entity recall | Surface object recall |
| --- | --- | --- |
| de | 10/12 = 0.833 | 10/10 = 1.000 |
| en | 9/12 = 0.750 | 10/10 = 1.000 |
| es | 8/12 = 0.667 | 9/10 = 0.900 |
| fr | 8/12 = 0.667 | 10/10 = 1.000 |
| id | 8/12 = 0.667 | 9/10 = 0.900 |
| it | 16/16 = 1.000 | 14/15 = 0.933 |
| nl | 8/12 = 0.667 | 9/10 = 0.900 |
| pl | 11/12 = 0.917 | 9/10 = 0.900 |
| pt | 8/12 = 0.667 | 8/10 = 0.800 |
| ru | 8/12 = 0.667 | 9/10 = 0.900 |
| **overall** | **94/124 = 0.758** | **97/105 = 0.924** |

## Miss breakdown

The 30 misses are:

- **29 ORG**
  - 16 generic office/studio heads: `Büro Steinbach`, `Büro Auerbach`,
    `Studio Halden`, `Studio Mercer`, `estudio Bermejo`, `estudio Casal`,
    `bureau Marchal`, `bureau Verdier`, `kantor Nugraha`, `kantor Mahesa`,
    `bureau Holt`, `bureau Smit`, `gabinete Seixas`, `gabinete Lima`,
    `бюро Волга`, `бюро Заря`.
  - 13 company names with no legal suffix or curated head: `Agua Norte`,
    `Energía Puerto`, `Eau Nord`, `Énergie Port`, `Air Utara`,
    `Energi Pelabuhan`, `Water Noord`, `Energie Haven`, `Energia Port`,
    `Água Norte`, `Energia Porto`, `Вода Север`, `Энергия Порт`.
- **1 PERSON**: `Daniel Frost` (`en_stadium_long_01`).

The Italian scripts score 16/16; the gap is in the other nine languages, not
in the Italian pipeline.

## Root cause

1. `lexicon/entity_gazetteer.json -> organization_markers` has 13–14 generic
   heads per language (`stiftung`, `labor`, `council`, `fondation`, etc.) but
   no `büro`, `studio`, `estudio`, `bureau`, `kantor`, `gabinete` or `бюро`.
2. `organization_name_runs` in `visualner/src/extractor/names.rs` requires the
   marker token to start uppercase. That can match `Büro Steinbach` and
   `Studio Halden`, but not lowercase `estudio Bermejo`, `bureau Marchal`,
   `kantor Nugraha`, `gabinete Seixas` or `бюро Волга`.
3. The 13 unmarked company names are two capitalized tokens with no legal
   suffix. Typing them as ORG needs contextual evidence (subject of
   `expected`/`claimed`/`reported`, comparison with another company), not a
   generic head marker.
4. `organization_context_verbs` exists only for Italian (10 verbs). There is no
   per-language contextual-organization pass for the other nine languages.

A data-only marker addition is therefore insufficient: it could fix at most
the four capitalized German/English office names, not the lowercase heads or
the unmarked company names.

## Proposed implementation

1. Add a per-language organization-head table for the generic office/studio
   words, with a rule that allows a curated lowercase head only when followed
   by a capitalized name and the sentence has no competing person/place cue.
2. Add negative tests for generic common-noun uses (`the studio was empty`,
   `el estudio duró dos años`) so the rule does not type every `studio` as ORG.
3. Extend contextual ORG typing to subject/verb patterns per language, reusing
   the existing relation/verb evidence rather than a new model.
4. Re-run the short gold and the long gold. Target: entity recall ≥ 0.95
   overall, no regression in precision/noise, grounding still 0.
5. Ask a human annotator to review the 19 draft scripts before treating the
   score as certification.

## Acceptance criteria

- Long-gold entity recall **≥ 0.95** overall.
- Per-language recall reported; a per-language gate is meaningful only after
  each language has enough annotated entities (the current slice has 12 per
  language, so 0.95 requires 12/12).
- No new noise leaks; short-gold precision stays **≥ 0.98**.
- Grounding errors stay **0**; replay stays byte-identical.
- The fix is data/tests only; no language-specific branch in the planner.

## Non-goals

- Adding the four profile-only languages (`ko`, `ar`, `hi`, `th`).
- Certifying the draft gold without human review.
- Changing the named-entity type contract or hiding misses in the scorer.
