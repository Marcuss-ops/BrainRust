# TICKET 14 — overlay planner single-call port

- Priority: P1
- Status: PARTIAL (Rust single-call compiler landed; Go cutover open)
- Source: [TICKET-13](TICKET-13-go-demolition-closeout.md) §13d and the
  2026-11-15 architecture audit
- Owners: BrainRust `overlay-compiler` / `semantic-engine`; PipelineGen
  `internal/platform/media/rustexec` and `internal/capabilities/scripts`
- Depends on: TICKET-13 13a–13c DONE; 13d Rust batch op and Go batch adapter
  landed
- Blocks: TICKET-13 final closure (13d remains PARTIAL until this ticket is
  DONE)
- Scope: the scripts overlay-plan hot path only. Render-side Chronon changes,
  ticket 08/10 external gates and the offline `imagecatalog` crate are out of
  scope.

## Goal

Make one `compile_editorial_plan` request produce the complete, validated
overlay plan for a run. PipelineGen builds the request, performs only
projection/validation of the response, and uses the returned final plan. The
four current phases must disappear as independent Go worker calls:

1. pre-budget map-window trim;
2. entity ranking and card selection;
3. clustered entrance cascade;
4. final map-overlap resolution.

The target is not a smaller transport wrapper around the current four calls.
The target is a single call whose **request is the complete planning input**
and whose **response is the final overlay plan**:

```text
manifest + scenes + timing + entities + media refs + canvas/toggles/budgets
        -> compile_editorial_plan (one request)
        -> final OverlayPlan (items, placements, intake, duration)
```

After this ticket, Go must not contain a second implementation of editorial
selection, entity-card selection, admission budgets, map trim/composition,
entrance cascade, overlap resolution or planner animation layout. Go may still
own the surrounding application concerns: reading the request/result, verified
media materialization, persistence, logging and transport error handling.

## Progress — 2026-11-15 (Rust single-call compiler landed)

WP0 (fixtures) and WP1 (complete contract) are done, and the planner phases
now run inside Rust. `overlay-compiler::editorial_plan_v2` owns:

- **request** (`request.rs`): plan identity, seal extent, canvas (group
  limits, map presentation, animation pools and directives), timed scenes with
  certified annotations, entity timeline, media refs, budgets and canonical
  scene windows; every section validates fail-closed;
- **response** (`response.rs`): `EditorialPlanV2Compiled` / `FinalPlanV2` with
  typed items, intake, recap/chart counts, duration and output profile;
  `validate_for` rejects a plan that does not belong to its request or whose
  items escape their scene;
- **planner** (`planner.rs`): one `compile_overlay_plan_v2` call runs candidate
  generation, pre-budget map trim, admission, entrance cascade, final overlap
  resolution and seal. Chapter bullets own the bullet slot on the scenes they
  cover, and a grounded place renders as a map without a duplicate location
  card;
- **errors** (`error.rs`): typed `EditorialPlanError`; no empty-plan or
  fallback path.

`semantic-engine` routes `compile_editorial_plan` by the declared
`schema_version` before deserializing: `editorial_plan.v2` goes to the full
compiler, an unknown version is rejected, and only a request with no version
stays on the sparse legacy batch. This closes a real hazard: the legacy request
defaults every section, so a v2 request read as legacy would have returned an
empty plan instead of failing closed.

Fixtures live in `overlay-compiler/tests/fixtures/overlay_plan/`:

- five request fixtures (phrases only; map trim/overlap; entity cards with and
  without verified media; editorial toggles with charts and a chapter recap; a
  localized run with a short-phrase scene and an untimed fixed-media scene);
- one plan golden per request, produced by the real planner through
  `cargo run -p overlay-compiler --example compile_fixture -- <request>`;
- three malformed fixtures (unknown version, duplicate scene, unmaterialized
  semantic asset).

`tests/overlay_plan_contract.rs` (23 tests) compares every compile to its
frozen golden byte-for-byte, checks determinism, verified-versus-text-only
entity media, map trim/resolution, chart and chapter-bullet admission, localized
surfaces and the negative contract cases. `tests/editorial_plan_protocol.rs`
(6 tests) covers the worker routing.

Still open before DONE:

- WP6: Go production still calls the four phase ports; the Go editorial and
  card selection are still present and `validate_for` is not yet the adoption
  path;
