# TICKET 13 — Go-side demolition closeout (P2–P8)

- Priority: P1
- Status: PARTIAL
- Source: 2026-11-15 workspace audit ("andiamola a completare e tagliare per
  davvero") + Go→Rust migration analysis
- Owner: any session working in `refactored` (PipelineGen)
- Companion crates: `semantic-core`, `semantic-engine`, `imagecatalog`,
  `overlay-compiler` (all in this workspace)

## Why this ticket exists

Tickets 01–12 close the BrainRust core: the semantic engine owns analysis,
entity linking, editorial extraction, map-window timing and ranking. The
consumer (`refactored`, Go) must stop re-implementing those decisions. Every
line of Go logic that *changes* an editorial outcome instead of *projecting*
one is a second authority on the same decision — two code paths that can drift
silently, two test suites that can both stay green while the product diverges.

The target architecture is already demonstrated inside the codebase; this
ticket generalizes it. Two Go modules show the desired shape:

- `internal/platform/media/rustexec/semantic_engine_projection.go` (505
  lines): pure functions that validate worker output and copy it into typed
  Go structs. No decisions, no language rules, no thresholds. Fail-closed via
  validation errors.
- `internal/platform/media/rustexec/semantic_engine_overlay_timing.go` (154
  lines): delegates a whole decision (`resolve_map_overlaps`,
  `trim_map_windows_before_budget`) to the worker, then applies **only** the
  returned timing edits while preserving every typed and opaque Go field.

Everything in the inventory below violates one of these two shapes.

## Context: what is already closed (this audit, 2026-11-15)

| Commit | Repo | Scope |
| --- | --- | --- |
| `2a557d432` | refactored | semantic entity linking + local scoring wired (P0) — pushed |
| `03f21d132` | refactored | `render_clip` retired from the mediaexec protocol — pushed |
| `fc39e03e8` | refactored | Go-only overlay compilation path deleted, resolver fail-closed mandatory (P1) — pushed |
| `8d7495200` | refactored | `watermark` retired (kernel types, alias, transport, tests) — pushed |
| `60e69c8` | BrainRust | `dist-icu72` binaries untracked (release-only distribution) — pushed |
| `578ed4f` | BrainRust | mediaexec.v2 schema retires `watermark`/`render_clip`/`admin_render`; README cleanup — pushed |
| `8bc9ad456` | refactored | final-audio 40 ms tolerance owned by `audio.FinalAudioDurationTolerance*` (13c) — pushed |
| `0868387d4` | refactored | local MP3 duration parser and tests deleted; shared executor probe is the only duration authority (13c) — pushed |
| `eb4bf234b` | refactored | entitycatalog fail-closed on retired/unrecognized states — pushed |
| `b6f5379` | BrainRust | 13a rules 2–4: one editorial entity decision (repair, prompt echo, image admission) — local |
| `c4c12ac98` | refactored | 13a rules 2–4 projected through annotations and localization — local |
| `8b4482660` | refactored | 13b verified asset registry owned by `entitycatalog` — local |
| `ded9f28` | BrainRust | 13d `compile_editorial_plan` batch operation — local |
| `68ddb83f5` / `652c03217` | refactored | 13d Go batch adapter; all overlay ports route through it — local |

Push gate: `make verify-main` in `refactored` must pass once before pushing
(AGENTS.md rule). Do not push while another session has uncommitted edits in
the tree.

## Current IPC surface (measured 2026-11-15)

Semantic-plane operations Go sends today (distinct call sites in
`internal/platform/media/rustexec/`):

- `analyze_script` — 2 production call sites (`semantic_engine.go:71`, one
  per query class), plus `select`, `score_local`, `extract`, `analyze`
  one-shot helpers in adapters (`select_group`, `local_scorer`,
  `media_sampler`)
- Overlay timing ops via `mapTiming` (one worker round-trip each):
  `resolve_map_overlaps`, `trim_map_windows_before_budget`,
  `rank_entity_overlays`, `cascade_clustered_entrances`
- `link_entities` path via `semantic_engine_entity_linking.go` (adapter, 136
  lines)

The end state for the scripts hot path (item 13d) is: **one** `analyze_script`
call for the manifest, **one** `compile_editorial_plan` call for the overlay
plan, mediaexec ops for rendering. Every intermediate decision disappears
into the two calls.

## Demolition inventory (4,424 decision lines in `scripts/`, tests excluded)

### 13a — entity annotations: Go re-decides what Rust already decided

**Status 2026-11-15: DONE.** Rule 1 landed locally in `7fd7b718b`.
BrainRust's `named_entity`/`visual_concept` role is carried through
`VisualEntity`, `ExtractedEntity` and `AnnotatedEntity`, and both annotation
projections use it for the primary/secondary split. Rules 2–4 landed in
BrainRust `b6f5379` and refactored `c4c12ac98`: the linker emits
`editorial_surface`, `editorial_type`, `prompt_echo` and `visual_admission`
from one pure decision, and Go no longer runs `normalizeEditorialEntity`,
`isPromptEchoEntityName` or the acronym guard. Every v3 entity carries that
decision through the durable annotation and the localized merge; pre-v3
fixtures retain only the legacy kind fallback.

`internal/capabilities/scripts/entity_annotations.go` — 598 lines. Violations,
in order of severity:

1. **Primary/secondary classification** (`projectEntityAnnotations`,
   PERSON/ORG/GPE → primary, everything else → secondary) — duplicates the
   Rust linker's role assignment that `semantic_manifest.v3` already carries.
   The Go comment says it "mirrors the batch path's sceneAnnotations
   classification": an admitted mirror is a drift risk.
2. **NER repair** (`normalizeEditorialEntity`, line 143) — repairs recurring
   NER errors in Go ("these phrases describe concepts, while the generic
   Italian verbs/adverbs are not place names"). Language-model-adjacent
   judgment embedded in Go. Must move behind the entity linker; the linker
   must return the corrected value or no entity, never leave Go to fix it.
3. **Prompt-echo rejection** (`isPromptEchoEntityName`, line 166) — explicit
   denylist of instruction-like entity strings ("Overlay Check", "Mappa
   Pertinente …"). Denylists are editorial policy; they belong next to the
   entity linker in Rust where the same contamination actually happens.
4. **Image binding policy** (`entityImageBindingFor`, line 183) — decides
   which annotation may reserve a visual slot. This is admission policy:
   `verified-hit admission` is already a worker-owned rule (ticket 12);
   Go must not re-adjudicate it.

Companions in the same violation class:

- `entity_annotations_numeric.go` (91 lines): numeric-threshold projection of
  the same classification.
- `entity_projection.go` (457 lines): scene/entity projection with grounding
  checks; grounding (verbatim occurrence) is fine in Go as validation, but
  any re-classification or re-ranking here is a violation.

**Migration mechanics.** Move rules 1–4 into `semantic-core` (rules, pure) and
wire them through `semantic-engine` so `analyze_script`/`link_entities`
responses carry the final classification, the repaired value, the
prompt-echo rejection flag and the visual-admission decision. The Go side
keeps only: validate (entity occurs verbatim in scene text, offsets legal)
→ project → fail closed with a typed error when the manifest lacks the field.
No silent skip: a missing classification is an error, not "treat as
secondary".

### 13b — overlay entity cards + asset registry — REGISTRY DONE (2026-11-15, `8b4482660`)

- `overlay_entity_cards.go` (590) + `overlay_entity_cards_text.go` (232):
  card composition (which annotations become cards, text shaping, fallback
  cards). Card *rendering* stays in Chronon; card *selection and copy
  decisions* duplicate the editorial extractor.
- `overlay_entity_asset_registry.go` (403): re-implements the asset chain
  (registry → resolve → reserve) that the `imagecatalog` crate owns (ticket
  02: snapshot integrity, real dates, `evaluated_at`, fail-closed policy).

**Registry result (2026-11-15).** The live owner is the Go
`internal/capabilities/images/entitycatalog` package, not the Rust
`imagecatalog` crate: `BrainRust/imagecatalog/README.md` and
`refactored/docs/operations/entity-image-pipeline-flow.md` both declare the
crate experimental and not wired, and the canonical Go flow is the only live
path. Moving this registry into the offline crate would have created a second
planner. The verified local registry loader/verifier now lives in
`internal/capabilities/images/entitycatalog/verified_asset_registry.go`:
path/env discovery, manifest version check, SHA-256 re-verification, Drive
locator, media type and key normalization. `scripts` keeps only the overlay
binding and now propagates a malformed/unsupported registry as an error
instead of silently returning no assets.

**Migration mechanics.** The registry half is now done by pointing
`overlay_entity_asset_registry.go` consumers at the entitycatalog-owned
loader; `eb4bf234b` already made that package fail closed on
retired/unrecognized catalog states. Card selection remains with the
`compile_editorial_plan` op (13d), together with the Go planner cutover.

Evidence (2026-11-15, `refactored`):

```sh
go test ./internal/capabilities/images/entitycatalog -count=1   # pass
go test ./internal/capabilities/scripts -run 'TestRegistry|TestApplyVerifiedEntityAssets|TestApplyOverlayPresentation' -count=1   # pass
go vet ./internal/capabilities/images/entitycatalog ./internal/capabilities/scripts   # pass
```

The 13b targeted tests and the entitycatalog package pass. The full
`scripts/...` tree is green as well (`go test ./internal/capabilities/scripts/... -count=1`);
the three localized-render tests were resolved by the concurrent Chronon
refactor, and the final-job subtitle matrix passes after the single certified
render language was restored.

### 13c — audio timing: verify, then delete residue — DONE (2026-11-15)

`render_audio_plan` already returns certified word-level timing; the
editorial extractor requires it before anything renders (`editorial_overlay`
comments say "certified word audio timing"). Canonical sample rate lives in
`audio_copy::CANONICAL_SAMPLE_RATE` (ticket 09).

Audit result:

- The canonical scene timeline is compiled once by
  `compileResolvedSceneTimeline`; final duration/profile come from the
  `render_audio_plan` response metadata. No Go path re-derives event timing
  from the rendered file or a second audio timeline.
- The residual duplicate authority was the 40 ms AAC padding tolerance,
  repeated as literals in five Go gates. It now lives only in
  `internal/capabilities/audio` as `FinalAudioDurationToleranceUS` /
  `FinalAudioDurationToleranceMS`, consumed by `ValidateFinalAudio`,
  `ValidateFinalAudioReference`, `ValidateMasterAudioInvariants`,
  `final_job_payload.go` and `final_job_compiled_plan.go`. No numeric literal
  for this window remains outside the constant definition.
- Deleted: `internal/platform/audio/mp3_duration.go` and its tests. The MP3
  frame walker was a second duration rule for provider assets; synthesized
  audio duration now comes only from the authoritative shared media executor
  probe. The ~140 chunk probes per run return to the shared 4-runner pool —
  accepted as the cost of single duration authority, not hidden behind a
  local parser.

Evidence (2026-11-15, `refactored`):

```sh
gofmt -w internal/capabilities/audio/strategy.go \
  internal/capabilities/scripts/audio_timeline.go \
  internal/capabilities/scripts/final_job_payload.go \
  internal/capabilities/scripts/final_job_compiled_plan.go \
  internal/capabilities/scripts/audio_timeline_test.go
go vet ./internal/capabilities/audio ./internal/platform/audio ./internal/capabilities/scripts/...   # exit 0
go test ./internal/platform/audio ./internal/capabilities/audio ./internal/capabilities/scripts/...   # all packages pass
```

Commits: `8bc9ad456` (single tolerance) and `0868387d4` (MP3 parser
deleted), both on `main` and local/unpushed at ticket update time.

Probe cost (2026-11-15, `refactored`): `BenchmarkProbeTTSMP3` on a
representative synthesized MP3, 100 probes per run × 5 runs, one media
execution slot, measured **10.94–12.08 ms/probe** (median ≈ 12.00 ms) and
**82.7–91.3 probes/s**. A shorter 30-probe burst measured 7.63–11.71 ms.
This is the isolated sequential probe cost; production TTS fans out through
the shared four-runner media executor. It is not a before/after comparison
with the removed parser: the old ~1.1 s note described a probe paid under
four-slot contention, not the isolated cost.

```sh
VELOX_TTS_PROBE_SAMPLE=data/media/voiceovers/<sample>.mp3 \
  go test ./internal/platform/media/rustexec -run '^$' \
  -bench '^BenchmarkProbeTTSMP3$' -benchtime=100x -count=5
```

### 13d — the last big cut: `compile_editorial_plan` — RUST OP + BATCH PORTS DONE (2026-11-15); SINGLE-CALL CUTOVER OPEN

`internal/capabilities/scripts/editorial_overlay.go` — 444 lines. Today it:

- Decides which extractive editorial text renders (manifest + language +
  scene identity + verbatim grounding + word timing must all agree).
- Guards phrase edges with `editorialBoundaryStopWords` — the comment itself
  says "guards phrase edges **the same way the Rust extractor does**": a
  same-rule-maintained-twice hazard. The stop-word set comes from the
  `LexiconRegistry` (config/lexicons/**) — Rust loads lexicons natively
  (`lexicon/` crate); Go should not bootstrap them at all.
- Grows short phrases to the 3-word minimum (`expandEditorialPhraseToMinimumWords`)
  with a 5-word ceiling and a content-word ending rule — pure editorial
  policy, currently Go-owned.
- Guards hanging punctuation (`,;:` endings) — again mirrored Rust logic.
- Parses chart values, validates title grounding and title source.

Target shape: a single op `compile_editorial_plan` in the `overlay-compiler`
crate. Request = `semantic_manifest.v3` + plates + scene windows + fps +
language + toggles. Response = final overlay plan (callouts, cards, map
windows already collision-resolved and cascaded). This **subsumes** the four
existing per-concern ops (`rank_entity_overlays`,
`trim_map_windows_before_budget`, `resolve_map_overlaps`,
`cascade_clustered_entrances`) — after the cutover those ports can be folded
into the one port, keeping `mapTiming`'s apply-edits-only pattern as the
response-consumption template.

Go side after cutover: build the request from the manifest, send it, apply
the response, fail closed (typed error `overlay plan: BrainRust editorial
compiler is required: …`) on worker absence. Same discipline as
`fc39e03e8`: no fallback compilation, no source-order stand-in.

**Rust op landed (2026-11-15).** `overlay-compiler` now exposes
`CompileEditorialPlanRequest` / `CompiledEditorialPlan` and
`compile_editorial_plan`, which batch the four existing decisions
(pre-budget map trim, map overlap resolution, entity ranking, entrance
cascade) in one call. `semantic-engine` dispatches
`compile_editorial_plan` and returns `{ok, plan}`. The per-concern functions
and handlers remain the pure building blocks until the Go cutover; they are
not a second authority because the batch calls exactly those functions.

Evidence:

```sh
cargo test -p overlay-compiler --lib        # 8 passed
cargo test -p semantic-core --lib           # 151 passed
cargo test -p semantic-engine --lib         # 53 passed
cargo fmt --all -- --check                  # exit 0
```

**Go contract + batch ports landed (2026-11-15, `68ddb83f5` /
`652c03217`).** `internal/platform/media/rustexec/semantic_engine_editorial_plan.go`
adds `CompileEditorialPlan` with typed request/response and apply-edits-only
outputs (`PreBudgetItems`, `ResolvedItems`, `RankedEntities`,
`EntranceItems`). It reuses the same `applyMapTimingEdits`,
`applyCascadeEntranceEdits` and `validateEntitySelections` helpers as the
legacy adapters. `TrimMapWindowsBeforeBudget`, `ResolveMapOverlaps`,
`RankEntityOverlays` and `CascadeClusteredEntrances` now all call this one
operation; the old per-concern wire operations are no longer sent.

This is **not** yet the one-call plan compilation the ticket requires. The
planner still invokes the four ports at four different phases, each carrying
only that phase's inputs. Trim must run inside `BuildPlan`'s budget, ranking
inside card resolution, cascade after motion assignment, and overlap
resolution at seal. Collapsing those into one request requires moving the
interleaved planner phases into Rust, then deleting `editorial_overlay.go`
and the Go card selection. Applying the batch at the end would compute trim
and cascade on items the planner has not produced yet, so it is deliberately
not wired as a silent approximation.

Evidence:

```sh
go test ./internal/platform/media/rustexec -run 'TestCompileEditorialPlan|TestOverlayPortsUseTheSingleBatchOperation' -count=1   # pass
go vet ./internal/platform/media/rustexec/...                                             # pass
```

Remaining for 13d — single-call port (OPEN, not implemented). This is not a
mechanical swap: the Go planner package is 15,171 non-test LOC with 14,660 LOC
of tests, and the scripts overlay planner/editorial/card layer is another
6,098 non-test LOC. The four phases are interleaved because trim feeds
`BuildPlan`'s budget, ranking feeds card resolution, cascade feeds motion
layout, and overlap resolution feeds the seal. A batch applied after the
planner would run those decisions on items the planner has not selected yet;
that is forbidden by this ticket.

Execution plan:

1. Expand `CompileEditorialPlanRequest` into a versioned full-plan request:
   language, fps, scene windows, canvas/toggles/budgets, semantic manifest,
   scene text/timing/annotations, entity timeline, resolved plate/image refs,
   and the runtime media/materialization index. Response must be either the
   final item set plus placements/intake or a complete, validated edit set;
   no nullable "fallback" path.
2. Port the planner phases in their real order to `overlay-compiler`: scene
   surfaces + chapter/editorial projection, entity card resolution + ranking,
   admission/budget/group caps, style/animation finalization, and seal. Keep
   opaque render metadata opaque; never re-derive it in Rust.
3. Replace `compileOverlayPlanWithMotionOffset` with: build request → one
   `compile_editorial_plan` call → validate/apply response → seal. Delete
   `editorial_overlay.go`, the Go card-selection helpers and the four
   per-concern ports once parity is proven.
4. Add parity/golden coverage before deleting anything: map trim, entity
   ranking, entrance cascade, map-overlap resolution, editorial/card category
   filtering, image/materialization rejection, and byte-identical sealed
   plans for representative fixtures. Add a real-worker test for the single
   round-trip.
5. Run `cargo test -p overlay-compiler -p semantic-core -p semantic-engine
   --lib`, `cargo fmt --all -- --check`, `go test
   ./internal/capabilities/scripts/... ./internal/platform/media/rustexec/...`,
   `make verify-agent`, and `make verify-main` before marking 13d DONE.

Do not mark 13d DONE until steps 1–5 hold. The current verified state keeps
the four batch ports: no silent approximation and no Go-only fallback.

**Verification boundary (2026-11-15).** The full
`go test ./internal/platform/media/rustexec/...` suite passes (74s), and
`make verify-agent` passes end-to-end (`final: PASS`) with every component
reported PASS or cache-hit, including `script`, `rust` and `timeline`.
`make verify-main` also passes end-to-end: foundation, static, full Go tests,
changed-components (`final=PASS`) and architecture checks
(`CLIP_RENDER_CUTOVER=PASS`). The full `go test
./internal/capabilities/scripts/...` tree passes, and the real-worker smoke
`TestSemanticEngineRealBinaryProjectsLinkedEntities` passes in 2.94s. No
TICKET-13 test fails.

**Final-job subtitle cleanup (2026-11-15).** The pre-render had started
iterating `fixedRenderLanguages`, producing one fragment per target language.
A final job is one video in the language the run already certified;
`ensureFinalJobFixedMediaSubtitles` now resolves that single language through
`finalJobRenderLanguage` and skips every already-certified clip. The four
`final_job_subtitles` tests pass, and the full `scripts/...` tree is green
after the concurrent localized-render refactor updated its expectations and
fixtures.

### 13e — in-flight concurrent work (do not duplicate, just sequence after)

- `BrainRust/pipelinegen-muscles`: `admin_media.rs` (151) /
  `render_clip_exec.rs` (335) / `protocol.rs` demolition uncommitted at
  ticket-open time.
- `refactored/internal/capabilities/images/entitycatalog` + adapters:
  P5 policy work (`eb4bf234b` committed; wiring edits may still be in
  flight).

## Execution order and per-unit gates

1. **Push the verified local commits** — DONE (2026-11-15). `make verify-main`
   passed once, then `03f21d132..c06aef174` (refactored),
   `4871256..98a1bf3` (BrainRust) and `fe1019b..7fee04b` (root) were pushed.
   13a starts from this pushed base.
2. **13c (audit-only) — DONE (2026-11-15)** — cheapest unit, no Rust
   change; the residue list is recorded above and the duplicated 40 ms
   tolerance was collapsed to one Go constant.
3. **13a — DONE (2026-11-15, `b6f5379` + `c4c12ac98`)** — rules 1–4 are
   owned by BrainRust; the durable and localized projections only carry the
   decision. Targeted entity/translation and rustexec tests pass.
4. **13b — REGISTRY DONE (2026-11-15, `8b4482660`)** — verified registry
   moved to the Go `entitycatalog` owner; Rust `imagecatalog` stays offline
   by canonical design. Card selection stays with 13d's op.
5. **13d — Rust op + Go batch ports DONE (2026-11-15); SINGLE-CALL PORT OPEN** —
   `compile_editorial_plan` batches the four timing/ranking/cascade decisions
   in `overlay-compiler`, and all four Go ports now use it. `make verify-agent`
   and `make verify-main` pass. Remaining: execute the single-call port plan
   above; do not mark 13d DONE before parity.

Per-unit verification commands:

```sh
# refactored (per unit)
make verify-agent
go test ./internal/capabilities/scripts/... -count=1
go test ./internal/capabilities/entities ./internal/capabilities/images/entitycatalog \
  ./internal/capabilities/scripts/adapters -count=1

# refactored (once, before push)
make verify-main

# BrainRust (Rust-side unit)
cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib
cargo fmt --all -- --check
RUST_ICU_MAJOR_VERSION_NUMBER=70 cargo clippy --workspace --all-targets -- -D warnings

# cross-repo contract parity (after any schema edit)
bash contract/mediaexec.v2/check_consumer_copy.sh ../refactored/testdata/mediaexec/v2
```

## Risk register

- **Silent re-classification drift during migration**: if Go keeps its rules
  while Rust gains new ones, both stay green and outputs diverge. Mitigation:
  each 13x unit deletes the Go rule in the same commit that makes Rust
  authoritative — never keep both temporarily.
- **Lexicon bootstrap asymmetry**: Go's `LexiconRegistry` and Rust's
  `lexicon/` may disagree on stop-word sets today. Before 13d, diff the two
  sources and reconcile in Rust only; record the diff in the unit's commit
  message.
- **Golden regression in the hot path**: 13d touches the generation flow.
  Run the existing e2e (`map_plan_e2e_test.go` and the semantic e2e suites)
  plus a real-worker smoke (`TestSemanticEngineRealBinaryProjectsLinkedEntities`)
  before pushing.
- **Concurrent sessions**: three workstreams were live at ticket-open time
  (muscles demolition, entitycatalog P5, this ticket). Each 13x unit must
  rebase on `origin/main` before starting and re-run the parity gate after
  any contract edit.

## Out of scope

- Ticket 08 (editorial gold set), ticket 10 (multimodal pgvector), Gate B
  human annotations, Gate D provider measurements — externally blocked, keep
  their status in their own tickets.
- Chronon render-side changes (card *rendering*, animations): out of scope;
  only selection/planning moves.
