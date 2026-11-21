# TICKET-30 — numeric facts have no unit, scale, year or comparison

- Priority: P1
- Status: PARTIAL (verified 2026-11-20)
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

## Avanzamento implementazione (2026-11-19)

- `lexicon/units.json` ora definisce unità, fattori, qualificatori e cue di
  predicato per le 11 lingue; `build_units.py` genera i pack inclusi da Rust e
  la CI ne verifica la corrispondenza.
- `Quantity`/`QuantityComparison` sono proiettati in `SceneManifest`; il
  worker collega gli span numerici grounded a unità adiacenti, scala, date
  nello stesso periodo frasale e fatti entità con soggetto/predicato/status.
  Gli intervalli condividono un `range_id`; i confronti esatti richiedono
  dimensione e codice unità uguali e, per valuta, lo stesso anno. Il chart
  bridge usa valore, scala e firma dimensione/unità di `Quantity` sui punti
  con span esatto; le conversioni di presentazione preesistenti restano nel
  chart layer, con fallback compatibile per punti privi di unità nota.
- Il consumer Go decodifica e convalida span, unità, riferimenti e periodi; i
  test mirati coprono euro scalati, popolazione, percentuali, intervalli,
  confronti incompatibili, anni/frequenze negative e un campione di unità per
  ogni lingua.
- `semantic-engine/fixtures/quantities/score_quantity_gold.py` esercita ora
  esempi di rapporto localizzati in tutte le 11 lingue, oltre ai casi di
  confronto incompatibile. Il report draft
  attuale mostra 1.533/1.533 positivi corretti su 1.729 casi totali,
  valore/scala/unità/span esatti al 100%, recall qualificatore 11/11,
  confronto/rapporto 11/11 e zero falsi positivi. Il gold ha anche trovato e fatto
  correggere il moltiplicatore `millón` accentato in spagnolo e la precedenza
  errata della lunghezza `km` sul composto `km/h`.

Il ticket resta **PARTIAL**: le quantità positive ora ruotano fra tre contesti
per lingua e i rapporti sono coperti in tutte le lingue, ma i testi e le
annotazioni restano draft, i negativi sono ripetuti e non c'è revisione
indipendente. Servono contesti più vari e una copertura più ampia dei tipi di
confronto prima di usare i numeri come criterio di rilascio.

## Lavoro rimanente

1. Ampliare i casi in frasi naturali varie, includendo forme/scale, tutti i
   qualificatori, legami temporali e confronti previsti.
2. Far revisionare indipendentemente le annotazioni draft e registrare i
   risultati per lingua e classe dimensionale. Il report è un gate sintetico,
   non una misura di copertura reale.
3. Solo se tutti i
   threshold di accettazione sono soddisfatti, aggiornare lo stato a CLOSED.

## Verifica 2026-11-20

Ricontrollato sul working tree corrente (`2bbeca1` più le modifiche non
committate). In questa passata il runtime quantità non è cambiato; sono stati
ampliati i contesti del draft scorer e rigenerato il report con il worker
release corrente.

- Implementazione presente e collegata: `lexicon/units.json` e i pack
  generati, `semantic-core/src/{units,quantity}.rs`,
  `semantic-engine/src/quantities.rs`, i campi
  `SceneManifest.quantities` / `quantity_comparisons` e il chart bridge.
- `semantic-engine/fixtures/quantities/report.json` (schema
  `brainrust.quantity_gold_report.v1`) è stato rigenerato il 2026-11-20 dopo
  una build release aggiornata del worker e l'espansione da tre a sei contesti
  per lingua. Il worker SHA-256 nel report è
  `8ca393bcbdfe15696e9b6c1cfa1df9dc43e42b4e220f6c1f80545043891bb7f6`.
  Il report
  riporta **1.686 casi** (153 per lingua, 156 in italiano; 11 lingue),
  **1.469/1.469** quantità positive estratte e abbinate, **0** falsi positivi
  sui 220 negativi e **0** estrazioni non attese nei casi positivi; valore,
  scala, unità e span esatti. `annotation_authority` è
  `assistant_draft_pending_human_review`.
- Verificato in questa passata: `cargo build -p semantic-engine --release` e
  `python3 semantic-engine/fixtures/quantities/score_quantity_gold.py --worker
  target/release/semantic-engine --report
  semantic-engine/fixtures/quantities/report.json` — **PASS**. Il report
  registra l'hash del worker usato. Il gate 30c di rerun locale è completato.
