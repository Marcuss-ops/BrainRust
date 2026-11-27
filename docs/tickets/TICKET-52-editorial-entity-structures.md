# TICKET-52 — Editorial entity structures (2026-11-27)

Source: external editorial audit of `main` at `ec9d2b7`. The audit found the
chart lane mature but three structural gaps that limit editorial intelligence:

1. quantities are extracted but not bound to the person as attributes;
2. a cross-sentence chart series never checks that every value has the same
   subject;
3. micro-summaries exist only when a legal domain pack exists.

The audit's 14-item roadmap is reproduced here with a status per item. Only
items whose acceptance can be met inside this repository are implemented; the
rest are `BLOCKED` with the missing external artifact named. No item is marked
DONE for a render or a human annotation that was not produced here.

## Status matrix (2026-11-27)

| # | Item | Priority | Status |
|---|---|---|---|
| 1 | Entity attribute extraction (age, height, weight, profession bound to the subject) | P0 | DONE |
| 2 | `entity_attribute_list` grouped card | P0 | DONE |
| 3 | Chart subject binding (never merge two subjects into one series) | P0 | DONE |
| 4 | Cross-scene chart aggregation | P1 | BLOCKED (no cross-scene chart placement in the plan contract) |
| 5 | Multi-series charts | P1 | BLOCKED (no certified multi-series layout) |
| 6 | Progressive attribute reveal | P1 | BLOCKED (word-level attribute timing does not exist yet) |
| 7 | Generic (non-legal) micro-summary | P1 | DONE |
| 8 | Intro agenda | P1 | DONE |
| 9 | Spoiler-aware selection | P1 | DONE (shared with 8) |
| 10 | Editorial interest scoring | P1 | BLOCKED (needs an annotated interest corpus; weights would be invented) |
| 11 | Visual recipe selection for structured content | P1 | DONE (attribute card precedence) |
| 12 | Extended chart series (>6 points) | P2 | BLOCKED (renderer/planner certified 6 bars; see below) |
| 13 | Semantic overlay deduplication | P2 | DONE (attribute card vs fact card vs KPI vs role) |
| 14 | End-to-end visual certification | P2 | PARTIAL (manifest→consolidation→planner covered by tests; rendering not run) |

### Why items 4, 5, 6, 10, 12 are blocked

* **4 — cross-scene charts.** A `ProductionCandidate` is anchored to one scene
  and to one verbatim evidence span. A chart that aggregates values from
  several scenes has no single legal `scene_id`/`start_byte` and no placement
  rule in `editorial_plan_v2`. Inventing one would produce a card that cannot
  be timed. Reopening requires a plan-contract field for multi-scene anchors.
* **5 — multi-series charts.** The certified chart layouts are single-series
  (`timeline_growth`, `bar_comparison`, `ranked_bars`, ...). No renderer
  template accepts two named series in one chart.
* **6 — progressive attribute reveal.** The semantic manifest has no word
  timing; timing lives in PipelineGen. A per-attribute reveal needs the
  planner to receive attribute windows, which item 1 deliberately does not
  invent.
* **10 — editorial interest scoring.** The proposed weights are a hypothesis
  with no annotated corpus. Adding them would be an unmeasured change, which
  the repository's "measured" invariant forbids.
* **12 — extended series.** `maxSemanticChartValues = 6` in PipelineGen and the
  chart layouts certify six readable bars. Raising the producer limit alone
  would emit values the planner truncates, so the capability is not reachable
  at the producer layer.

## Item 1 — Entity attribute extraction

Implemented by `semantic-engine/src/entity_fact_resolver.rs` (a generic
UD subject–predicate–object resolver) and
`semantic-engine/src/attributes.rs` (projection), with the canonical ontology in
`lexicon/attribute_predicates.json`.

An attribute is emitted only when all of these hold:

* the value is a grounded numeric `Quantity` (`length`/`mass`), a grounded
  `PersonAgeFact`, or an asserted `PersonRole`;
* the UDPipe `head`/`dependency` path connects the quantity to a predicate
  lemma declared for that dimension in the ontology, and that predicate's
  subtree names exactly one subject (local owner, `nsubj`, relative-clause
  head, unique subtree, or unique sentence);
* a plural or coordinated subject is rejected rather than split;
* the sentence is not negated or uncertain (`fact_status`), so
  `Mario non pesa 80 kg` never becomes a positive weight;
* the value span is exact and the evidence is the verbatim sentence.

The same projection emits `EntityFact`s with predicates `height`, `weight`,
`profession` (and reuses the existing `personal_data` age fact) so
micro-summaries and coverage reference one fact inventory. No per-language
extractor, regex table or manual synonym dictionary was added: only localized
lemmas, labels and units are data.

Acceptance:

- `Andrea Semprio ha 45 anni, è alto 188 centimetri e pesa 789 chilogrammi.`
  yields one list with `age=45 year`, `height=188 cm`, `weight=789 kg`.
- `Mario è alto 188 cm, Luca 175 cm.` yields two lists, one per person, with
  no cross-binding.