- WP5 byte parity against the current Go planner (these goldens are Rust
  self-consistency; Go-oracle parity still needs the Go export harness);
- WP7: real-worker one-request-per-plan counter test and the deletion guard;
- one phase-order deviation: entity cards are generated as candidates before
  admission rather than merged after it. This must be aligned before cutover,
  not hidden;
- `make verify-agent` / `make verify-main` on the cutover tree (not run here:
  the Go production tree is unchanged apart from the worker contract routing).

### WP6 cutover — blocked on planner parity (attempted 2026-11-15)

The Go cutover was scoped and measured, then deliberately **not** landed: the
Rust plan is not yet a behavior-preserving replacement, and adopting it would
be the silent approximation this ticket forbids.

Measured parity gap (Rust planner versus the Go planner it would replace):

| Decision | Go production | Rust v2 planner today |
| --- | --- | --- |
| Motion selection | certified `motion_id` per lane, recycled by counts | phrase/image lanes still emit none; entity cards are ported |
| Render keys | recomputed at seal | cleared at seal (items carry none); Go recomputes SHA-256 |
| Dates | `timeline_date` lane, scene-capped | no date lane |
| Chains/composites | `Prepare/FinalizeCompiledAnimationLayout`, coalescing | absent |
| Presentation | fonts, frame, style, caption projection | absent |
| Image effects / light leaks | deterministic effects, clip-start leaks | absent |
| Verified intent images | fills a freed image slot | absent |
| Recap/chart placements | placement counts and cards | `recap_placements`/`chart_placements` hardcoded 0 |
| Entity image motions | `assignEntityImageMotions` + below style | **ported**: request pools rotated by `(image_motion_offset + ordinal) % len`, independent caption lane, `premium_below_random_v1`, `image_scale_in`, `enter_frames: 8` |
| Review naming | `NameItemsForReview` | absent |

Concrete gates that would fail on adoption:

- `TestCompileOverlayPlanRetryKeepsStableIndependentImageMotions` requires two
  `entity_image` items with distinct, non-empty, certified `MotionID`s; the
  Rust plan emits none.
- `TestOverlayPlanGoldenMix` requires 100 items, including 10 maps, 10 entity
  images, dates and no chain fusion; the Rust plan has no date lane and no
  chain pass.
- The presentation/seal, map/date/chart and intake tests assert the stages the
  Rust plan does not implement.

Queued lanes not landed in this pass (2026-11-15):

- **Chain/composite coalescing** (`CoalesceCaptionedEntityImageCards` /
  `fuseChain`): the `chain_segments` item schema, segment timing and motion
  re-sealing must be ported together; emitting a partial chain would change
  render order and fail `ValidateChainItem`.
- **Date lane** (`timeline_date`): the v2 request does not yet carry grounded
  date candidates, and date cards depend on the seeded ChrononTemplate
  preset/motion sampler; a Rust date card would be an approximation.
- **Verified intent images**: needs `entity_ref` on the response item and the
  seeded `SelectEntityImagePreset` sampler; admitting the asset alone would emit
  a card whose preset Go cannot reproduce.

These remain part of WP3/WP4. The entity-card motion/style lane and seal-time
render-key clearing are the only new parity in this pass.

Therefore WP6 stays open. The four phase ports are already reached only through
the single `compile_editorial_plan` batch; deleting them before parity would
remove the working authority without a replacement. The cutover must follow
WP3–WP5, in this order: admission/budgets and entity-card merge, then layout /
presentation / seal, then one `CompileOverlayPlan` adapter and the deletion of
the four ports.

### Evidence (2026-11-15, this session)

```sh
cargo fmt --all -- --check                                        # rc 0
cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib
# overlay-compiler 38 passed, semantic-core 151 passed, semantic-engine 53 passed
cargo test -p overlay-compiler --test overlay_plan_contract        # 27 passed
cargo test -p semantic-engine --test editorial_plan_protocol       # 6 passed
cargo test -p semantic-engine --test protocol_validation           # 4 passed
cargo clippy --workspace --all-targets -- -D warnings              # rc 0
cargo clippy -p overlay-compiler --all-targets -- -D warnings      # rc 0 (entity-card lane)

gofmt -l internal/capabilities/scripts internal/platform/media/rustexec \
  internal/kernel/overlayplan                                      # empty
go vet ./internal/platform/media/rustexec ./internal/capabilities/scripts  # rc 0
go test ./internal/capabilities/scripts -run TestBuildOverlayPlanV2Request -count=1  # pass
go test ./internal/platform/media/rustexec \
  -run 'TestCompileEditorialPlan|TestOverlayPortsUseTheSingleBatchOperation' -count=1  # pass
```

