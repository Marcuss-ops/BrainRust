# Typed quantity matrix

`score_quantity_gold.py` runs the semantic-engine worker over 53 cases in each
of the 11 supported languages: 33 positive quantities (10 currency, 10
percent, 10 length, plus one scale, one approximate qualifier and one period
binding) and 20 negative numeric forms. It reports exact-value/scale, unit,
qualifier, period, grounding and negative-set measurements by language and
positive dimension.

The matrix is explicitly
`assistant_draft_pending_human_review`. It currently uses short, mostly
language-neutral unit fragments and repeated negative forms; its perfect
mechanical score is a smoke/regression measure, not evidence of natural-language
recall. TICKET-30 remains PARTIAL until the per-language cases are expanded to
varied source contexts and independently reviewed, including the remaining
dimensions, qualifiers and comparison cases.

Run after building the worker:

```sh
python3 semantic-engine/fixtures/quantities/score_quantity_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/quantity-gold-report.json
```
