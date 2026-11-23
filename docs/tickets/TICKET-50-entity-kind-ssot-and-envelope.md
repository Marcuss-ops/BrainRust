# TICKET 50 — Entity-kind SSOT, operation-true envelope, sample-rate SSOT

- Priority: P2
- Status: **DONE (2026-11-23)**
- Source: [`docs/reviews/AUDIT-2026-11-23-architettura.md`](../reviews/AUDIT-2026-11-23-architettura.md),
  findings F1–F4 plus the envelope and metric notes
- Files:
  - `semantic-core/src/entity_capabilities.rs`
  - `overlay-compiler/src/editorial_plan_v2/projection.rs`
  - `overlay-compiler/src/editorial_plan_v2/planner/media.rs`
  - `overlay-compiler/src/lib.rs`, `overlay-compiler/src/lib_tests.rs`
  - `overlay-compiler/src/editorial_plan_v2/planner/tests/core.rs`
  - `pipelinegen-muscles/src/protocol.rs`, `artifact.rs`, `dispatcher.rs`
  - `pipelinegen-muscles/src/{cut,probe,transform,transform_operations,transform_manifest,transform_assemble,transform_merge,transform_mux,render_stock,render_stock_canonical_executor,render_audio_execution,render_audio_probe}.rs`
  - `README.md`

## Problem

TICKET-46 gave the workspace one `EntityCapabilities` table and named the
failure mode it was fixing: a kind known to four copies but not the fifth is
extracted and never pictured. `overlay-compiler` was the fifth: it kept its own
`fold_entity_type` and `entity_card_kind`, and in that copy `MEDIA_OUTLET` was
unknown. `identity.rs` prefixes an outlet's canonical id with `org` and
`entity_capabilities` marks it a named subject, so the same type was an
organization for identity and an unknown type for the planner: no card, no
verified-asset binding, no recovered image, organization ranking weight of 0.5.

Two smaller SSOT breaches had also drifted:

- `config::CANONICAL_SAMPLE_RATE` existed, but the ffmpeg graph and argv still
  contained six literal `48000`/`"48000"` sites, so a change to the constant
  would make the validator approve a plan the encoder did not follow.
- `artifact::failed_response` hardcoded `operation: "cut_batch"` for all 113
  callers; only the dispatcher rewrote the field afterwards, an implicit
  invariant no other entry point honoured.

## What landed

### WS1 — one kind fold, `MEDIA_OUTLET` included

- `semantic_core::entity_capabilities::{EntityKind, fold_entity_kind}` is now
  the only spelling→family table. It carries every alias the planner used to
  keep locally (`GPE`/`CITY`/`FACILITY`, `COMPANY`/`AGENCY`/`UNIVERSITY`,
  `PERCENTAGE`, `CALENDAR_DATE`, …) plus `MEDIA_OUTLET` → `Organization`, and
  the canonical `PERSON`/`ORG`/`PLACE` spellings the old pass-through relied on.
- `overlay-compiler` deleted `fold_entity_type` and routes the projection
  annotation lane, `entity_card_kind`, the verified-intent image gate and
  `entity_type_weight` through the shared fold. A test pins that an outlet
  ranks exactly like an organization (`0.9`).
- The lexicon coverage test asserts every type declared in
  `lexicon/entity_type_terms.json` either folds to a family or is an explicit
  additive visual type: a new extractor spelling cannot be silently unknown.
- Regression tests on the three real lanes: an outlet produces an
  `organization` card (`org_default`), binds its verified asset as an
  `entity_image`, and is recovered when admission dropped it.

### WS2 — the failure envelope names its own operation

- `Response::failure(operation, source, error)`, `Response::success(...)` and
  `with_preflight(...)` own the seven-field envelope. All 112 failure call
  sites were converted; `failed_response` and its `cut_batch` literal are gone.
- The shared helpers now take the typed operation: `checked_multi_inputs`,
  `finish_run` and `projection_inputs`, so no module can report another
  operation's name. The dispatcher patch was deleted.
- `cut_batch` keeps its per-job `items` on partial failure through an explicit
  literal whose operation still comes from `Operation::CutBatch`.
- A regression test fails if a failed `assemble_copy`/`merge_inputs`/
  `mux_audio_copy` reports anything but its own name.

### WS3 — 48 kHz has one source

- `source_normalize` builds its filter once from `CANONICAL_SAMPLE_RATE`
  (`OnceLock<String>`, no per-call allocation), the silence bed and both
  encoder/stem argv entries use the constant, `remove_silence` derives `-ar`,
  and the probe compares a parsed `i64` against the constant instead of the
  string `"48000"`. The mux error message derives its rate too.
- Test literals stay explicit on purpose: they are the expected wire values.

### WS4 — metrics and boundary documentation

- `render_stock_canonical_executor` documents that `encode_ms` mirrors
  `ffmpeg_ms` because there is one ffmpeg process, not two phases. No wire
  change.
- `README.md` states that `semantic-engine` is the unified worker that also
  runs the five planner phases inside `compile_editorial_plan` (TICKET-14). The
  boundary to preserve is "planner decides, Go materializes bytes", not
  "engine never plans".

## Acceptance evidence

Run on 2026-11-23 with the UDPipe models at
`$HOME/.local/share/visualner/models` and `ORT_DYLIB_PATH` pointing at the
versioned `libonnxruntime.so.1.23.2` (the unversioned name is not installed on
this host; without it the GLiNER worker cannot load and 22 semantic-engine
tests fail for an environmental reason).

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | rc 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | rc 0 |
| `cargo test -p semantic-core` | 259 lib + 4 integration pass |
| `cargo test -p overlay-compiler` | 172 lib + 33 frozen contract pass (goldens byte-identical) |
| `cargo test -p pipelinegen-muscles` | 152 lib + 12 integration pass |
| `cargo test -p semantic-engine --lib` | 175 pass |
| `cargo test --workspace -j 2` | all suites pass except `--test consolidation` `consolidation_types_vigevano_as_location_and_ris_as_organization`, which fails identically on the unmodified tree (GLiNER types `Vigevano` as `PERSON`); pre-existing and environmental, not caused by this ticket |

## Residual

- `select_phrases` (cognitive complexity 38) and `noun_phrase_candidates` (35)
  were not split: they need a corpus parity run, which this ticket explicitly
  does not have. They stay the top entries of the audit's hotspot matrix.
- Unknown entity types are routed to `EntityKind::Other` (fail-closed for the
  lanes) rather than rejected in `validate_compilable`: rejecting them would
  break the additive TICKET-43 types that are legitimately picture-only.
- `metadata.encode_ms` remains a mirror until a real per-stage measurement
  exists; the code now says so.