## Current state (measured 2026-11-15)

`BrainRust/overlay-compiler/src/lib.rs` already exposes:

- `compile_editorial_plan(CompileEditorialPlanRequest)` returning a
  `CompiledEditorialPlan` with four sparse edit sets:
  `pre_budget_edits`, `timing_edits`, `ranked_entities`, `entrance_edits`;
- the pure functions `trim_map_windows_before_budget`,
  `rank_entity_overlays`, `cascade_clustered_entrances` and
  `resolve_map_overlaps`.

`BrainRust/semantic-engine/src/protocol.rs` dispatches
`compile_editorial_plan` and the four per-concern operations. The worker is
persistent, but the protocol is **NDJSON one-request/one-response**; there is
no stateful planner session.

`refactored/internal/platform/media/rustexec/semantic_engine_editorial_plan.go`
sends the batch and applies the sparse edits. The four existing ports
(`TrimMapWindowsBeforeBudget`, `ResolveMapOverlaps`, `RankEntityOverlays`,
`CascadeClusteredEntrances`) all route through that one batch op, but the
planner still invokes them at four different phases:

- trim inside `BuildPlanWithMapWindowTrimmer` (`overlay_plan_compile.go`);
- ranking inside `resolveOverlayEntityCardItems`
  (`overlay_plan_compile_phases.go`);
- cascade after `assignEntityImageMotions` (`overlay_plan_compile.go`);
- resolve inside `sealOverlayPlan` (`overlay_plan_seal.go`).

The remaining Go decision surface is large: `internal/capabilities/overlays`
is 15,171 non-test LOC with 14,660 test LOC; the scripts editorial/card layer
is another 6,098 non-test LOC. `editorial_overlay.go` still decides grounded
titles/bullets, phrase edges, minimum word counts and chart values.
`overlay_entity_cards.go` / `overlay_entity_cards_text.go` still select and
compose entity cards.

The first slice of the target contract has landed: `overlay-compiler::editorial_plan_v2`
now owns the versioned `editorial_plan.v2` request envelope (plan identity,
language, canvas toggles, timed scenes, editorial and chapter manifests) with
fail-closed validation, and `internal/kernel/overlayplan` plus
`internal/capabilities/scripts/overlay_plan_v2_request.go` are the Go producer.
The envelope is deliberately not on the wire yet, and its current response is
only `scene_phrases`/`covered_scenes`: this ticket extends that same v2 contract
to the full planning input and the final plan. Do not introduce a third schema.

### Parity harness (2026-11-15)

`refactored/internal/capabilities/scripts/overlay_plan_v2_parity_test.go`
compiles the checked-in `minimal_phrase_only` v2 fixture through the Go planner
and compares the normalized decision surface (kind, template, editorial role,
scene, text and absolute window) with the Rust sealed golden.
`TestOverlayPlanV2ParityFixtureShape` is always-on; the differential is opt-in:

```sh
OVERLAY_PLAN_PARITY=1 go test ./internal/capabilities/scripts \
  -run TestOverlayPlanV2ParityMinimalPhraseOnly -count=1 -v
```

Current result: Go keeps 2 items (`entity_image` + `scene_bullet`), Rust keeps
3 items (`scene_title` + `scene_bullet` + the plain phrase). The delta is
admission/dedup and entity-card ordering, not only the style/animation phases.
Do not switch the planner or delete the Go phases until this reports zero
diffs and the harness is expanded to the map, entity and localized fixtures.

Current gates are green, but they prove the old contract, not this ticket:

```sh
cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib
go test ./internal/capabilities/scripts/... ./internal/platform/media/rustexec/...
make verify-agent
make verify-main
```

## Non-negotiable constraints

- **No fallback planner.** If the Rust worker is absent, returns malformed
  JSON, omits a required section or fails validation, the compile fails with a
  typed error. Never fall back to the Go planner, an empty plan or source-order
  candidates.
