# TICKET-30 — numeric facts have no unit, scale, year or comparison

- Priority: P1
- Status: OPEN
- Depends on: TICKET-29
- Files: `semantic-core/src/{numbers.rs,quantity.rs (new)}`,
  `lexicon/units.json`, `semantic-engine` (new `quantities.rs`),
  `semantic-core/src/manifest.rs` (`SceneManifest.quantities`)

## Problem

"3,2 miliardi di euro nel 2019" yields a `MONEY` `NumericFact` with an
unparsed value and nothing else: no scale applied, no currency code, no year,
no subject, no hint that "il doppio rispetto al 2018" compares two values.
`EntityFact` already has `unit`/`operator` slots but they are filled only by
facility facts. A downstream planner cannot build a counter or a bar pair
without re-reading the sentence.

## Data model (additive)

```jsonc
// SceneManifest.quantities[]
{
  "id": "q_01",
  "value": 3200000000.0,          // after scale; null if not exact
  "raw_value": 3.2, "scale": 1e9,
  "dimension": "currency",        // currency|length|mass|area|volume|time|speed|temperature|percent|count|population|energy|data
  "unit": "EUR",                  // ISO-4217 for currency, SI-style code otherwise
  "unit_surface": "euro",
  "qualifier": "exact|approx|lower_bound|upper_bound|range_start|range_end",
  "period": { "year": 2019, "fact_index": 3 },   // bound TemporalFact, optional
  "subject_entity_id": "org_02",                 // optional
  "predicate": "revenue",                        // closed vocabulary, optional
  "span": [start_byte, end_byte], "evidence": "…", "sentence_index": 2,
  "status": "asserted|negated|uncertain"
}
// SceneManifest.quantity_comparisons[]
{ "kind": "ratio|difference|change|rank", "left": "q_01", "right": "q_02",
  "dimension": "currency", "ratio": 2.0, "evidence_span": […] }
```

## Design

1. **Lexicon `lexicon/units.json`** (per language rows, generated + checked):
   unit surfaces → `{dimension, code, factor_to_base}`; currency symbols and
   words (`€`, `euro`, `eur`, `dollari`, `$`, `US$`, `złoty`, `рубл*`);
   qualifier cues (`circa`, `oltre`, `più di`, `fino a`, `almeno`, `tra … e …`,
   `about`, `over`, `up to`); predicate cues (`fatturato`, `revenue`,
   `popolazione`, `altezza`…).
2. **Candidate quantities** = every `NumericFact` + TICKET-29 word numbers.
   Attach a unit by scanning the **adjacent window** (≤ 3 tokens after, ≤ 1
   before for symbols); `di` + noun is allowed (`3 milioni di abitanti` →
   dimension `count`, unit `inhabitants`).
3. **Scale**: multiply by the scale word immediately following the number
   (`3,2 miliardi`). Value stays `null` if the number or scale is ambiguous
   (existing fail-closed rule).
4. **Period binding**: nearest `TemporalFact` in the same sentence connected by
   a preposition cue (`nel`, `in`, `en`, `im`, `w`, `в`) or a parenthesis; never
   cross a sentence boundary. Cross-sentence only via explicit anaphora
   (`quell'anno`) and only when TICKET-31 has resolved it.
5. **Subject / predicate**: reuse `EntityFact` subject resolution
   (nearest grounded entity in the clause) plus the predicate cue table.
   Both optional; absence is allowed and is never filled by default.
6. **Qualifier** from the cue table; `tra X e Y` produces two quantities with
   `range_start/range_end` sharing a `range_id`.
7. **Comparisons** (`quantity_comparisons`): only between quantities of the
   **same dimension and unit code**, and for currency the **same period or an
   explicit "at constant" statement is not required but the periods must be
   listed**; different currencies are never compared (no FX). Triggers:
   existing comparison cues (`discourse.comparison_signals`), `il doppio`,
   `triplicato`, `+12%` change patterns, rankings already in `groups.rs`.
   `ratio`/`difference` are computed arithmetically from the two exact values.
8. **Chart bridge**: `chart_rules.rs` consumes `quantities` instead of
   re-parsing numbers, so a bar pair carries unit and period. The chart output
   schema does **not** change; unit/period are read-only metadata.

## Acceptance criteria

- [ ] Gold ≥ 30 quantities per language (≥ 10 per dimension class):
      value+scale exactness 100 % of emitted, unit recall ≥ 90 %, period
      binding precision ≥ 95 %, qualifier recall ≥ 90 %.
- [ ] Negative set: ordinal numbers, phone numbers, addresses, house numbers,
      IDs, `3 volte` (frequency vs multiplier), years mistaken for counts
      (`nel 2019` is never a quantity) → 0 false positives.
- [ ] Comparisons: never across dimension/currency; 0 cross-currency pairs on
      the negative set; ratios exact to 1e-9.
- [ ] Every quantity span is verbatim and byte-exact; `value` is `null` rather
      than guessed.
- [ ] Existing chart goldens (`chart_acceptance_36`, `chart_multilingual`,
      `chart_bare_cue`) pass unchanged.
- [ ] `AnalyzerVersions` bump; Go consumer copy and goldens updated.

## Non-goals

Currency conversion/inflation adjustment, unit conversion for display,
verifying that a number is true, forecasting, extracting tables.

## Implementation steps

1. `units.json` schema + generator (it, en first), unit tests.
2. `quantity.rs` (types) and attachment logic with unit tests on windows.
3. Period/subject binding reusing `dates` and `facts.rs`.
4. `quantity_comparisons` and the chart bridge.
5. Remaining languages + gold + `score_long_gold.py` metrics.

## Prossimi passi (2026-11-19)

1. **Attende TICKET-29** (senza numeri in parole metà delle quantità non
   esiste). Nel frattempo si può chiudere `lexicon/units.json` + generatore
   (it/en), che non dipende dallo scanner.
2. **Poi `quantity.rs` + attachment window** con i test sui confini
   (≤ 3 token dopo, ≤ 1 prima per i simboli); `3 milioni di abitanti` deve
   uscire come `dimension: count`.
3. **Periodo/soggetto** riusando `dates` e `facts.rs`; mai oltre il confine di
   frase.
4. **Confronti** solo tra stessa dimensione e stessa unità; nessuna coppia
   cross-currency.

Gate: 30 quantità per lingua con valore+scala esatti sul 100 % degli emessi,
unit recall ≥ 90 %, binding del periodo ≥ 95 %, negativi (anni, telefoni,
`3 volte`) a 0 falsi positivi.
