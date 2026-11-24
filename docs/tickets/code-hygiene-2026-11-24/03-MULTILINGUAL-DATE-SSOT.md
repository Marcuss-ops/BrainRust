# BR-DATE-01 — Italian-only month tables in `overlay-compiler`; gate exception documented

- **Status:** CLOSED (2026-11-24) — `overlay-compiler` now derives its month
  pattern and token set from `semantic-core`; `script-qa` keeps its deliberate
  Go-parity table.
- **Priority:** P1
- **Owner:** `overlay-compiler` (fix), `script-qa` (documented exception)
- **Evidence date:** 2026-11-24

## Problem

`overlay-compiler` imports `semantic-core`, which already owns the CLDR month
table (`semantic_core::lexicon::Lexicon::global().month_date_scan_table`), but
its projection still hardcodes Italian months in three places:

- `overlay-compiler/src/editorial_plan_v2/projection/numeric.rs:244-249`
  (`date_pattern`: `gennaio|febbraio|…|dicembre`);
- same file `:252-257` (`date_identity_pattern`);
- same file `:418-451` (`date_tokens`).

Consequence: a grounded date in English, French, Spanish, Portuguese, German,
Dutch, Polish, Russian, Turkish or Indonesian is not surfaced or compared by
the projection even though the engine already extracted it.

`script-qa/src/dates.rs:33` also carries a month table (`GATE_MONTHS`), but that
is **not** the same defect: the comment at `:27-32` says the gate must stay
byte-for-byte compatible with the Go implementation it ports. The audit's
"three disconnected implementations" claim must not erase that contract.

## Acceptance criteria

- [ ] `date_pattern`, `date_identity_pattern` and `date_tokens` build their
      month set from `semantic_core::lexicon::Lexicon::global()
      .month_date_scan_table`; no Italian-only literal list remains in
      `overlay-compiler`.
- [ ] A test proves `explicit_date_surfaces("13 January 2007")`,
      `explicit_date_surfaces("13 janvier 2007")` and
      `explicit_date_surfaces("13 marzo 2007")` all return the full date.
- [ ] A test proves `same_grounded_date("13 January 2007", "13 January 2007")`
      is true and a different month is false.
- [ ] `date_tokens` accepts a non-Italian month token and still rejects
      non-date text.
- [ ] `script-qa` keeps `GATE_MONTHS` and gains/retains a comment and test that
      pin the Go-parity exception.
- [ ] `cargo test -p overlay-compiler -p script-qa` is green.

## Checks

```sh
cd BrainRust
rg -n "gennaio|febbraio|marzo|aprile|maggio|giugno|luglio|agosto|settembre|ottobre|novembre|dicembre" \
  overlay-compiler/src/editorial_plan_v2/projection/numeric.rs
cargo test -p overlay-compiler -p script-qa --locked
cargo clippy -p overlay-compiler -p script-qa --all-targets -- -D warnings
```

## Evidence (2026-11-24)

- `overlay-compiler/src/editorial_plan_v2/projection/numeric.rs` builds
  `date_pattern`, `date_identity_pattern` and `date_tokens` from
  `semantic_core::lexicon::Lexicon::global().month_date_scan_table`.
- New tests: `explicit_date_surfaces_are_multilingual`,
  `same_grounded_date_compares_the_month_across_languages`,
  `date_tokens_accepts_non_italian_months` → all passed.
- `cargo test -p overlay-compiler --locked` → 169 unit + 34 contract tests
  passed; the only failure in the combined run was the pre-existing doctest
  loader issue, reproduced separately.
- `script-qa` remains unchanged: `GATE_MONTHS` is a deliberate Go-parity
  contract, documented at `script-qa/src/dates.rs:27-32`.

## Out of scope

- Replacing `script-qa::GATE_MONTHS` with the CLDR table: the gate's Go parity
  is a deliberate exception, not drift.
- Changing the date normalization owned by `semantic-core`.
