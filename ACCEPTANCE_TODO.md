# BrainRust acceptance checklist

Goal: pass the four release gates with evidence. The 100-script benchmark is a
regression suite, not a substitute for the independent editorial gold set.
Do not treat partial metrics as completion. Analytic checks only in this work;
no RenderingGen or Chronon runs.

## Gate A — Stability

- [x] Rust formatting check (`cargo fmt --all -- --check`).
- [x] Clippy for `semantic-core` and `semantic-engine` with warnings denied.
- [x] Full workspace Clippy with warnings denied.
- [ ] Full workspace tests (interrupted after discovering they include
  `canonical_render_segments` tests; three of those had already completed).
  Chronon was not run. Keep this outside the analytic-only test plan unless
  the scope changes.
- [x] Release build of `semantic-engine` against the host ICU major version.
- [x] Analytic tests for `semantic-core`, `semantic-engine` and `visualner`:
  138 + 114 + 98 tests passed, including installed UDPipe model checks.
- [x] Targeted Go projection tests for important segments, composed and
  contextual image queries, and one-batch scene analysis.
- [x] 100 multilingual fixtures: 100/100 PASS, 10/10 per language.
- [x] UTF-8/source grounding checks exercised by the 100-fixture runner.
- [x] Deterministic worker cache replay: 100/100 byte-identical manifests.
- [x] Gold subset cold-start replay: 20 scripts × 2 workers, byte-identical.
- [x] Retain all grounded important-segment windows for PipelineGen; the
  three-segment/non-overlap cap is removed and covered by an analytic test.
- [ ] Full manifest/Go projection audit, including all newly exposed semantic
  fields, on the current worktree.

Latest corpus report after retaining all segment windows:
`/tmp/brainrust_final_100_all_segments.json`. It processed 203,254 words at
428.02 words/s; request P50/P95 4.45/6.57 s; replay P50/P95 0.0968/0.1681 s;
peak RSS 1,811,888 KiB and VSZ 2,304,820 KiB. All 100 manifests passed and
100/100 cache replays were byte-identical. These measurements are from this
host, not universal performance limits.

## Gate B — Independent semantic quality

- [x] Start a separate long-form gold pilot: one independently authored
  584-word Italian script annotates entities, objects, complete
  phrases/segments, facts, chart controls, a four-step sequence, and a
  contextual image query. Pilot setup is complete; gold coverage is not.
- [ ] Grow that pilot to at least 20 long scripts across all ten languages;
  the scorer intentionally exits incomplete until then.
- [ ] Expand the independent gold from 20 short scripts (32–50 words) to at
  least 20 long scripts, distributed across all ten languages.
- [ ] Independently annotate named entities, important short/long phrases,
  complete multi-sentence segments, visual objects, dates, facts and relations.
- [ ] Annotate faithful micro-summaries and critical subject/negation/date
  invariants; score omissions and unsupported claims separately.
- [ ] Add positive and negative examples for image queries, charts, bullets,
  sequences and incomplete rankings.
- [ ] Report precision, recall, F1 and critical-error counts by language and
  overall; retain the Pareto target without fixture-specific engine rules.

Current short-gold report: `/tmp/brainrust_gold_current_after_semantic_fixes.json` — 98/98 annotated named
entities found, 112/112 extracted items matched, zero noise leaks. This is only
entity/noise coverage on short scripts; it does not satisfy this gate.

Long-form pilot report before the segment-retention change:
`/tmp/brainrust_long_gold_pilot_scored.json`; after retaining all segments:
`/tmp/brainrust_long_gold_pilot_all_segments.json`; after extractive
multi-fact summaries: `/tmp/brainrust_long_gold_pilot_micro.json`. On its 584-word Italian
case, named-entity F1 is 0.625 (5/9 found, 5/7 extracted correct); typed
visual-object recall is 4/10 with 126 object/concept candidates; 6/6 manually
marked important phrases are exact; the expected complete three-sentence
segment is now found (1/1 exact), among 102 complete candidate windows; 3/7
facts and 1/1 contextual image queries match; the comparison chart has the
right two values and PERCENT type. A source-grounded, verbatim micro-summary
now covers three co-evidenced facts; human review is still pending. The four
sequence items are not extracted as separate bullets. The single case is
diagnostic only and cannot certify language-wide metrics.

## Gate C — Multilingual coverage

- [x] 100 long synthetic fixtures pass: ten each in it, en, es, pt, fr, de,
  nl, pl, ru and id.
- [x] Explicit facility-location query smoke test passes in all ten languages.
- [ ] Document per-language gold precision/recall and critical errors for the
  full semantic annotation set.
- [ ] Review inflection, compounds, scripts and morphology using long gold
  examples; do not infer full language quality from one smoke phrase.

## Gate D — PipelineGen utility

- [x] Expose grounded contextual image queries only from asserted
  subject-location facts; preserve both source spans and sentence evidence.
- [x] Test positive, co-mention-negative, negated, and ten-language cases.
- [x] Fix repeated-entity selection so query components come from the sentence
  supporting the fact; unit-tested and confirmed by the 100-script replay.
- [ ] Measure image-query precision@10 against independently judged provider
  results; keep literal source queries distinct from derived queries.
- [ ] Gold-score chart validity, bullet usefulness/duplication, full sequences,
  and preservation of all important facts in the Go contract.
- [ ] Verify the 1,000 overlays/minute target with PipelineGen integration
  measurements after the semantic worker contract is certified.

## Current language-query change

Facility heads and explicit location predicates are now loaded from the
facility lexicon across the ten corpus languages. Regression checks cover the
canonical entity span, asserted relation, contextual query grounding and
negative relation cases. The complete Rust workspace and the independent long
gold set remain outstanding, so BrainRust is not complete.
