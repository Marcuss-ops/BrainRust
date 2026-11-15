# TICKET 13 — Go-side demolition closeout (P2–P8)

- Priority: P1
- Status: OPEN
- Source: 2026-11-15 workspace audit ("andiamola a completare e tagliare per
  davvero") + Go→Rust migration analysis
- Owner: any session working in `refactored` (PipelineGen)

## Context

Tickets 01–12 close the BrainRust core. The remaining migration work is on the
Go consumer side: duplicated logic in
`refactored/internal/capabilities/scripts/` that still re-implements decisions
BrainRust owns, plus commits completed locally but not yet on origin.

### Already closed (this audit, 2026-11-15)

| Commit | Repo | Scope |
| --- | --- | --- |
| `2a557d432` | refactored | semantic entity linking + local scoring wired (P0) — pushed |
| `03f21d132` | refactored | `render_clip` retired from the mediaexec protocol — pushed |
| `fc39e03e8` | refactored | Go-only overlay compilation path deleted, resolver fail-closed mandatory (P1) — **local, unpushed** |
| `8d7495200` | refactored | `watermark` retired (kernel types, alias, transport, tests) — **local, unpushed** |
| `60e69c8` | BrainRust | `dist-icu72` binaries untracked (release-only distribution) — **local, unpushed** |
| `578ed4f` | BrainRust | mediaexec.v2 schema retires `watermark`/`render_clip`/`admin_render`; README cleanup — **local, unpushed** |

Push gate: `make verify-main` in `refactored` must pass once before pushing
(AGENTS.md rule). Do not push while another session has uncommitted edits in
the tree.

## Remaining demolition items

### 13a — entity + linguistics (~2.9k lines duplicated in Go scripts)

`semantic_manifest.v3` already carries roles, gloss, negative constraints and
typed intents across the boundary (ticket 12). The Go packages below still
contain decision logic that must become pass-through projection over the
manifest:

- `internal/capabilities/scripts/entity_annotations.go` (598 lines) +
  `entity_annotations_numeric.go`
- `internal/capabilities/scripts/overlay_entity_cards.go` (590) +
  `overlay_entity_cards_text.go`
- `internal/capabilities/scripts/entity_timeline.go`,
  `entity_image_motions.go`, `entity_projection.go`

Rule: any Go branch that *changes* an editorial decision instead of
projecting one is a violation. Where the Rust engine must own the decision,
move it to `semantic-core`/`semantic-engine` behind an existing port and keep
the Go side fail-closed on missing data (typed error, no silent fallback).

### 13b — imagesearch / asset registry

- `internal/capabilities/scripts/overlay_entity_asset_registry.go` and
  `entity_image_*` resolution must delegate to the `imagecatalog` crate
  (snapshot + policy per ticket 02). The Go `entitycatalog` package was made
  fail-closed on retired states in `eb4bf234b`; finish by removing the
  scripts-side registry re-implementation.

### 13c — audio timing

Verify Go no longer computes audio timing locally; it must consume
`render_audio_plan` responses only (canonical sample rate gate lives in
`audio_copy::CANONICAL_SAMPLE_RATE`, ticket 09). Any Go-side residual timing
math is duplicate authority and gets deleted.

### 13d — the last big cut: editorial overlay budget

- `internal/capabilities/scripts/editorial_overlay.go` (444 lines) is the
  only "visual budget" module left in Go (`visual_budget.go`/`priority.go`
  no longer exist). Target shape: a single worker op
  `compile_editorial_plan` in the `overlay-compiler` crate that receives the
  semantic manifest + plates and returns the final overlay plan; Go then
  makes **one** IPC call instead of the residual 12-call sequence.
- Acceptance: no Go fallback path; the op fails closed with a typed error
  when the worker is unavailable (never a silent no-op, per AGENTS.md).

### 13e — in-flight concurrent work (do not duplicate)

- `BrainRust/pipelinegen-muscles`: `admin_media.rs` / `render_clip_exec.rs` /
  `protocol.rs` demolition uncommitted at ticket-open time.
- `refactored/internal/capabilities/images/entitycatalog`: P5 policy work
  (`eb4bf234b` committed; wiring edits may still be in flight).

## Acceptance evidence

```sh
# refactored
make verify-agent                # after each demolition unit
make verify-main                 # once, before pushing main
go test ./internal/capabilities/scripts/... -count=1

# BrainRust (contract parity gate after any schema edit)
bash contract/mediaexec.v2/check_consumer_copy.sh \
  ../refactored/testdata/mediaexec/v2
cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib
cargo fmt --all -- --check
RUST_ICU_MAJOR_VERSION_NUMBER=70 cargo clippy --workspace --all-targets \
  -- -D warnings
```

## Out of scope

- Ticket 08 (editorial gold set), ticket 10 (multimodal pgvector), Gate B
  human annotations, Gate D provider measurements — externally blocked, keep
  their status in their own tickets.