- **No silent approximation.** Trim must still run before the budget, ranking
  before card admission, cascade after motion assignment, and overlap
  resolution after final layout. The port must preserve this order.
- **No lossy round-trip.** Opaque render metadata (`params`, `asset_refs`,
  `entity_ref`, `map`, `image_layers`, style and animation payloads) must
  survive Rust -> Go unchanged. Prefer typed DTOs matching `capmodel` JSON
  tags; use `serde_json::Value` only for fields that are genuinely opaque.
- **Rust owns decisions; Go owns application state.** Go may validate that
  the response is internally consistent and may attach already-materialized
  media/persistence data. It must not select, rank, trim, cascade, budget or
  reclassify overlay items after the call.
- **One production path.** Do not leave the four phase ports behind a toggle.
  During development, parity tests may call both paths; production must expose
  only the single call at the cutover.
- **Determinism.** Same request bytes plus same assets produce byte-identical
  response items and sealed-plan JSON, excluding only fields the current
  planner already treats as non-deterministic.

## Target contract

Extend the already-landed `editorial_plan.v2` envelope. It is not on the wire
yet, so additive sections are allowed without a version bump; a breaking change
after a v2 consumer ships requires a new version. Keep the operation name
`compile_editorial_plan`. Unknown schema versions and malformed required fields
fail closed.

### Request

The request must contain everything Rust needs to make every decision. It must
not contain a precomputed selection or a completed item list from Go, except
for already-materialized media refs and resolved plate/image candidates that
are certified inputs.

```json
{
  "operation": "compile_editorial_plan",
  "schema_version": "editorial_plan.v2",
  "plan": {
    "plan_id": "plan-...",
    "video_id": "video-...",
    "project_id": "project-...",
    "motion_seed": "...",
    "coherence_seed": "...",
    "width": 1920,
    "height": 1080,
    "fps_num": 30,
    "fps_den": 1
  },
  "language": "it",
  "source_language": "it",
  "seal": {
    "audio_duration_us": 0,
    "output_profile_id": "...",
    "background": null
  },
  "canvas": {
    "overlay_categories": {},
    "group_limits": {},
    "max_phrase_overlays": 0,
    "max_image_overlays": 0,
    "max_map_overlays": 0,
    "max_overlays_per_scene": 0,
    "map_presentation": {},
    "animation_counts": {},
    "phrase_motions": [],
    "short_phrase_motion": "",
    "long_phrase_motion": "",
    "image_motions": [],
    "image_with_text_motions": [],
    "entity_style_id": "",
    "animation_directives": [],
    "animation_contract_version": 0,
    "animation_catalog_digest": ""
  },
  "editorial": { "schema_version": "editorial.v1", "...": "..." },
  "chapters": { "schema_version": "chapter_manifest.v1", "...": "..." },
  "scenes": [
    {
      "id": "s1",
      "text": "...",
      "duration_ms": 3000,
      "execution_mode": "",
      "short_phrase_overlay": false,
      "overlay_categories": {},
      "voiceover": { "timing": { "duration_us": 3000000, "words": [] } },
      "timeline_start_us": 0,
      "duration_us": 3000000,
      "annotations": {
        "phrases": [],
        "keywords": [],
        "images": [],
        "numbers": [],
        "chart_numbers": [],
        "brand_texts": [],
        "quotes": [],
        "products": [],
        "logos": [],
        "maps": [],
        "multi_entity_groups": [],
        "end_us": 3000000
      }
    }
  ],
  "entity_timeline": { "scenes": [] },
  "media": {
    "materialized_assets": [],
    "verified_entity_assets": [],
    "scene_image_candidates": []
  },
  "budgets": {
    "phrase_overlays": 0,
    "image_overlays": 0,
    "map_overlays": 0,
    "max_per_scene": 0,
    "short_phrase": 0,
    "long_phrase": 0,
    "bullet": 0
  },
  "scene_windows": {}
}
```

The `annotations` section carries certified semantic surfaces and their exact
timing; it is not the planner's admitted item list. Rust still generates the
planner candidates, applies phrase/image/map lanes and budgets, and selects
the final set. `media` carries only materialized/verified refs and certified
plate or image candidates; it never carries a pre-admitted overlay.

Rules:

- The request is built from the same typed inputs currently passed to
  `compileOverlayPlanWithMotionOffset`: `GenerateResult`, language, canvas,
  plan/video/project IDs, resolved scenes, entity timeline, plates, image
  candidates, materialized refs and production plan/intake budgets.
