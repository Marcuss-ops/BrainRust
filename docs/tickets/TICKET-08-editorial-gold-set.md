# TICKET 08 — independent editorial gold set

- Priority: P1
- Status: BLOCKED
- Source: audit roadmap PR 8
- Files: `ACCEPTANCE_TODO.md`, editorial fixtures

## Problem

The documented 20-long-script measurements are not an independent gold set:
19 of 20 scripts still carry machine-produced annotations awaiting human
review. Technical tests cannot certify editorial precision.

## Acceptance criteria

- A held-out set of scripts annotated by a reviewer who did not see the engine
  output.
- Per-language, per-category and per-error-type measurements.
- Recall, surface recognition, typing, query quality and ordering measured
  against those labels.
- Any “90% confidence” claim backed by a calibrated, independently judged
  association set, not a ranking score renamed as probability.

## Blocker

This requires a human annotation owner and a reviewed dataset. It cannot be
manufactured from the existing automatic annotations without invalidating the
independence the audit requires. No code change in this repository can close
it.

## Next owner action

Freeze the annotation protocol and the 60/20/20 split, have a second reviewer
label the held-out portion, then add the measurement harness against that
frozen set.
