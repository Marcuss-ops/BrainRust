# Per-language gold report — 2026-10-27

Gate C asks for per-language gold precision/recall and critical errors over the
full semantic annotation set, and for an inflection/compound/script/morphology
review that does not infer language quality from a smoke phrase. This report
records what was measured on 2026-10-27 on this host, with the release worker
built from `main@03a671d` plus the fmt/clippy repair of the same day, and states
every limit explicitly.

## 0. Measurement precondition (read this first)

Every number below was produced with the UDPipe models exported:

```sh
cd BrainRust
export VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models      # or ~/.local/share/visualner/models
export RUST_ICU_MAJOR_VERSION_NUMBER=70
```

Without `VISUALNER_MODEL_DIR` the worker logs
`visualner: no UDPipe model installed for <lang>; POS evidence and verb
filtering are disabled` and the same corpora measure **materially worse**:

| Corpus | Without models | With models |
|---|---|---|
| Short gold, entity recall | 0.959 (German 0.600) | **1.000** (every language 1.000) |
| Long gold, visual objects | 8/10 | **10/10** |
| 100-script corpus | 63 PASS / 37 NEEDS_REVIEW, review items = `missing visual object: …` | **100 PASS**, zero review items |
| Workspace tests | `every_mention_of_eight_people…` FAILED | **553 passed, 0 failed** |

The earlier downloads live in `BrainRust/.tmp/udpipe-models/` (13 models,
Italian included). Any run that omits the variable is a *degraded-mode* run and
must be labelled as such; the 2026-10-27 acceptance claims are reproducible only
with it.

## 1. Commands and artifacts

| Artifact | Command |
|---|---|
| `gold_multilingual/reports/short_gold_report_2026-10-27.json` | `python3 score_gold.py --report …` |
| cold replay (not archived) | `python3 score_gold.py --check-reproducible` |
| `gold_multilingual/reports/long_gold_report_2026-10-27.json` | `python3 score_long_gold.py --report …` |
| corpus shape | `python3 long_scripts/validate_long_gold.py` |
| `multilingual_100/reports/brainrust_100_2026-10-27-models.json` | `python3 benchmark.py --worker target/release/semantic-engine --report …` |
| Go projection (`refactored`) | `go test ./internal/platform/media/rustexec/... -count=1` |

## 2. Short gold — entities and noise, per language

Global: recall **1.000** (109/109), precision **0.991** (107/108), noise leaks
**0**, scorer exit **0 (PASS)**.

| Lang | Scripts | Gold | Found | Recall | Precision | Critical errors |
|---|---:|---:|---:|---:|---:|---|
| de | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| en | 2 | 9 | 9 | 1.000 | 1.000 | 0 |
| es | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| fr | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| id | 2 | 10 | 10 | 1.000 | **0.923** | 0 misses, 1 false positive (`Bukan` as LOCATION) |
| it | 2 | 11 | 11 | 1.000 | 1.000 | 0 |
| nl | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| pl | 2 | 9 | 9 | 1.000 | 1.000 | 0 |
| pt | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| ru | 2 | 9 | 9 | 1.000 | 1.000 | 0 |

Zero typed misses. The single critical fact is one Indonesian false positive:
`Bukan` ("not") extracted as a LOCATION — a negation word leaking into the
place detector, worth a targeted fix.

Determinism: `--check-reproducible` → `PASS (20 scripts × 2 cold runs,
byte-identical manifests)`.

## 3. Long-form gold — one script, one language

`long_scripts/gold.json` holds the hand-annotated Italian pilot
(`it_stadium_long_01`, 584 words). Scorer output with models:

```
{"script_count": 1, "language_count": 1, "languages": ["it"], "word_count": 584,
 "is_release_sized": false, "annotation_authority": "human_annotator",
 "certification": "blocked_pending_release_size"}
it: entities=0.556 (5/9) objects=1.000 (10/10) phrases=1.000 (6/6)
    facts=0.429 (3/7) charts=1.000 (1/1) queries=1/1
    grounding-errors=0 replay-identical=True
```

Per-category reading:

- **Entities 5/9.** Misses: `Comune di Torino` (ORG) and `Università di Torino`
  (ORG) — compound organisation names built with `di` + place. False
  positives: `Aurora Sport` and `Lumen Energia` extracted as **PERSON** instead
  of ORG (suffix-less company names).
- **Facts 3/7.** The three `Stadio Aurora` facts pass (`inaugurated_year 1961`,
  `located_in Torino`, `capacity 18.000`). The two
  `estimated_saving_percent` facts miss **because of the ORG typing defect
  above** (their subjects became PERSON entities), and the two negated
  invariants (`pompa | fault = false`, `risparmio annuale | demonstrated =
  false`) are not emitted at all: negation of a fault/saving is not modelled in
  entity facts.
