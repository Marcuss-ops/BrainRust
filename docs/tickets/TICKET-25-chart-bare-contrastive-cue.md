# TICKET-25 — chart recall stops at bare `a`/`а`

- Priority: P2
- Status: DONE (2026-11-17)
- Source: `PER-LANGUAGE-GOLD-REPORT-2026-10-28.md` §6
- Files: `semantic-engine` chart rules, long-gold fixtures

## Problem

Chart recall is 17/20. The three misses use only a bare `a`/`а` as the
contrastive cue; treating it as a global chart marker is unsafe because it is
also an ordinary conjunction. The case is therefore neither a threshold nor a
lexicon gap: it needs a context rule with a precision gate, or a written
decision to keep the 17/20 bound.

## Acceptance criteria

- [x] Either 20/20 with 0 false positives on a 20-paragraph negative set where
      `a`/`а` is an ordinary conjunction, or the 17/20 bound is accepted in
      writing and removed from the open-work list.
- [x] Any new rule requires a numeric/quantity contrast inside the same
      sentence or an explicit paired structure; it is not a bare cue.
- [x] The metric and the negative set are part of the per-language report.
- [x] No fixture-name or language exception.

## Non-goals

- Recovering chart intent from arbitrary prose without a contrastive signal.
- Changing the chart output schema.

## Evidence — 2026-11-17

- Chart recall **20/20**; the 20-paragraph negative set where `a`/`а` is an
  ordinary conjunction produces **0 charts**.
- Rule: the bare cue binds only when two grounded values straddle it on the
  same line and substantive words occur on both sides. It is not added to the
  global chart vocabulary; Italian `a` remains excluded.
- `score_long_gold.py` reports `chart_negative_set.cases = 20` and
  `charts = 0`; the metric is part of the per-language report.
- Tests: `cargo test -p semantic-engine --test chart_bare_cue` — 4 passed.
