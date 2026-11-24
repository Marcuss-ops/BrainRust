# BR-HOT-01 — Eight complexity hotspots (CC 40–65, 114–311 LOC)

- **Status:** OPEN — not attempted in this wave. The eight functions are
  behavior-heavy (114–311 LOC); splitting them without an equivalence suite
  would be a silent semantic change. Next step: add golden tests per function,
  then extract one branch at a time. The shared `target/` is also held by a
  concurrent workstream, so a full test cycle is not available right now.
- **Priority:** P1
- **Owner:** `semantic-core`, `pipelinegen-muscles`, `visualner`, `semantic-engine`
- **Evidence date:** 2026-11-24

## Measured hotspots

| File / function | CC | LOC | Problem |
| --- | ---: | ---: | --- |
| `semantic-core/src/phrase_impact.rs:432` `run_with_chapter_options` | 62 | 311 | legacy extractive and editorial.v1 branches mixed |
| `pipelinegen-muscles/src/render_audio_plan.rs:90` `validate_plan` | 65 | 114 | highest CC in the workspace |
| `visualner/src/extractor/values/classify.rs:5` `classify_type` | 59 | 221 | long condition cascade |
| `semantic-engine/src/events/proximity.rs:76` `nearest_candidate` | 56 | 140 | proximity/scoring branching |
| `visualner/src/spellout.rs:212` `candidates` | 53 | 243 | monolithic scanner |
| `pipelinegen-muscles/src/transform_manifest.rs:40` `execute_projection` | 49 | 178 | two large functions in one file |
| `pipelinegen-muscles/src/transform_manifest.rs:222` `execute_window` | 50 | 169 | same file |
| `semantic-engine/src/facts.rs:251` `extract_entity_facts` | 40 | 297 | one large block |

Also measured: `semantic-engine/src/lib.rs` is 616 LOC and the most-churned
file (38 commits in the last 150); `visualner/src/extractor/names.rs` (672 LOC)
and `visualner/src/extractor/pipeline/names.rs` (659 LOC) are two large files
with the same name in the same subsystem.

## Addendum — 2026-11-23 audit

The 2026-11-23 architectural audit measured three more production functions
above the default cognitive-complexity threshold of 25:

| File / function | CC | LOC | Note |
| --- | ---: | ---: | --- |
| `semantic-core/src/phrase_select.rs:89` `select_phrases` | 38 | 232 | also in the long-function list |
| `visualner/src/extractor/phrases.rs:249` `noun_phrase_candidates` | 35 | 276 | also in the long-function list |
| `visualner/src/extractor/values.rs:522` `value_candidates` | 29 | — | tabular; do not split without a corpus |

The audit's own plan is explicit: extract `select_phrases` and
`noun_phrase_candidates` only with a parity run on the existing corpus;
`value_candidates` and `classify_type` are tabular and must not be split
without a corpus. They are tracked here rather than in a duplicate ticket.

## Acceptance criteria

- [ ] Each function above is split so every resulting function is **≤40 CC**
      and ≤100 LOC, or the ticket records why a function must stay above the
      threshold (declarative table, not logic).
- [ ] No behavior change: existing golden/contract tests stay byte-identical.
- [ ] Each extraction has a focused test for the extracted phase.
- [ ] `semantic-engine/src/lib.rs` is reduced by extracting orchestration
      phases, not by moving unrelated code.
- [ ] The two `names.rs` files are renamed or one is removed after checking
      their responsibilities; no same-name duplicate remains without a comment.
- [ ] `cargo test --workspace --locked` and `cargo clippy --workspace
      --all-targets --locked -- -D warnings` are green.

## Checks

```sh
cd BrainRust
cargo test -p semantic-core -p pipelinegen-muscles -p visualner -p semantic-engine --locked
cargo clippy -p semantic-core -p pipelinegen-muscles -p visualner -p semantic-engine \
  --all-targets --locked -- -D warnings
```

## Out of scope

- Rewriting the algorithms; the ticket preserves the decisions and only
  extracts named phases.

## Progresso 2026-11-24

- `pipelinegen-muscles/src/render_audio_plan.rs::validate_plan` (CC 65, 114 LOC)
  è stato diviso in sei fasi nominate: `validate_plan_version`,
  `validate_canonical_profile`, `validate_tracks`, `validate_primary_events`,
  `validate_layers`, `validate_automation`. Messaggi e ordine dei controlli
  invariati; `cargo test -p pipelinegen-muscles --locked`, clippy `-D warnings`
  e `rustfmt --check` verdi.
- `semantic-core/src/phrase_impact.rs::run_with_chapter_options` (CC 62, 311
  LOC) è ora un orchestratore di ~70 righe; le fasi sono
  `validate_chapter_options`, `validate_request_header`,
  `validate_segment_alignment`, `resolve_scene_context`, `build_vectors`,
  `rank_sentences` e `build_bullets`. I 66 test Phrase Impact passano,
  clippy `-D warnings` e rustfmt sono verdi. La scheda resta PARTIAL finché gli
  altri hotspot non sono chiusi.
