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
| 4 | Cross-scene chart aggregation | P1 | BLOCKED (contract designed in TICKET-53; no renderer/planner implementation) |
| 5 | Multi-series charts | P1 | BLOCKED (contract designed in TICKET-53; no certified layout) |
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
  be timed. The additive contract that makes it expressible (`charts` with
  `anchor_scene_id`, `scene_ids`, per-point `scene_id`, `scene_evidence`) is
  designed in `TICKET-53-cross-scene-multiseries-chart-contract.md`; the
  planner and renderer implementation is still missing.
* **5 — multi-series charts.** The certified chart layouts are single-series
  (`timeline_growth`, `bar_comparison`, `ranked_bars`, ...). No renderer
  template accepts two named series in one chart. `TICKET-53` designs the
  series identity, x-domain alignment and capability gate; no layout is
  certified yet.
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

### Subject resolution and the fallback coreference

`SubjectResolution` is `One(subject)` / `Absent` / `Ambiguous`. A sentence with
no nominal subject ("È alto 188 cm", "Weighs 80 kg") binds to the preceding
sentence's subject only when that sentence is in the same scene and its subject
is a single `PERSON`; anything else — no preceding sentence, a scene boundary, a
non-person subject, a coordination, or a second entity that is not the value
itself — stays `Absent`/`Ambiguous` and emits nothing. The pronoun or elided
form is recognised through `discourse::is_known_pronoun`, the same inventory the
chart lane uses, so there is one pronoun table and not two. The lookback is one
sentence, so two people mentioned in different sentences can never share a
value.

### Language coverage

Certification runs on 11 languages (it, en, de, fr, es, pt, nl, pl, ru, tr,
id). Nothing in the engine branches on a language: everything below is data and
the code path is the same for all of them.

| artifact | what a language adds |
|---|---|
| `lexicon/attribute_predicates.json` | the dimension lemmas (age/height/weight/profession) |
| `lexicon/units.json` + `lexicon/units/*.json` | unit surfaces and genitives, generated by `lexicon/tools/build_units.py` |
| `lexicon/roles/*.json` | role lemmas plus `attach_markers`, the particles that attach a role to its holder |
| `lexicon/relation_cues.json` | negation cues used by `fact_status` |
| `lexicon/intro_agenda.json` | spoiler cues |
| UDPipe model | tokenisation, lemmas and dependencies for that language |

Adding a twelfth language therefore means a UDPipe model that emits UD, a small
pack (the rows above) and a run of
`semantic-engine/tests/language_certification_52.rs`. A meta-test in that suite
asserts every certified language appears in every corpus, so a language cannot
be added to one case set and silently forgotten in another.

A role asserted in another case is pack data, not code: `lekarzem` (pl
instrumental) and `врачом` (ru instrumental) are surfaces in `roles/pl.json`
and `roles/ru.json`, and `an_instrumental_profession_binds_to_its_subject`
certifies them. Any further inflection is one surface plus one case.