- La CI esegue lo stesso scorer sul binario release
  (`.github/workflows/ci.yml`), quindi il gate non dipende dal laptop.
- Resta **PARTIAL**: mancano la revisione indipendente delle annotazioni e
  contesti naturali indipendenti più vari (qualificatori e confronti compresi).

## Piano di chiusura e split (2026-11-20)

Il codice è completo; il ticket non è chiudibile perché il gold è una bozza
generata dall'assistente e copre pochi contesti ripetuti. La chiusura è quindi
un lavoro dati + revisione, non un'altra feature. Split in 3 parti.

### 30a — Matrice naturale e copertura (S/M, parte subito)

- Sostituire i tre template fissi per lingua in `score_quantity_gold.py` con
  frasi naturali varie (cronaca, rapporto, intervista), mantenendo le 13
  dimensioni e i 20 negativi per lingua.
- Coprire esplicitamente tutti i qualificatori (`exact`, `approx`,
  `lower_bound`, `upper_bound`, `range_start`, `range_end`) e i quattro
  confronti (`ratio`, `difference`, `change`, `rank`), prima it/en e poi le
  altre 9 lingue.
- Aggiungere i negativi di confronto: valute diverse, dimensioni diverse,
  periodi diversi, `3 volte`/`3x`, anni e numeri civici.
- Rigenerare `report.json` con un worker release ricompilato; il report resta
  `assistant_draft_pending_human_review`.
- Fatto quando: 0 falsi positivi e 0 confronti cross-valuta, tutti i
  qualificatori e i 4 confronti coperti, `charts_numeric` e i chart golden
  invariati.

### 30b — Revisione indipendente (M, esterna)

- Un revisore che non ha visto l'output del motore annota un sottoinsieme
  held-out (almeno 10 misure per dimensione per lingua) e registra id/data nel
  report.
- Calcolare precision/recall per lingua e per dimensione sulle righe riviste;
  aggiornare l'authority a `human_annotator` solo per quelle righe.
- Fatto quando: report firmato e riproducibile. È lo stesso blocco esterno di
  TICKET-08 e non può essere prodotto dal codice.

### 30c — Chiusura (S)

- Rerun completo: `score_quantity_gold.py` exit 0,
  `cargo fmt --all -- --check`, `cargo clippy -p semantic-core
  -p semantic-engine --all-targets -- -D warnings`,
  `python3 lexicon/tools/build_language_packs.py --check`.
- Se cambia il comportamento, bump `AnalyzerVersions.quantities`, copia Go e
  goldens; aggiornare la matrice a CLOSED.

Ordine: **30a → 30b → 30c**. Solo 30b richiede un owner umano; 30a e 30c sono
in questo repository.

## Chart bridge follow-up — 2026-11-20

- `chart_multilingual` now checks annual series and scaled EUR comparisons in
  all ten chart locales. It verifies exact chart-item source spans, year-bound
  labels, category labels, and rejection of mixed EUR/USD comparisons.
- The Polish unit pack now recognizes inflected `dolarów`/`dolara` and
  `funtów`/`funta`; `python3 lexicon/tools/build_units.py --check` passes.
- `chart_acceptance_36` case 33 now asserts that `20 euro al mese` and
  `150 euro all’anno` remain visible in the chart item evidence. Workspace
  tests, formatting and Clippy pass on the updated tree.
- These scenario sentences are an engineering regression draft, not a
  replacement for the independent quantity annotations required to close the
  ticket.

## Qualifier, range and localized comparison follow-up — 2026-11-21

- Expanded the quantity draft matrix to **1,729 cases** (1,533 positive
  quantity expectations; 220 negative forms), adding localized lower/upper
  bounds and range pairs across all 11 languages. All 1,533 expected
  quantities match; qualifier recall is 100% in each language and the
  negative/positive false-positive counts are zero. Added one ratio comparison
  expectation per language; all 11 match using language-scoped ratio cues.
- The new cases exposed two engine defects: range-ending conjunctions were
  missing for Indonesian, Dutch and Russian (and Turkish range cues were not
  fully closed), and the shorter Turkish cue `fazla` overrode the more
  specific `en fazla`. Range connectors now include those localized forms;
  qualifier selection uses the longest matching cue.
