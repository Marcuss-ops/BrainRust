# BrainRust acceptance checklist

Goal: pass the four release gates with evidence. The 100-script benchmark is a
regression suite, not a substitute for the independent editorial gold set.
Do not treat partial metrics as completion. Analytic checks only in this work;
no RenderingGen or Chronon runs in the original analytic scope. The user's
2026-10-27 workspace-wide ticket closeout expands that restriction for the
explicit full-workspace test below; editorial acceptance remains independent.

## Gate A — Stability

- [x] Rust formatting check (`cargo fmt --all -- --check`).
- [x] Clippy for `semantic-core` and `semantic-engine` with warnings denied.
- [x] Full workspace Clippy with warnings denied.
- [x] Full workspace tests: `RUST_ICU_MAJOR_VERSION_NUMBER=70 cargo test
  --workspace -j 2` completed on 2026-10-27 with 553 passed, zero failed and
  one existing ignored manual performance probe. The user's workspace-wide
  closeout explicitly expanded the former analytic-only scope; the three
  `canonical_render_segments` integration tests ran and passed.
- [x] **Measurement precondition documented and verified (2026-10-27).** The
  gate command is `RUST_ICU_MAJOR_VERSION_NUMBER=70
  VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models cargo test --workspace -j 2`.
  Without `VISUALNER_MODEL_DIR` the worker disables POS evidence and verb
  filtering, `every_mention_of_eight_people_survives_the_default_entity_budget`
  fails on VisualNER fallback output, and the corpora measure worse (short-gold
  entity recall 0.959 with German 0.600; long-gold objects 8/10; template
  corpus 63 PASS / 37 `missing visual object` review items). With the variable
  exported the same commands give 553 passed / 0 failed, recall 1.000, objects
  10/10 and 100 PASS.
- [x] **Formatting and lint gates repaired (2026-10-27).** `cargo fmt --all --
  --check` was red on `semantic-engine/src/candidates.rs` and `lib.rs`;
  `cargo clippy --workspace --all-targets -- -D warnings` failed with two
  `unnecessary_lazy_evaluations` errors in `consolidate.rs` and one
  `items_after_test_module` error in `consolidate/concepts.rs`. All are fixed
  in source (no lint suppression, no `#[allow]`), then re-verified: `fmt_rc=0`,
  `clippy_rc=0`.
- [x] Release build of `semantic-engine` against the host ICU major version.
- [x] Analytic tests for `semantic-core`, `semantic-engine` and `visualner`:
  139 + 128 + 100 tests passed on the current worktree, including installed
  UDPipe model checks; `mediasampler` + `ndjson-stdio` add 34 passes (401 total
  analytic tests). One existing manual performance probe is ignored.
- [x] Targeted Go projection tests for important segments, composed and
  contextual image queries, and one-batch scene analysis.
- [x] Multilingual synthetic candidate-quality gate rerun on 2026-10-27:
  100 PASS, zero NEEDS_REVIEW, zero structural FAIL and 100/100 identical
  cache replays. All ten German organization-compound cases and
  `en-07-flood` now pass. This closes the synthetic candidate regression,
  not independent editorial quality.
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

Current closeout evidence (2026-10-27, project-local, output-only):
`.tmp/ticket-closeout-analytic.log` (417 PASS, zero FAIL, one existing ignored
manual probe), `.tmp/ticket-closeout-workspace.log` (553 PASS, zero FAIL, the
same ignored probe), `.tmp/ticket-closeout-release.log` (current-source
`semantic-engine` release build already up to date), and
`.tmp/ticket-closeout-multilingual100.json` (100 PASS, zero review/failure,
100/100 identical replays). The corpus processed 203,254 words at 464.53
words/s, P50/P95 4.17/6.05 s and peak RSS 1,676,916 KiB while a C++ build was
running. These are host observations, not before/after speedup evidence.

Historical automated evidence (superseded for synthetic candidate status):
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
- [x] Synthetic candidate-quality acceptance: 100/100 PASS on the rebuilt,
  up-to-date release worker; all former German ORG and English compound-object
  misses pass. Independent multilingual gold remains open.
- [x] Explicit facility-location query smoke test passes in all ten languages.
- [x] Per-language gold precision/recall and critical errors documented for the
  full semantic annotation set that exists today:
  `semantic-engine/fixtures/gold_multilingual/reports/PER-LANGUAGE-GOLD-REPORT-2026-10-27.md`.
  Every language is at entity recall 1.000 with zero typed misses; the single
  precision loss is the Indonesian false positive `Bukan` typed LOCATION. The
  report names what is *not* covered (nine languages without long gold).
