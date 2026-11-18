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

## Review packet (2026-11-17)

`semantic-engine/fixtures/gold_multilingual/long_scripts/review_packet.py`
exports the 19 drafts for a human reviewer without mutating `gold.json`:

- `BrainRust/docs/reviews/TICKET-08/review_packet.json` — machine-readable
  packet with source text, current annotations and checklist.
- `BrainRust/docs/reviews/TICKET-08/review_packet.md` — reviewer-facing
  version of the same 19 drafts.

Every entry keeps `authority: assistant_draft_pending_human_review` and
`review_status: pending_human_review`. The pilot (`it_stadium_long_01`) is
excluded; the packet does not promote anything to `human_annotator`.

Regression command:

```sh
cd semantic-engine/fixtures/gold_multilingual/long_scripts
python3 -m unittest review_packet_test.py -v   # 4 passed
python3 review_packet.py --output-dir ../../../../docs/reviews/TICKET-08
```

This packet helps the reviewer but does not satisfy the independence blocker:
an independent human still has to annotate the held-out set. TICKET-08 remains
**BLOCKED**.

## Next owner action

Freeze the annotation protocol and the 60/20/20 split, have a second reviewer
label the held-out portion, then add the measurement harness against that
frozen set. Use the review packet as the review queue for the 19 existing
drafts, not as evidence of independent annotation.
