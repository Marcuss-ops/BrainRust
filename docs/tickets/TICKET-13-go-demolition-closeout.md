# TICKET 13 — Go-side demolition closeout (P2–P8)

- Priority: P1
- Status: OPEN
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
| `fc39e03e8` | refactored | Go-only overlay compilation path deleted, resolver fail-closed mandatory (P1) — **local, unpushed** |
| `8d7495200` | refactored | `watermark` retired (kernel types, alias, transport, tests) — **local, unpushed** |
| `60e69c8` | BrainRust | `dist-icu72` binaries untracked (release-only distribution) — **local, unpushed** |
| `578ed4f` | BrainRust | mediaexec.v2 schema retires `watermark`/`render_clip`/`admin_render`; README cleanup — **local, unpushed** |
| `8bc9ad456` | refactored | final-audio 40 ms tolerance owned by `audio.FinalAudioDurationTolerance*` (13c) — **local, unpushed** |
| `0868387d4` | refactored | local MP3 duration parser and tests deleted; shared executor probe is the only duration authority (13c) — **local, unpushed** |
| `eb4bf234b` | refactored | entitycatalog fail-closed on retired/unrecognized states — pushed |

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

### 13b — overlay entity cards + asset registry

- `overlay_entity_cards.go` (590) + `overlay_entity_cards_text.go` (232):
  card composition (which annotations become cards, text shaping, fallback
  cards). Card *rendering* stays in Chronon; card *selection and copy
  decisions* duplicate the editorial extractor.
- `overlay_entity_asset_registry.go` (403): re-implements the asset chain
  (registry → resolve → reserve) that the `imagecatalog` crate owns (ticket
  02: snapshot integrity, real dates, `evaluated_at`, fail-closed policy).

**Migration mechanics.** The registry call becomes a single imagecatalog
query whose snapshot policy is enforced in Rust; `eb4bf234b` already made the
Go `entitycatalog` package fail closed on retired/unrecognized states —
finish by deleting the scripts-side re-implementation and pointing
`overlay_entity_asset_registry.go` consumers at `entitycatalog` ports. Card
selection moves into the `compile_editorial_plan` op (13d) so cards and
extractive callouts are planned by the same brain in the same pass.

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

### 13d — the last big cut: `compile_editorial_plan`

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

### 13e — in-flight concurrent work (do not duplicate, just sequence after)

- `BrainRust/pipelinegen-muscles`: `admin_media.rs` (151) /
  `render_clip_exec.rs` (335) / `protocol.rs` demolition uncommitted at
  ticket-open time.
- `refactored/internal/capabilities/images/entitycatalog` + adapters:
  P5 policy work (`eb4bf234b` committed; wiring edits may still be in
  flight).

## Execution order and per-unit gates

1. **Push the verified local commits** (see table; the two 13c commits are
   included) after one `make verify-main`. Do not start 13a on an unpushed
   base.
2. **13c (audit-only) — DONE (2026-11-15)** — cheapest unit, no Rust
   change; the residue list is recorded above and the duplicated 40 ms
   tolerance was collapsed to one Go constant.
3. **13a** — entity decisions → Rust. Per AGENTS.md: package tests after the
   change, then `make verify-agent`; commit as one focused unit
   (`refactor(scripts): project entity annotations instead of re-deciding them`).
4. **13b** — asset registry → imagecatalog, card selection folded into 13d's
   op (commit the registry move first, cards together with 13d to keep
   commits focused).
5. **13d** — `compile_editorial_plan` in `overlay-compiler`, then delete
   `editorial_overlay.go` decisions + the four per-concern ports. The Rust
   side lands with its own tests first (crate tests + golden), then the Go
   cutover commit.

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