- **Objects 10/10, phrases 6/6, segments 1/1, charts 1/1 (typed PERCENT),
  contextual query 1/1, zero grounding errors, identical cached replay.**

`validate_long_gold.py` additionally enforces the corpus shape (`>= 250` source
words, category types, no placeholders) and the **per-language category
scope**: the facility predicates exist only in `lexicon/facility_facts.json`,
which is Italian-only, so a gold expecting them for another language is refused
by the validator — it would measure the gold, not the engine. Off-vocabulary
`OBJECT` terms are warnings (deliberate recall probes are allowed but visible),
and `annotation_policy.authority` drives the reported `certification`.

Scoreability correction applied to the pilot (recorded, not silent): two
negated-fact subjects used English glosses (`pump`, `annual_saving`) in an
Italian script, where the scorer requires the subject surface to appear in the
engine's evidence sentence. They now read the source forms `pompa` and
`risparmio annuale`. **The metric did not move (3/7 before and after)** — the
correction removed a label artefact and confirmed those two facts as genuine
engine misses.

To lift this gate a human annotator must add 19 long scripts covering the nine
missing languages; `score_long_gold.py` then stops exiting 2. Draft labels must
not be promoted to `human_annotator` without review.

## 4. Template corpus — 100/100 with models

Re-run on 2026-10-27 with models: **100 PASS, 0 structural problems, 431.6 s**
for ~203k words across the ten languages (10/10 per language), reproducing the
archived `brainrust_100_all_pass_2026-10-25.json` (100 PASS, 411 s). The
runner checks schema, scene counts, byte-grounded entities/bullets/phrases/
quotes, grounded facts, grounded visual queries and contextual-query component
integrity.

Coverage observation (not a defect claim): the fixture authors' object anchors
are largely outside the shipped `lexicon/object_vocabulary.json` (99/100,
18–57 terms per language). With models installed the engine recalls them anyway
through the POS/VisualNER path, so the vocabulary list is not the binding
constraint the way it looks — the measured 100 PASS takes precedence over the
static reading.

## 5. Morphology, compounds, scripts — what was reviewed

| Language | Cases | Anchors exercised per case | Observed 2026-10-27 |
|---|---:|---|---|
| de | 10 | compound ORGs (`Prisma-Labor`, `Stella-Rat`, `Aurora-Institut`, `Lumen-Genossenschaft`, `Horizont-Stiftung`), hyphenated persons, `elektrische Straßenbahn`, `Wasserstandssensor` | 10/10 PASS; short-gold persons 10/10 |
| nl | 10 | particles, `noodaggregaat`, `waterstandsensor` | 10/10 PASS |
| pl, ru | 10 each | inflected surnames, transliterated facilities | 10/10 PASS |
| id | 10 | one-word surnames, no morphological marking | 10/10 PASS; short-gold `Bukan` FP |
| it, es, pt, fr | 10 each | articulated prepositions, accented surnames, elision | 10/10 PASS |
| en | 10 | anglo compounds (`water-level sensor`) | 10/10 PASS |

Compound handling is exercised at scale for German (five hyphenated ORG
families, 20k words) and Dutch, and the short-gold German persons that a
model-less run missed are all recovered with models. The Italian long gold then
shows the residual compound limit in the other direction: `Comune di Torino`
and `Università di Torino` are still not formed as ORG. Neither corpus
certifies full language quality, and this report does not claim it.

## 6. Open engine gaps, ranked by measured impact

1. **Suffix-less company names typed PERSON** (`Aurora Sport`, `Lumen Energia`)
   → costs 2 entities and 2 percent facts on the Italian long gold.
2. **Compound ORG with `di` + place** (`Comune di Torino`,
   `Università di Torino`) not formed → 2 entities.
3. **Negated fact invariants not emitted** (`fault = false`,
   `demonstrated = false`) → 2 facts; the pilot's whole point is that a negated
   claim must survive extraction.
4. **`Bukan` (id) typed LOCATION** → the only precision loss in the short gold.

Fixes 1–3 are engine work in `semantic-engine`; none of them may be closed by
tuning a fixture. Fix 4 is a negation-cue guard in the place detector.

## 7. What this report certifies

- **Certified here:** per-language entity recall/precision and critical-error
  counts over the 20-script gold (all ten languages at recall 1.000, one
  precision loss); zero noise leaks; byte-identical cold replay; 100-script
  structural grounding with zero problems, reproduced with models; the
  corpus-shape, language-scope and provenance rules for the long gold; the
  measurement precondition (`VISUALNER_MODEL_DIR`) that makes any of it
  reproducible.
- **Not certified:** editorial quality in the nine languages without long gold;
  summary faithfulness and date/subject/negation invariants on long scripts
  (human review pending); image-query precision@10 (needs a provider run);
  object recall as a *metric* (the corpus reports structural grounding, not
  object precision/recall); production provider cascades.