- The request carries the certified `editorial.v1` and
  `chapter_manifest.v1` projections produced from `semantic_manifest.v3`.
  Missing certification, fingerprint or profile version is an error, not a
  no-op fallback to narration annotations.
- Scene text and exact word timings are required wherever a title, bullet,
  phrase, map, chart, date or entity card can be emitted. An ineligible scene
  is represented explicitly (empty candidates) rather than left ambiguous.
- Map candidates carry certified plate coverage. Rust never invents a plate,
  coordinate, asset URL or SHA-256.
- Entity cards carry the same canonical identity, scene and timing fields now
  consumed by `ResolveEntityOverlayPlanWithRanker`; Go materializes verified
  assets before the request and marks unresolved semantic URLs as
  non-renderable.
- Budgets and toggles are the resolved production values, not Go-computed
  candidate counts. Rust owns admission and drop accounting.
- Keep the request JSON free of Go-only pointers, interfaces and logger
  objects. It must be serializable and replayable as a fixture.

### Response

```json
{
  "ok": true,
  "schema_version": "editorial_plan.v2",
  "plan": {
    "schema_version": "editorial_plan.v2",
    "plan_id": "plan-...",
    "video_id": "video-...",
    "project_id": "project-...",
    "language": "it",
    "width": 1920,
    "height": 1080,
    "fps_num": 30,
    "fps_den": 1,
    "duration_ms": 0,
    "background": null,
    "items": [],
    "recap_placements": 0,
    "recap_cards": 0,
    "chart_placements": 0,
    "chart_cards": 0,
    "timed_scenes": 0,
    "intake": {},
    "output_profile_id": ""
  }
}
```

Rules:

- `items` is the final admitted set, in final render order, with final timing,
  motion, style, chain/composite grouping and `render_key` cleared or
  recomputed exactly as the current seal does.
- Item identity is final: no placeholder IDs that Go later rewrites. If the
  current planner assigns review names or item IDs after layout, that pass
  moves to Rust before the response is returned.
- `intake` contains the same drop accounting the current `newOverlayPlanIntakeTracker`
  produces; it is observability, not a second selection stage.
- `duration_ms` is derived from the request's canonical audio/scene extents and
  final item windows. Go must not re-derive a different duration.
- The response must be complete. A missing item section, a duplicate ID, an
  item outside its scene window or an unvalidated map overlay is an error; Go
  must not repair the plan.
- On failure return `{"ok":false,"error":"..."}` with a stable, human-readable
  reason. Go wraps it as
  `overlay plan: BrainRust editorial compiler is required: ...`.

## Work packages

### WP0 — Freeze the baseline and fixtures

Before moving code, capture the current planner as the oracle:

- Add a checked-in fixture matrix under
  `BrainRust/overlay-compiler/tests/fixtures/overlay_plan/` containing the
  request and the **current** sealed plan for:
  - a minimal phrase-only scene;
  - maps with trim, overlap and a scene boundary;
  - a map plus an unanchored card that must shift or drop;
  - entity cards with verified media and without media;
  - editorial title/bullet/category combinations;
  - a short-phrase scene (no fallback bullet);
  - translated localized overlays;
  - numbers/dates/products/logos/quotes and chart values;
  - recap and chart placements;
  - a mixed plan with chain/composite fusion;
  - empty/no-timing scenes and a plan that must return no plan.
- Store both the Go-side input projection and the sealed `OverlayPlan` JSON so
  the Rust result can be compared byte-for-byte.
- Add a script or Go test that regenerates the fixtures from the current
  production path only while this ticket is open. Once cutover lands, the
  fixtures are immutable contract inputs.

**Exit:** fixtures are reviewed and reproduce the current plan exactly.

### WP1 — Extend the v2 DTO and protocol

The v2 envelope already exists in
`BrainRust/overlay-compiler/src/editorial_plan_v2.rs`. Extend it additively;
do not create a parallel `overlay_plan.v1`. The Go producer is
`internal/capabilities/scripts/overlay_plan_v2_request.go` with types in
`internal/kernel/overlayplan`.

Files:

- `BrainRust/overlay-compiler/src/editorial_plan_v2.rs` and its new submodules
  (split rather than growing one file):
  - `request.rs` — request/response DTOs and schema validation;
  - `error.rs` — typed compile errors;
  - `editorial.rs` — manifest projection and editorial rules;
  - `entities.rs` — entity ranking and card selection;
  - `planner.rs` — candidate generation, admission and item ordering;
  - `timing.rs` — trim, map composition and overlap resolution;
  - `layout.rs` — animation layout, finalization and presentation;
  - `seal.rs` — duration, intake, plan validation.
- `BrainRust/semantic-engine/src/protocol.rs` — dispatch the v2 request to the
  full compiler; reject unknown schema versions.
- `BrainRust/overlay-compiler/tests/overlay_plan_contract.rs` — round-trip
  fixtures and malformed-request tests.

Implementation notes:

- Keep the current pure timing functions; they are building blocks, not a
  second authority.
- Do not put the whole planner in `lib.rs`; keep the repository's file-size
  and module conventions.
- Serialize deterministically (`BTreeMap` where ordering matters, stable item
  sort keys).
- Add `#[serde(deny_unknown_fields)]` only where forward compatibility is not
  required; version the contract instead of silently accepting fields that
  change semantics.

**Exit:** Rust parses a full request, validates it and returns a typed
response; no planner logic is wired yet.

### WP2 — Port scene surfaces and editorial projection

Move the decisions currently in:

- `refactored/internal/capabilities/scripts/editorial_overlay.go`:
  `projectEditorialOverlays`, `projectChapterBullets`,
  `editorialOpeningPhrase`, phrase grounding/location, title/bullet source and
  category filtering;
- `refactored/internal/capabilities/scripts/overlay_plan_compile_phases.go`:
  scene-surface assembly and phrase/chart/recap candidate wiring;
- the Go lexicon bootstrap and editorial stop-word/edge rules referenced by
  the ticket-13 inventory.

Port rules to Rust in this order:

1. validate manifest certification, schema, fingerprint and language;
2. verify exact scene identity and verbatim grounding;
3. locate the phrase against certified word timing;
4. apply title/bullet/category toggles;
5. apply minimum/maximum word and boundary rules using the Rust `lexicon`
   crate, not a Go-maintained stop-word list;
6. emit typed scene phrases with final offsets and roles.

Do not port Go-only logging or error text as behavior. Errors must be typed so
Go can fail closed.

**Exit:** Rust emits the same scene phrase set/order/offsets as the current Go
projection for the WP0 editorial fixtures.

### WP3 — Port entity cards, ranking and admission

Move the selection/copy decisions currently in:

- `overlay_entity_cards.go` and `overlay_entity_cards_text.go`;
- `resolveOverlayEntityCardItems` in `overlay_plan_compile_phases.go`;
- `ResolveEntityOverlayPlanWithRanker` call sites in
  `internal/capabilities/entities/overlay_resolver.go` where they are part of
  the overlay plan pipeline.

Order inside Rust:

1. build canonical occurrence scenes from `entity_timeline`;
2. run the existing `rank_entity_overlays` scoring and stable dedupe;
3. select card templates/copy and per-scene caps;
4. attach only media refs supplied in the request as materialized/verified;
5. emit text-only cards when no verified asset exists, never a fake image;
6. preserve canonical identity fields and scene/window clamping.

Go keeps `entitycatalog` verified-asset lookup/materialization and may build
the media index before the call. Rust must not read the filesystem, registry or
network.

**Exit:** entity-card fixtures match the current plan, including the
text-only fallback and the per-scene identity cap.

### WP4 — Port planner admission, budgets and timing

Move the decisions currently in `internal/capabilities/overlays` that are still
required to produce the final plan:

- `BuildPlan` / `buildPlan` orchestration;
- phrase/keyword/image/number/brand/quote/product/logo candidate generation;
- `AllCandidatesPlannerConfig`, lane caps, group caps and run-level budgets;
- `admitOverlayItems` and the intake/drop accounting;
- map candidate selection, grouping, composition and pin timing;
- recap and chart placement;
- background/light-leak placement;
- date/image collision handling and overlap degradation.

Order must remain:

```text
candidate generation
  -> pre-budget map trim
  -> editorial admission / group+run budgets
  -> entity card merge
  -> map composition and image/date placement
  -> motion assignment
  -> entrance cascade
  -> animation layout / chain fusion / presentation
  -> final map overlap resolution
  -> seal
```

