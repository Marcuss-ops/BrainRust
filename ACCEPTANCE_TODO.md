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
  139 + 128 + 100 tests passed on the current worktree, including installed
  UDPipe model checks; `mediasampler` + `ndjson-stdio` add 34 passes (401 total
  analytic tests). One existing manual performance probe is ignored.
- [x] Targeted Go projection tests for important segments, composed and
  contextual image queries, and one-batch scene analysis.
- [ ] Multilingual candidate-quality gate on the current worktree: 89 PASS,
  11 NEEDS_REVIEW, 0 structural FAIL. Ten German cases miss the expected
  hyphenated ORG; `en-07-flood` misses `water-level sensor`. The previous
  100/100 claim is not the current result.
- [x] UTF-8/source grounding checks exercised by the 100-fixture runner.
- [x] Deterministic worker cache replay: 100/100 byte-identical manifests.
- [x] Gold subset cold-start replay: 20 scripts × 2 workers, byte-identical.
- [x] Retain all grounded important-segment windows for PipelineGen; the
  three-segment/non-overlap cap is removed and covered by an analytic test.
- [x] Full manifest/Go projection audit on the current worktree: preserve
  `story_graph`, `visual_coverage`, analyzer `lexicon`, group-member
  `normalized_date`, semantic bullet identity/evidence/provenance, editorial
  bullet sentence/timing and ranked-sentence centrality/novelty. Current Rust
  release worker → Go recursive wire-field check and all six analytic
  real-binary adapter/planner tests PASS. Corrupt graph/coverage fixtures
  fail closed. Missing additive whole-script views remain nil for older workers.

Current automated evidence (project-local, output-only):
`../.tmp/brainrust-audit-analytic-tests.log` (401 PASS, 1 existing ignored probe),
`../.tmp/brainrust-audit-repair.log` (query regression, Clippy and release build),
`../.tmp/brainrust-audit-multilingual100-final.json` (89 PASS, 11 NEEDS_REVIEW,
0 structural FAIL, 100/100 identical replays; 203,254 words, 446.76 words/s,
P50/P95 4.14/6.41 s, peak RSS 1,589,888 KiB, CPU 430.1 s). These timings
are observations from this host, not a before/after performance claim.
Short gold: recall 98/98, precision 112/113, zero noise leaks; one false
LOCATION (`Bukan`). Long gold remains one 584-word Italian script, scorer
exit 2: entity recall 5/9, typed object recall 4/10, fact recall 3/7,
phrase recall 6/6, contextual query 1/1, chart 1/1, zero grounding errors.
Neither report certifies long-form multilingual quality.

Historical corpus report after retaining all segment windows:
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

- [x] All 100 long synthetic fixtures are structurally grounded across it,
  en, es, pt, fr, de, nl, pl, ru and id, with 100/100 identical cache replays.
- [ ] Candidate-quality acceptance: 89/100 PASS; 10 German ORG cases and one
  English compound-object case still NEEDS_REVIEW (not release-certified).
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

## Remaining-ticket classification

- Automatically closed: current manifest/Go transport audit, analytic test
  baseline, release build, UTF-8 literal-query whitespace regression, cache
  replay (100/100) and independent cold replay (20 scripts × 2 workers).
  The corpus checker now verifies grounded co-mentions separately from
  asserted subject/target relations, with positive and negative unit tests.
- Automatically measurable but NOT closed: eleven multilingual candidate
  misses listed above and the diagnostic long-gold entity/object/fact misses.
  No fixture-specific names or rule exceptions were added to make them pass.
- Needs independent editorial input: expand and annotate at least 20 long
  scripts across ten languages; summary faithfulness, complete sequences,
  morphology and full per-language precision/recall remain uncertified.
- Needs provider/deployment scope: independently judged precision@10,
  production provider cascade, 1,000 overlays/minute and render-consumer
  migration. No provider requests, video renders or deployments certify these
  tickets in this analytic run. Full workspace video tests remain out of scope.

## Test layout regrouping (2026-10-26)

The oversized test files were split by topic and the two test conventions are
now uniform across crates (documented in `README.md`, "Test layout"):

- `semantic-engine/tests/semantic_engine.rs` (2,155 lines, 80 tests) became
  nine topic binaries plus `tests/common/mod.rs`. Known WIP test failures are
  unchanged by the move: `every_mention_of_eight_people...`,
  `filler_phrases_headings...` (entities_identity) and
  `entity_search_queries...`, `visual_query_suggestions...` (search_queries)
  fail identically against the pre-move `semantic_engine.rs` checked out from
  git; several search-query failures still evolve with the in-flight
  `src/candidates.rs` work.
- `visualner/src/extractor_tests.rs` became the `src/extractor_tests/`
  topic folder (57 tests, kept in `src/` because `tokenize` is
  `pub(crate)`). The three failing extractor/value tests fail identically
  against the pre-move single file from git.
- `semantic-core/src/phrase_impact_tests/summary_rank.rs` (954 lines)
  became `summary.rs` + `rank.rs` + `corpus_metrics.rs`; all 138
  `semantic-core` tests pass.
- Those failure descriptions are historical, not the current baseline. The
  analytic run on 2026-10-26 passed 366 tests across `semantic-core`,
  `semantic-engine` and `visualner`; one existing manual performance probe
  remains ignored. It includes the entity, query and multilingual regression
  suites named above. The local host requires the documented ICU override
  (`RUST_ICU_MAJOR_VERSION_NUMBER=70`); the release pin remains ICU 72.
- Full workspace tests remain outstanding: the analytic run does not execute
  `pipelinegen-muscles` video tests and must not be reported as workspace
  certification.

## Current language-query change

Facility heads and explicit location predicates are now loaded from the
facility lexicon across the ten corpus languages. Regression checks cover the
canonical entity span, asserted relation, contextual query grounding and
negative relation cases. The complete Rust workspace and the independent long
gold set remain outstanding, so BrainRust is not complete.
