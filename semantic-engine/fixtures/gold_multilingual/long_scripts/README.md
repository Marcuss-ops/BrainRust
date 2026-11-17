# Independent long-form editorial gold

`gold.json` holds 20 long-form scripts over all ten corpus languages
(de, en, es, fr, id, it, nl, pl, pt, ru; ~5 700 words). Each script carries the
same ten annotation categories: named entities, typed and surface-only visual
objects, complete important phrases and multi-sentence segments, asserted and
negated facts, a percent comparison with a numeric negative control, an ordered
four-step sequence, a derived subject-place image query and micro-summary
expectations.

`gold.json` is separate from the 100 templated multilingual smoke corpus: that
corpus is a structural regression suite, this one is the editorial quality gate.

## Running the gate

```sh
cargo build --release -p semantic-engine
export VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models        # or ~/.local/share/visualner/models
python3 semantic-engine/fixtures/gold_multilingual/long_scripts/validate_long_gold.py
python3 semantic-engine/fixtures/gold_multilingual/long_scripts/score_long_gold.py
```

`VISUALNER_MODEL_DIR` is **required**: without the UDPipe models the POS
evidence and verb filtering are disabled, and measured recall changes
materially (the short gold dropped from 1.000 to 0.900 entity recall, with
German at 0.600; the pilot's visual objects dropped from 10/10 to 8/10). Any
measurement recorded without the models is not a BrainRust measurement. The
release worker also needs the documented ICU override on this host
(`RUST_ICU_MAJOR_VERSION_NUMBER=70`).

## What the scorer reports

Precision/recall for named entities, typed and surface-only visual objects,
exact important phrases and segments, asserted and negated facts,
contextual-image-query composition, chart values and chart type, exact
sequence-item extraction, source-grounding errors and cached replay identity.
Critical summary invariants and summary candidates are emitted for human
editorial review; the scorer does not claim to judge paraphrase faithfulness.

The report carries `overall` and `per_language` blocks. A category with no
annotation in a slice reports `recall: null` ("not measured") instead of a
fabricated 0.0, and `annotated_categories` lists what that language actually
scored.

## Provider precision@10 harness (TICKET-26)

`provider_precision.py` scores a **frozen** provider run; it never calls a
provider and never creates relevance labels. It takes two files:

- `--results`: `brainrust.provider_precision.results.v1`, with run metadata
  (`provider`, `cascade`, `started_at`, 40-character `git_sha`) and exactly 10
  unique HTTP results per query. Every result needs `rank`, `provider`,
  `provenance` and positive `downloaded_bytes`.
- `--judgments`: `brainrust.provider_precision.judgments.v1`, with an
  independent judge (`id`, `independent: true`, `judged_at`) and one boolean
  label for every result pair.

The scorer rejects relevance labels in the results file, provider failures,
missing/extra/duplicate judgments, ranks outside 1..10, duplicate URLs,
missing bytes/provenance and derived queries that do not match `gold.json`
exactly. It reports precision@10 separately for literal and derived queries.
A run is `measured` only when at least one literal query is present; otherwise
the status is `blocked_literal_queries_missing`. Every non-measured status
exits 2.

```sh
python3 semantic-engine/fixtures/gold_multilingual/long_scripts/provider_precision.py \
  --results /path/to/provider-results.json \
  --judgments /path/to/judge-labels.json \
  --report /tmp/provider-precision-report.json
python3 semantic-engine/fixtures/gold_multilingual/long_scripts/provider_precision_test.py
```

The unit fixture is synthetic: it proves the fail-closed validator and the
precision arithmetic, not provider quality. Real provider credentials and an
independent judge are still required for TICKET-26.

## Corpus shape, provenance and certification

`validate_long_gold.py` checks the shape the scorer reads plus the three things
the numbers alone cannot express:

- **Per-language category scope** from the shipped contract. The facility
  predicates exist only in `lexicon/facility_facts.json`, which carries Italian
  markers: a gold expecting them for another language would measure the gold,
  not the engine. The validator rejects that pair. Off-vocabulary `OBJECT`
  terms are reported as warnings (deliberate recall probes are allowed, but
  they must be visible).
- **Predicate contract**: the scored `facts` may only use
  `inaugurated_year`, `capacity`, `located_in`, `owned_by` — the four the
  implementation emits for a FACILITY subject (`semantic-engine/src/facts.rs`
  with the marker lists in `lexicon/facility_facts.json`). An expectation
  outside that set is not an engine miss: it belongs in the declared
  `capability_gap_expectations` block, which the validator requires to carry an
  `expectation` and a `reason` and which is never scored.
- **Annotation authority, per script**: `annotation_policy.authority` is the
  default, and a script may override it with `authority`. Only
  `human_annotator` can certify; any
  `assistant_draft_pending_human_review` script is listed in
  `draft_scripts` and forces `certification: blocked_pending_human_review`.
  Silence never promotes a draft.

The scorer exits 2 until the corpus has ≥20 scripts over the ten corpus
languages (it now does); certification stays the human step.

## Current provenance state (2026-10-27)

- `it_stadium_long_01` (the pilot, 584 words) inherits the policy authority
  (`human_annotator`). That field predates this session: the harness was built
  around a pilot described as hand-annotated, and the claim is **not
  re-verified here**. An annotator should confirm or re-label it.
- The other 19 scripts were written as drafts in this session and declare
  `assistant_draft_pending_human_review`. They are deliberately release-sized
  so the mechanical part of Gate B is done: an independent annotator now
  reviews/relabels 19 scripts and flips their `authority`, instead of writing
  them from scratch. The scorer will not call the corpus certified before that.

`it_stadium_long_01` declares four `capability_gap_expectations`: the two
`estimated_saving_percent` facts (42%/18%) and the two negated invariants
(`pompa.fault = false`, `risparmio annuale.demonstrated = false`). No shipped
predicate covers them — estimates are represented as typed chart items (the
pilot scores `charts 1/1`), and the negation mechanism is exercised by
`it_stadio_long_02`, which expects a contract-backed negated `located_in`.

Annotation corrections to score better must not happen: labels stay written
from the source. Two *scoreability* corrections are recorded. The first moved
two negated-fact subjects to their source forms (`pompa`, `risparmio
annuale`). The second re-expressed the four out-of-contract expectations above
as declared gaps: it removed them from the scored denominator without changing
any engine behaviour, because a label the contract cannot express measures the
label, not the engine.