- `Mario non pesa 80 kg.` yields no positive attribute and no asserted fact.
- `Mario pesa circa 80 kg.` keeps the `approximately` qualifier.

## Item 2 — `entity_attribute_list`

`consolidate/recipes.rs` emits one `ProductionCandidate` per
(scene, identity) whose list has at least two attributes, then suppresses the
individual `fact_card`, `counter_odometer` and `role_lower_third` for the same
claim. Payload is `visual_recipe.v1`:

```json
{
  "schema": "visual_recipe.v1",
  "recipe": "entity_attribute_list",
  "entity_id": "person:andrea_semprio",
  "entity_label": "Andrea Semprio",
  "evidence": "…verbatim sentence…",
  "lines": ["Età 45 anni", "Altezza 188 cm", "Peso 789 kg"],
  "attributes": [
    {"predicate": "age", "value": "45", "unit": "year", "qualifier": "exact",
     "status": "asserted", "start_byte": 18, "end_byte": 20, "evidence": "…"}
  ]
}
```

The payload carries the fact, never a window. Coverage ids are canonical
`cf_` ids; an attribute fact that has no canonical counterpart contributes no
coverage rather than failing the consumer's grounding check.

## Item 3 — Chart subject binding

`groups/charts/series.rs` compares gap, metric, unit, time anchor and subject.
A sentence's subject is the unique `PERSON` entity when present, otherwise the
UDPipe `nsubj`, otherwise a capitalized-lemma named subject. Different canonical
topic surfaces stop the series; an unnamed (pronoun/elided) subject continues
it; a coordination or two named subjects are ambiguous and stop it. This
catches a second name the NER pass typed as `VISUAL_CONCEPT` ("Eminem").

Acceptance:

- `Rick Ross nel 2011 ha venduto 25.000 copie. Eminem nel 2014 ne ha vendute
  34.999.` produces no cross-subject series.
- `Nel 2015 Rick Ross ha venduto 1500 dischi. Nel 2016 ne ha venduti 1200.`
  still produces one series: the second sentence has no competing person.

## Item 7 — Generic micro-summary

`micro.rs` no longer requires a legal pack. It emits `attribute_summary` (one
compact line per person with at least two attributes, labels from
`lexicon/attribute_predicates.json`) and `metric_summary` (an existing
`EntityFact` that already carries a unit and a normalized date). Both keep
`evidence_fact_ids`, verbatim evidence and the subject id. A legal `micro`
template that would repeat an attribute fact is skipped. Go's
`validMicroSummaryKind` accepts both new kinds.

## Item 8/9 — Intro agenda

`ResultDocument`/`EditorialManifest.intro_agenda` carries 3–5 topics selected
from the whole-script ranking. A topic is dropped when its source sentence
asserts a conclusion (verdict, death, winner, final score, ending) using the
`spoiler_cues` table in `lexicon/intro_agenda.json` (11 languages, whole-token
matching), unless the sentence is already in the first 10% of the script.
Near-duplicate topics are skipped. When fewer than three non-spoiler topics
remain, the field is absent rather than padded. Go's rustexec boundary
re-grounds every span and rejects a count outside 3–5.

## Item 11/13 — Recipe lowering and deduplication

`entity_attribute_list` is in the closed validation set and lowers to the
certified `LOWER_THIRD` template: name on the first line, ontology-labeled
attributes joined on the second, timed by the person's entity occurrence. The
planner refuses a second lower third for the same person (role card wins if it
came first). Go forwards the recipe (`overlayVisualRecipes`).

When an `entity_attribute_list` represents a subject's attributes:

* the individual attribute facts do not also become `fact_card`s (the age fact
  is reused, so there is one canonical age fact, not two);
* a `counter_odometer` whose quantity span is already in the card is skipped;
* a `role_lower_third` for a role already in the card is skipped.

## Evidence

* `semantic-engine/tests/entity_attributes_52.rs` — 10 tests: card, two
  people, negation, approximation, distance≠height, unseen inflections,
  plural guard, 8 languages, production recipe/coverage, generic micro line.
* `semantic-engine/tests/chart_subject_binding_52.rs` — 3 tests: two subjects,
  elided subject, ambiguous coordination.
* `semantic-core` unit tests — intro agenda selection and no-padding rule;
  `micro` unit test — fact-backed metric summary.
* `overlay-compiler` recipe tests — lowering, one card per person, one-line
  no-op.
* Go `rustexec` tests — intro agenda grounding/count and micro-summary kinds.

Known verification gap: `internal/capabilities/scripts` test package was
mid-refactor by another session (`undefined: clips` in unrelated test files)
when this ticket closed; the recipe allowlist change was verified through
`internal/capabilities/scripts/overlayplan` and the Rust planner tests instead.

## Verification

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models cargo test --workspace -j 2
```

Go: `go test ./internal/capabilities/scripts/overlayplan ./internal/platform/media/rustexec ./internal/kernel/overlayplan`.
