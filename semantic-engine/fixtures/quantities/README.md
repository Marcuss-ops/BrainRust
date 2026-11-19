# Typed quantity matrix

`score_quantity_gold.py` runs the worker over 153 cases per language (156 for
Italian): 133 draft positive quantities per language, with 10 examples for
each of the 13 unit dimensions, plus scale, approximate qualifier and period
cases; it also includes 20 negative numeric forms per language. Three extra
Italian cases verify an exact ratio and reject cross-currency and
cross-dimension comparisons. The matrix totals 1,469 positive quantities and
220 negative forms, and reports value/scale, unit, qualifier, period,
comparison, grounding and negative-set measurements by language and dimension.

The matrix is explicitly
`assistant_draft_pending_human_review`. The cases use short unit phrases and
repeated negative forms; their mechanical score is a regression measure, not
evidence of natural-language recall. TICKET-30 remains PARTIAL until the cases
are written in varied natural contexts and independently reviewed, with broader
qualifier and comparison coverage.

Run after building the worker:

```sh
python3 semantic-engine/fixtures/quantities/score_quantity_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/quantity-gold-report.json
```
