# TICKET 11 — deterministic discourse continuity

- Priority: P0
- Status: DONE (deterministic core)
- Source: 2026-11-11 narrative-continuity proposal, PR 1–2
- Files: `semantic-core/src/narrative.rs`, `semantic-engine/src/discourse/`,
  `semantic-engine/tests/discourse.rs`, `docs/operations/discourse.md`

## Problem

The engine shared one `IdentityRegistry` across scenes, but pronouns, role
nominals, topic continuity and long-range returns were not represented. A
planner could therefore repeat the same subject or attach a pronoun to the most
recent name without evidence.

## Acceptance criteria implemented

- Additive `discourse.v1` projection in `semantic_manifest.v2`; old manifests
  decode with an empty projection.
- Explicit names, surname aliases, pronouns and role nominals with exact source
  spans and sentence evidence.
- `resolved`, `ambiguous` and `unresolved` statuses; ambiguous references carry
  every candidate id and no target.
- Conservative rules: unique same-sentence/nearest/global antecedent, unique
  compatible role, two-person plural pair; quoted speech is never attributed.
- Entity threads with `starts`, `continues` and `returns` beats.
- Scene continuity based on deterministic entity-id overlap, with no invented
  topic labels.
- Tests for long-range returns, same-surname people, two-person ambiguity,
  role ambiguity, quoted speech, legacy decode, counts and continuity.

## Verification

`cargo test -p semantic-engine --test discourse` — 13/13.
`cargo test -p semantic-engine --lib` — 51/51.
The full workspace gate is recorded in `docs/tickets/README.md`.

## Explicitly outside this ticket

Subjectless-verb ellipsis, generated entity-centric prose, narrative-importance
scoring, visual asset reuse and calibrated editorial quality remain follow-on
work. They are not silently represented as implemented.