- [x] Inflection/compound/script/morphology review documented on gold and
  fixture evidence (compound German ORGs, Dutch compounds, inflected Slavic
  surnames, Indonesian one-word surnames, Romance elision): same report, §5.
  It does not infer language-wide quality from a smoke phrase, and it records
  the residual Italian compound limit (`Comune di Torino`, `Università di
  Torino` not formed as ORG).

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
- Automatically closed on 2026-10-27: full workspace test gate and all eleven
  former multilingual synthetic candidate misses, verified by the 100-case
  runner and cache replay. No engine or fixture exceptions were added in this
  closeout. Diagnostic independent long-gold entity/object/fact misses remain
  open.
- Needs independent editorial input: expand and annotate at least 20 long
  scripts across ten languages. The harness for that work is complete
  (`long_scripts/validate_long_gold.py` plus per-language reporting and the
  authority/certification fields); the corpus is at 1 script and the scorer
  reports `blocked_pending_release_size`. Summary faithfulness, complete
  sequences and full per-language precision/recall remain uncertified.
- Open engine gaps measured on 2026-10-27 (none may be closed by tuning a
  fixture): suffix-less company names typed PERSON (`Aurora Sport`, `Lumen
  Energia`); compound ORG with `di` + place not formed (`Comune di Torino`,
  `Università di Torino`); negated fact invariants not emitted (`fault =
  false`, `demonstrated = false`); Indonesian `Bukan` typed LOCATION. Evidence
  and ranking: PER-LANGUAGE-GOLD-REPORT-2026-10-27.md §6.
- Needs provider/deployment scope: independently judged precision@10,
  production provider cascade, 1,000 overlays/minute and render-consumer
  migration. No provider requests or deployments certify these tickets.
  The workspace video integration tests now pass, but they do not measure
  end-to-end production overlay throughput.

## Verification run — 2026-10-27 (all commands re-executed, models exported)

| Step | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | `fmt_rc=0` |
| Lints | `RUST_ICU_MAJOR_VERSION_NUMBER=70 cargo clippy --workspace --all-targets -- -D warnings` | `clippy_rc=0` |
| Workspace tests | `RUST_ICU_MAJOR_VERSION_NUMBER=70 VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models cargo test --workspace -j 2` | `tests_rc=0`, **553 passed, 0 failed** |
| Release worker | `cargo build --release -p semantic-engine` | `release_rc=0` |
| Short gold (10 languages) | `VISUALNER_MODEL_DIR=… python3 semantic-engine/fixtures/gold_multilingual/score_gold.py` | recall **1.000**, precision **0.991**, leaks 0, exit 0 |
| Cold replay | `… score_gold.py --check-reproducible` | PASS, 20 scripts × 2 cold runs byte-identical |
| Long gold (pilot) | `… long_scripts/score_long_gold.py` | exit 2, `blocked_pending_release_size`, entities 5/9, objects 10/10, facts 3/7 |
| Long-gold shape | `… long_scripts/validate_long_gold.py` | shape + language-scope + provenance OK |
| Template corpus | `… multilingual_100/benchmark.py --worker target/release/semantic-engine` | **100 PASS**, 0 problems, 431.6 s |
| Go projection | `go test ./internal/platform/media/rustexec/... -count=1` (in `refactored`) | 119 passed, 0 failed, 25 skipped (env-gated audio/mux/listen/render gates) |

Logs: `BrainRust/.tmp/closeout-verification-2026-10-27-final.log` (fmt/lint/test/
release) and the report files under
`semantic-engine/fixtures/*/reports/`. A first run of the same commands without
`VISUALNER_MODEL_DIR` is kept as `BrainRust/.tmp/closeout-verification-live.log`
and is the evidence for the degraded-mode numbers quoted in Gate A.

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
- Workspace verification superseded that historical limitation on 2026-10-27:
  `RUST_ICU_MAJOR_VERSION_NUMBER=70 cargo test --workspace -j 2` passed 553
  tests with zero failures and one existing ignored probe, including the
  `pipelinegen-muscles` video tests. Local output-only log:
  `.tmp/ticket-closeout-workspace.log`.

## Current language-query change

Facility heads and explicit location predicates are now loaded from the
facility lexicon across the ten corpus languages. Regression checks cover the
canonical entity span, asserted relation, contextual query grounding and
negative relation cases. The complete Rust workspace test gate now passes;
the independent long gold set and external utility gates remain outstanding,
so BrainRust is not complete.
