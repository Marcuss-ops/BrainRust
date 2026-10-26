# Independent long-form editorial gold

`gold.json` is a hand-authored pilot, kept separate from the 100 templated
multilingual smoke corpus. Its annotations were written from the script before
the release worker was run. The 584-word Italian script covers named and
visual entities, complete phrases and segments, asserted/negated facts, a
percent comparison with a numeric negative control, an ordered four-step
sequence, and a derived subject-place image query.

The scorer uses exact normalized text for important phrases and segments, and
reports both typed and surface-only visual-object matching. It checks fact
subject/predicate/value/status, chart values and chart type, contextual-query
components and UTF-8 source spans, exact sequence-item extraction, and cached
manifest replay. When multiple asserted facts about one subject share the
same sentence evidence, BrainRust can retain that complete sentence as a
language-neutral extractive micro-summary linked to all supporting fact IDs.
Critical summary invariants and summary candidates are emitted for human
editorial review; the scorer does not claim to judge paraphrase faithfulness.

This is a development pilot, **not** a release gate: it has one script in one
language. The scorer exits with status 2 until there are at least 20 scripts
covering all ten corpus languages.

Run after building the release worker:

```sh
cargo build --release -p semantic-engine
python3 semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold.py
```

The report defaults to `/tmp/brainrust_long_gold_report.json`.