Do not move the final resolve before layout. Do not make trim a post-budget
cleanup. If an existing Go helper only projects a renderer field (for example
copying an asset ref or a style map), it may stay Go; if it chooses, ranks,
drops, reorders or retimes an item, it moves.

**Exit:** Rust produces the exact admitted item set and timing for the WP0
planner fixtures; `deferred` paths are removed or become test-only wrappers
that cannot be called by production.

### WP5 — Port layout, presentation and seal

Move the final planning stages currently in:

- `internal/capabilities/overlays/animation_layout.go`;
- `internal/capabilities/overlays/policy_application.go`;
- `internal/capabilities/scripts/overlay_plan_presentation.go`;
- `internal/capabilities/scripts/overlay_plan_seal.go`.

The Rust response must include the final:

- item IDs/names, final order, chain/composite segments;
- `motion_id`, `motion_params`, presets and `render_key` state;
- `duration_ms`, recap/chart counts, `timed_scenes` and `intake`;
- background, output-profile stamp and validated plan envelope.

Keep the media-specific asset attachment/materialization in Go if it requires
local files; provide only the already-resolved refs to Rust. The seal must not
invent a duration from a different source than the current one: the request
carries the audio-plan/master duration and scene extents.

**Exit:** the full sealed plan matches the WP0 golden JSON byte-for-byte after
normalizing only documented non-deterministic fields.

### WP6 — Cut over Go and delete the old authority

Files to change/delete after parity:

- `refactored/internal/platform/media/rustexec/semantic_engine_editorial_plan.go`
  — replace the sparse-edit request/response with the full request/response;
  expose one `CompileOverlayPlan` call, not four phase methods.
- `refactored/internal/capabilities/scripts/overlay_plan_compile.go` — build
  the single request, call the adapter, validate/apply the final plan; delete
  the four interleaved phase calls.
- `refactored/internal/capabilities/scripts/overlay_plan_compile_phases.go` —
  delete scene/card selection; retain only pure request projection if needed.
- `refactored/internal/capabilities/scripts/editorial_overlay.go` — delete.
- `refactored/internal/capabilities/scripts/overlay_entity_cards.go` and
  `overlay_entity_cards_text.go` — delete the selection/copy path; keep only
  media materialization helpers if they are not duplicated.
- `refactored/internal/capabilities/overlays/map_overlap_port.go` — remove the
  four phase interfaces after the planner no longer needs them.
- `refactored/internal/capabilities/overlays/planner*.go` — reduce to
  DTO/validation code or delete the planner implementation once no production
  caller uses it.
- `BrainRust/semantic-engine/src/protocol.rs` — remove the four per-concern
  handlers from the Go contract after the single-call path ships. Keep them
  only if an independent, tested Rust caller still needs them; do not keep
  them as a hidden Go fallback.
- `refactored/internal/platform/media/rustexec/semantic_engine_overlay_timing.go`,
  `semantic_engine_entity_overlay_ranker.go`,
  `semantic_engine_entrance_cascade.go` — delete or reduce to the new adapter.

Fail-closed wiring:

```go
compiled, err := adapter.CompileOverlayPlan(ctx, request)
if err != nil {
    return nil, fmt.Errorf("overlay plan: BrainRust editorial compiler is required: %w", err)
}
if err := compiled.ValidateFor(request); err != nil {
    return nil, fmt.Errorf("overlay plan: invalid BrainRust editorial plan: %w", err)
}
return compiled.Plan, nil
```

The final function must not contain a branch that constructs a plan locally.
No environment variable, build tag or `nil` resolver may re-enable the old
path.

**Exit:** production compiles exactly one request and adopts exactly one
response; the four port symbols are gone from the planner path.

### WP7 — Parity, failure and regression gates

Add tests before deleting the old path:

- Rust unit tests for each moved decision, including the existing timing tests.
- Contract tests for schema version, unknown fields, missing sections,
  duplicate IDs, out-of-window items, invalid map windows and invalid media.
- Go golden tests comparing the Rust plan to the frozen WP0 plans.
- A real-worker test that runs the release semantic engine and asserts
  **one** `compile_editorial_plan` request per plan. The test double should
  count calls; any second request fails.
- A failure test: worker unavailable, malformed response and non-`ok`
  response all return typed errors and never a local plan.
