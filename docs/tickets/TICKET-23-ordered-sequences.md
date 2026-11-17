# TICKET-23 — ordered sequences are not extracted

- Priority: P1
- Status: OPEN
- Source: long-gold report 2026-10-28 §6; `docs/stato/brainrust.md` §2
- Files: `semantic-engine`, `semantic-core`, `semantic-engine/fixtures/gold_multilingual`

## Problem

`sequences_exactly_extracted = 0/80` on the long gold. A four-step passage is
not projected as an ordered group: the manifest can describe enumerations and
rankings, but it does not emit the members in source order with per-member
grounded evidence. The 2026-10-28 closeout measured this as a missing
capability, not a threshold, so lowering a score would only produce partial
lists.

## Acceptance criteria

- [ ] On the 80 annotated four-step passages, at least 72 (90%, Pareto) are
      emitted as one ordered group with the exact member count and source
      order.
- [ ] Every member carries a grounded source span; no step is invented,
      merged or reordered.
- [ ] Negative set: prose with `first/second/finally` but fewer than four
      explicit steps, unordered bullet lists, and rankings with ties do not
      become sequence groups (0 false positives).
- [ ] `score_long_gold.py` reports the metric per language and overall; the
      report names the cases that still fail.
- [ ] Deterministic replay byte-identical; no fixture-name or language
      exception.

## Non-goals

- Filling missing ranks or inventing a step to reach a declared count.
- Cross-scene sequence stitching beyond the existing discourse projection.

## Next action

Add the 80 passages to the long-gold sequence case list, write the failing
assertion against the current manifest, then implement the ordered projection
in `semantic-engine` using the existing grounded enumeration groups.
