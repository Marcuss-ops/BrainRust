# TICKET-25 — chart recall stops at bare `a`/`а`

- Priority: P2
- Status: OPEN
- Source: `PER-LANGUAGE-GOLD-REPORT-2026-10-28.md` §6
- Files: `semantic-engine` chart rules, long-gold fixtures

## Problem

Chart recall is 17/20. The three misses use only a bare `a`/`а` as the
contrastive cue; treating it as a global chart marker is unsafe because it is
also an ordinary conjunction. The case is therefore neither a threshold nor a
lexicon gap: it needs a context rule with a precision gate, or a written
decision to keep the 17/20 bound.

## Acceptance criteria

- [ ] Either 20/20 with 0 false positives on a 20-paragraph negative set where
      `a`/`а` is an ordinary conjunction, or the 17/20 bound is accepted in
      writing and removed from the open-work list.
- [ ] Any new rule requires a numeric/quantity contrast inside the same
      sentence or an explicit paired structure; it is not a bare cue.
- [ ] The metric and the negative set are part of the per-language report.
- [ ] No fixture-name or language exception.

## Non-goals

- Recovering chart intent from arbitrary prose without a contrastive signal.
- Changing the chart output schema.

## Next action

Build the 20-paragraph negative set first, then test the contextual rule
against the three misses and the negative set in one run.
