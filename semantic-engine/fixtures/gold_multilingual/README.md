# Multilingual gold set (10 languages × 2 scripts)

Twenty short documentary scripts — two per language (de, en, es, fr, id, it,
nl, pl, pt, ru) — with human-style annotations in `gold.json`, and a scorer
that runs the release worker over them.

This remains a short named-entity/noise suite. A separate independent
long-form editorial gold pilot is being built under
[`long_scripts/README.md`](long_scripts/README.md); it is not yet release-sized.

## What is scored

The certified named-entity set only: `PERSON`, `LOCATION`, `ORG`, `WORK`,
`BRAND`, `PRODUCT`. The curated object slice (T05) and the value types
(DATE/NUMBER/MONEY/PERCENT) are outside this gold: the object slice is
explicitly not complete-recall by contract, and values are covered by the
charts & bullets suites.

Noise is scored with the criterion's rules:

* a filler phrase or a chapter heading must never appear inside a query or an
  entity;
* a bare pronoun must never BE a query or an entity.

## Thresholds

recall ≥ 0.95, precision ≥ 0.98, noise leaks = 0.

## Provenance

The annotations in `gold.json` (`annotation`) are written from linguistic
judgment — never copied from engine output and never derived from the
filler/filter data the engine consumes. A gold entity the engine misses stays
a MISS in the report; the scorer never rewrites the gold.

Documented scope notes (see `gold.json -> annotation.language_notes`):

* English ships an empty legal-cue table, so the English scripts carry no
  institution mention.
* Polish/Russian scripts use dictionary-form place surfaces (Warszawa,
  Казань): inflected Slavic place forms are outside the current place-index
  contract.

## Running

```
cargo build --release -p semantic-engine
python3 semantic-engine/fixtures/gold_multilingual/score_gold.py
```

The scorer drives `target/release/semantic-engine` over stdin/stdout
(`operation: analyze_script`, the same protocol as
`fixtures/multilingual_100/benchmark.py`), prints the per-script table and
writes a JSON report (`--report`, default `/tmp/gold_score_report.json`).
Exit code 0 means every threshold is met.
