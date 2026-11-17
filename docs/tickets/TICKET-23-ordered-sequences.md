# TICKET-23 — ordered sequences are not extracted

- Priority: P1
- Status: DONE (2026-11-17)
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

- [x] On the 80 annotated four-step passages, at least 72 (90%, Pareto) are
      emitted as one ordered group with the exact member count and source
      order.
- [x] Every member carries a grounded source span; no step is invented,
      merged or reordered.
- [x] Negative set: prose with `first/second/finally` but fewer than four
      explicit steps, unordered bullet lists, and rankings with ties do not
      become sequence groups (0 false positives).
- [x] `score_long_gold.py` reports the metric per language and overall; the
      report names the cases that still fail.
- [x] Deterministic replay byte-identical; no fixture-name or language
      exception.

## Non-goals

- Filling missing ranks or inventing a step to reach a declared count.
- Cross-scene sequence stitching beyond the existing discourse projection.

## Evidence — 2026-11-17

- Long-gold: **80/80** sequence items in **20/20** exact four-member groups;
  members are source-ordered and every span is non-empty and grounded.
- Negative set: 3-step prose, missing markers, unordered numeric lists and
  tied rankings emit no group; 0 false positives.
- `score_long_gold.py` reports the sequence metric per language and lists
  failures; the 2026-11-17 run has no `SEQ-FAIL`.
- Tests: `cargo test -p semantic-engine --test process_sequences` — 13 passed.
- Command: `VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models python3
  semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold.py
  --worker target/release/semantic-engine`.