Known limit, left fail-closed rather than approximated: the Turkish fused
copula. `ağırlığındadır` arrives as an unknown token (id `-1`) from the UDPipe
model, so a weight written that way is not bound. Nothing is guessed from the
surface form; a model that lemmatises it fixes the case.

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
catches a second name the NER pass typed as `VISUAL_CONCEPT` ("Eminem"). When
the grammatical subject is a metric noun owning a named `nmod` ("i ricavi di
Alfa"), the series follows the named owner, so an organisation chart still
forms across two sentences; a bare number parsed as `nsubj` ("ne ha prodotti
120.000") continues the chain instead of inventing a subject.

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

* `semantic-engine/tests/entity_attributes_52.rs` — 15 tests: card, two
  people, negation, approximation, distance≠height, unseen inflections,
  plural guard, production recipe/coverage, generic micro line, plus subject
  coreference (pronoun and elided forms in it/en/de/fr/es/pt/nl), ambiguous
  coreference, nominal subject, scene boundary and grounding.
* `semantic-engine/tests/language_certification_52.rs` — 8 tests: 44 attribute
  cases (11 languages × age/height/weight/profession), 11 negations, 10 inverse
  directions, the Turkish fail-closed case, 11 plural guards, 11 localised
  micro-summaries, the pl/ru instrumental profession and a meta-test that every
  certified language appears in every corpus.
* `semantic-engine/tests/chart_subject_binding_52.rs` — 5 tests: two subjects,
  elided subject, ambiguous coordination, a positive series in each of the 11
  languages and a no-cross-subject series in each of the 11 languages. The
  36-case chart acceptance corpus still passes, including the organisation
  metric series and the elided quantity case.
* `semantic-core` unit tests — intro agenda selection and no-padding rule;
  `micro` unit test — fact-backed metric summary.
* `overlay-compiler` recipe tests — lowering, one card per person, one-line
  no-op.
* Go `rustexec` tests — intro agenda grounding/count and micro-summary kinds;
  real-binary wire test and both Garlasco end-to-end tests pass against the
  rebuilt release binary.
* `internal/capabilities/scripts` full package (including the visual-recipe
  forwarding test) passes; `overlayplan` passes.

Verification commands (all green):

```
cargo test -p semantic-core --lib
VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models \
  cargo test -p semantic-engine -p overlay-compiler --lib --tests
cargo clippy -p semantic-core -p semantic-engine -p overlay-compiler \
  --all-targets -- -D warnings --no-deps
go test ./internal/platform/media/rustexec ./internal/capabilities/scripts/...
```

Not reachable here: renderer/template work for the new layouts, cross-scene and
multi-series charts, progressive reveal, editorial interest scoring and series
longer than six points. Each needs a certified layout/plan contract and an
annotated corpus that do not exist in the repository; they are listed under
Blocked rather than approximated.

## Performance

Measured with `semantic-engine/examples/perf_attributes.rs` (20 runs per build,
release, same machine, three runs of each tree). The stashed baseline is `main`
at `3502955`.

| tree | mean ms | p50 ms | p95 ms | max ms | RSS KiB |
|---|---|---|---|---|---|
| baseline `3502955` | 216.0 (211.9–220.3) | 215.3 (210.7–222.5) | 241.0 (238.8–241.6) | 242.0 | 1 551 894 |
| this tree | 204.5 (198.7–216.0) | 203.4 (192.9–219.6) | 227.5 (217.1–241.2) | 243.3 | 1 551 025 |

The ranges of the two trees overlap on every statistic, and the run-to-run
spread of one binary is larger than the gap between the trees, so the change is
cost-neutral within measurement noise; no improvement is claimed from a lucky
run and no regression is hidden. The lane is dominated by UDPipe inference, and
the added work is a single sentence of lookback over an already-parsed tree.

## Verification

All of these were run on the final tree, after the last data edit:

```
cargo fmt --all -- --check                      # clean
cargo clippy --workspace --all-targets -- -D warnings   # clean
VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models cargo test --workspace -j 2
                                                # exit 0, 59 test binaries,
                                                # no FAILED line
python3 lexicon/tools/build_units.py --check    # unit lexicon OK: 11 languages
python3 lexicon/tools/check_roles.py            # 40 codes in each of 11 languages
```

The three TICKET-52 suites inside that run: `entity_attributes_52` 15/15,
`language_certification_52` 8/8, `chart_subject_binding_52` 5/5.

Go, against the release worker rebuilt from this tree:
`go test -count=1 ./internal/capabilities/scripts/overlayplan ./internal/platform/media/rustexec ./internal/kernel/overlayplan`
— `overlayplan` and `rustexec` ok (`rustexec` 71 s; `-count=1` so the result
is not a cached run from before the rebuild), `kernel/overlayplan` has no test
files.
