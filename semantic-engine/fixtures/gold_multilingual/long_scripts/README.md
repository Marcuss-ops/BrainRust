# Independent long-form editorial gold

`gold.json` holds the hand-annotated pilot (`it_stadium_long_01`, 584 words,
Italian). Its annotations were written from the script before the release
worker was run. The script covers named and visual entities, complete phrases
and segments, asserted/negated facts, a percent comparison with a numeric
negative control, an ordered four-step sequence, and a derived subject-place
image query.

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

## Corpus shape, provenance and certification

`validate_long_gold.py` checks the shape the scorer reads plus two things the
numbers alone cannot express:

- **Per-language category scope** from the shipped contract. The facility
  predicates (`inaugurated_year`, `located_in`, `capacity`,
  `estimated_saving_percent`) exist only in `lexicon/facility_facts.json`,
  which carries Italian markers: a gold expecting them for another language
  would measure the gold, not the engine. The validator rejects that pair.
  Off-vocabulary `OBJECT` terms are reported as warnings (deliberate recall
  probes are allowed, but they must be visible).
- **Annotation authority**: `annotation_policy.authority` is either
  `human_annotator` or `assistant_draft_pending_human_review`. The report
  states the matching `certification` value — `certified`,
  `blocked_pending_human_review` or `blocked_pending_release_size`. Only
  human-authored labels over a release-sized corpus (≥20 scripts, all ten
  languages) can certify the gate; drafts must never be promoted silently.

Current status: **one script, one language, `blocked_pending_release_size`.**
The scorer exits 2 until the corpus has ≥20 scripts over the ten corpus
languages. Nineteen long scripts still need an independent annotator; this is
the last human-only step of Gate B, and the harness above is what turns it into
a mechanical fill-in-and-run task.

Annotation corrections to score better must not happen: labels stay written
from the source. The one correction recorded so far is a *scoreability* fix —
two negated-fact subjects used English glosses (`pump`, `annual_saving`) in an
Italian script, where the scorer requires the subject surface to appear in the
engine's evidence sentence; they now read the source forms `pompa` and
`risparmio annuale`. The metric did not move (3/7 before and after), which
confirms those two facts are genuine engine misses rather than label artefacts.