- Verified with the release worker scorer, `cargo test -p semantic-engine
  quantities:: --locked`, complete `semantic-engine` tests (318 passed),
  Clippy, formatting and `git diff --check`.
- 30a remains partial: the matrix is still an assistant draft, and the four
  comparison kinds do not yet have broad multilingual coverage. 30b remains
  blocked on independent human review, so this work does not close TICKET-30.

## Recheck — incompatible comparison negatives — 2026-11-21

- Added three comparison-cue cases per language to the draft scorer: distinct
  currencies, distinct dimensions, and distinct currency periods. Each case
  still expects both source quantities to be extracted exactly; only a
  `quantity_comparison` is forbidden. This tests the compatibility guard rather
  than passing by dropping the quantities.
- Release report: **1,762 cases**, **1,599/1,599** positive quantity matches,
  zero quantity false positives (including the 220 original negative forms),
  **11/11** expected ratio comparisons matched and zero false comparisons in
  all 11 languages. The 33 new incompatible comparisons produced no comparison
  output. Worker SHA-256 matches `target/release/semantic-engine`:
  `56c80427c9fba1354745423d1242dfd630fde3978fb0d2fdca69bd0793345a73`.
- Verified: `python3
  semantic-engine/fixtures/quantities/score_quantity_gold.py --worker
  target/release/semantic-engine --report
  semantic-engine/fixtures/quantities/report.json` — **PASS**. This advances
  the negative comparison gate; positive `difference`, `change` and `rank`
  coverage, broader natural contexts and independent annotation review remain.

## Recheck — localized difference comparisons — 2026-11-21

- Added a grounded `difference` expectation in each of the 11 languages using
  its localized comparison cue. Both percentage values must be emitted and
  their arithmetic difference must equal 5 exactly.
- The release report now covers **1,773 cases** and **1,621/1,621** positive
  quantities. Every language matches both its ratio and difference comparison
  (22/22 total); all 33 incompatible comparisons and 220 quantity negatives
  remain false-positive-free. All scorer failures are zero. Report and worker
  SHA-256: `56c80427c9fba1354745423d1242dfd630fde3978fb0d2fdca69bd0793345a73`.
- The release scorer passes on the current worker. Positive `change`/`rank`
  patterns, wider natural contexts and independent annotation review remain;
  this is still draft evidence, not closure of TICKET-30.

## Recheck — change comparisons — 2026-11-21

- Added per-language `change_signals` and a `change` comparison kind. It records
  `new_value - previous_value`, so explicit increases yield a positive delta
  and decreases a negative one. The change cue span is retained as evidence;
  Turkish permits the finite change cue immediately after the second value.
- Bumped `AnalyzerVersions.quantities` to
  `typed_quantities.v2.change_signals`. Unit tests cover English rise/fall and
  Turkish rising forms; the release scorer passes one ratio, one difference,
  and one change case in each of the 11 languages.
- Release report: **1,784 cases**, **1,643/1,643** quantity matches, **33/33**
  positive comparisons, **33** incompatible comparisons rejected, and zero
  quantity/comparison false positives. Worker SHA-256:
  `7eb6b270c8913626f9da87e9445f8332cb04bd431e2a27804c8b781a628931c7`.
- Verified on the updated source: `cargo test -p semantic-engine --locked`,
  `cargo clippy -p semantic-engine --all-targets --locked -- -D warnings`, the
  chart acceptance/multilingual/bare-cue/numeric suites, formatter, diff check,
  and `build_language_packs.py --check` all pass. Cross-repo projection also
  passes with the current release worker:
  `VELOX_RUST_SEMANTIC_ENGINE_PATH=../BrainRust/target/release/semantic-engine
  go test ./internal/platform/media/rustexec` — PASS.
- Ranking remains represented by the existing `SemanticGroup` ranking shape,
  rather than by a numeric `QuantityComparison`. Added a regression combining
  ten typed EUR quantities with a complete Top 10 group and asserting that rank
  numbers never become a chart. Broader natural contexts and independent
  annotation review remain; `report.json` is still assistant-draft evidence.
- Rebuilt the release worker after the Spanish role lexicon was added and
  regenerated both draft scorers against the identical worker hash
  `047d7c70ae33056a87ffb28ae9b37fff06bf33b666a3c201ccc85d25b1455cd1`.
  Quantity totals remain unchanged at 1,784 cases, 1,643/1,643 expected
  quantities and zero false positives.
