# Typed quantity matrix

`score_quantity_gold.py` runs 1,784 cases: 1,643 expected quantities, 220
negative quantity forms, localized ratio/difference/change comparisons, and
33 incompatible comparison scenarios (cross-currency, cross-dimension and
different percentage scales). Positive measurements span all 13 unit
dimensions and rotate through varied templates; the report measures
value/scale, unit, qualifier, period, comparison, grounding and negative-set
results by language and dimension.

The matrix is explicitly
`assistant_draft_pending_human_review`. Context frames and negative forms are
still draft and synthetic; their mechanical score is a regression measure, not
evidence of natural-language recall. Localized ratio, difference and change
comparisons are covered, as are incompatible pairs, but broad natural contexts
and annotations still require independent review. TICKET-30 remains PARTIAL.

Run after building the worker:

```sh
python3 semantic-engine/fixtures/quantities/score_quantity_gold.py \
  --worker target/release/semantic-engine \
  --report /tmp/quantity-gold-report.json
```