- A deletion guard: `rg`/Go test asserts the four phase methods and the Go
  editorial/card selection symbols are absent from the production planner.

Required commands:

```sh
# BrainRust
cargo fmt --all -- --check
cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib
cargo test -p semantic-engine --test overlay_plan_contract
cargo clippy --workspace --all-targets -- -D warnings

# PipelineGen
gofmt -l .
go vet ./internal/platform/media/rustexec ./internal/capabilities/scripts
go test ./internal/capabilities/scripts/... ./internal/platform/media/rustexec/... -count=1
make verify-agent
make verify-main
```

`make verify-main` is the push gate and must pass on the exact final tree.

## Acceptance criteria

- [ ] One `compile_editorial_plan` request per overlay plan; the request
      contains the complete planning input and no precomputed selection.
- [ ] The response is the final `OverlayPlan`; Go validates and adopts it
      without selecting, ranking, trimming, cascading or reclassifying items.
- [ ] Trim, ranking, admission, cascade, layout and resolve execute in the
      documented order inside Rust.
- [ ] Golden plans match the frozen current-planner fixtures for phrases,
      maps, entity cards, editorial toggles, charts, recaps, chains and
      localized runs.
- [ ] Verified media refs survive the round-trip; unresolved semantic
      placeholders never become render assets and never become fake images.
- [ ] Missing worker, malformed response and invalid plan fail closed with no
      Go fallback.
- [ ] The four phase ports and the Go editorial/card selection are deleted
      from the production path.
- [ ] The whole Go `overlays`/`scripts` decision surface left after the port is
      projection/validation only; a follow-up audit finds no second authority.
- [ ] `cargo fmt`, Rust tests/clippy, Go tests, `make verify-agent` and
      `make verify-main` pass.
- [ ] TICKET-13 can then mark 13d DONE and move to DONE/PARTIAL closeout.

## Suggested commit sequence

Each commit must build and test; do not merge the cutover until all parity
fixtures pass.

1. `overlay-compiler: extend editorial_plan.v2 to the full-plan response`
2. `overlay-compiler: port editorial and entity-card selection`
3. `overlay-compiler: port admission, maps and timing`
4. `overlay-compiler: port layout, presentation and seal`
5. `rustexec: add full-plan adapter and real-worker contract test`
6. `scripts: cut over to one call; delete Go editorial/card selection`
7. `semantic-engine: remove obsolete per-concern handlers from the Go contract`
8. `docs: close 13d and record parity evidence`

The cutover commit (6) is the only one allowed to delete the Go planner path.
If parity fails, revert that commit, not the fixtures.

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Behavior drift while two planners exist | Freeze golden plans first; land the port before deleting; compare byte-for-byte. |
| Opaque render metadata lost in JSON | Mirror `capmodel` tags; round-trip fixtures for params/assets/maps/chains; use `Value` only for truly opaque fields. |
| Map timing changes because trim/resolve order moved | Keep the documented phase order; dedicated map trim/collision fixtures; no post-budget trim. |
| Entity cards become fake images | Pass only materialized refs; text-only fallback test; never synthesize URL/SHA. |
| Duration changes at seal | Send canonical audio/master duration and scene extents; compare `duration_ms` and item windows in goldens. |
| Locale/lexicon drift | Use the Rust `lexicon` crate; remove Go stop-word bootstrapping; localized fixtures. |
| Large change hides a regression | WP-by-WP commits; each has unit/golden coverage; cutover only after the full matrix. |
| A hidden fallback re-enables the old planner | Deletion guard + failure tests; no resolver nil branch; production adapter is mandatory. |

## Out of scope

- Chronon rendering, card rendering, animation execution and GPU/media work.
- Moving verified asset storage/registry into Rust; `entitycatalog` remains the
  owner of materialized assets.
- TICKET-08 human editorial gold set and TICKET-10 multimodal pgvector; both
  remain externally blocked.
- Reworking `analyze_script`; the manifest remains the input to this ticket.
- Performance tuning unrelated to removing the four planning calls.

## Definition of done

This ticket is DONE only when all acceptance boxes are checked, the golden
parity suite is checked in, `make verify-main` passes on the final tree, and
TICKET-13's 13d section links to that evidence. Until then TICKET-13 remains
PARTIAL; the current four-phase batch is a working compatibility step, not the
target architecture.
