# TICKET-21 — Hotspot remediation (structure, duplication, dead code)

Status: **DONE (2026-11-16)**. Owner: BrainRust workspace. Priority: P2.

The 2026-11-16 audit of the Rust workspace found it healthy overall — no cyclic
crate dependencies, no unreachable source files, three `unwrap`/`panic` sites in
production code (all guarded), `cargo check --workspace --all-targets` clean —
with four concrete smells left. This ticket closes them and records the
measurements, so the next audit starts from numbers instead of impressions.

## Acceptance criteria and evidence

### 1. `render_audio_execution::execute` decomposed (was 419 lines, nesting 8)

`pipelinegen-muscles/src/render_audio_execution.rs` now separates the stages:

| Stage | Responsibility |
| --- | --- |
| `execute` (39 lines, nesting 2) | destructure the request, validate the plan, build `RenderContext` |
| `RenderContext::render` | ordered pipeline: chains → FIFOs → final graph → encode run → publish |
| `track_chains` / `layer_chains` | the "Go decides" boundary: resolve and validate every event/layer |
| `resolve_asset`, `track_targeting`, `layer_targeting` | asset resolution and automation windows, split so each error keeps its wording |
| `prepare_output`, `stem_jobs` | output dir + part path, round-robin stem jobs |
| `EncodeRun::start` / `EncodeRun::finish` | spawn encoder before writers (deadlock-avoiding order), reap both with deadlines |
| `publish` | probe → timings → atomic publish → sha256, `ok:true` only after all four |

`Scratch` (RAII) now owns the temporary filter scripts and FIFOs, replacing eight
hand-written `remove_files` calls — one forgotten return used to leak a FIFO.
The internal error type is `RenderError` (message + optional source path);
`Response` is ~800 bytes, so returning it from every fallible stage tripped
`clippy::result_large_err`. One conversion happens at the boundary.

Longest function in the file: 54 lines (`finish`). Deepest nesting: 5.

Evidence: `cargo test -p pipelinegen-muscles` — 124 lib + 4 + 4 pass;
`cargo clippy -p pipelinegen-muscles --all-targets -- -D warnings` — rc 0.

### 2. `editorial_plan_v2/planner.rs` split (was 2,671 production lines)

| File | Production lines | Responsibility |
| --- | --- | --- |
| `planner/mod.rs` | 270 | prelude, `compile_plan`, `seal`, chart placement accounting |
| `planner/candidates.rs` | 626 | base/recap/entity-card/editorial/annotation candidates, item constructors |
| `planner/presentation.rs` | 566 | text splitting, styles, fonts, motion, light leaks |
| `planner/coalesce.rs` | 471 | caption/entity-card chain fusion and camera segments |
| `planner/media.rs` | 464 | images, maps, entity cards, verified-asset binding |
| `planner/admission.rs` | 323 | budgets, cascade, overlap resolution, drop accounting |

`compile_plan` keeps its signature and remains the single entry point: the split
is module-level, not API-level, so TICKET-14's single-call contract is intact.

Evidence: `cargo test -p overlay-compiler` — 64 lib + **30 contract** pass; the
contract suite compares the frozen goldens byte for byte, so the plan output is
unchanged. `cargo clippy -p overlay-compiler --all-targets -- -D warnings` — rc 0.

### 3. One factory for the `entity_search` recipe

The 24-field `ProductionCandidate` literal was duplicated verbatim in
`consolidate/concepts.rs` and `consolidate/producers.rs`. Both now call
`consolidate::entity_search_candidate`, which also owns the invariant
`requires_media == queries are non-empty` (a candidate with no query ladder has
nothing to search for). Adding a field to the struct is now one edit.

Evidence: `cargo test -p semantic-engine` — 58 lib + every integration target
pass; the consolidation output feeding the manifests is unchanged.

### 4. One ffprobe invocation

`probe::run_ffprobe` owns the argv (`-v quiet -print_format json -show_format
-show_streams [-show_data] <path>`) and classifies failures as `Spawn`/`Exit`;
callers keep their own wording, so every user-visible error message is
unchanged. Three call sites migrated (`validate_output`, `probe_file`,
`probe_source`/`probe_audio`). The per-caller serde projections stay separate on
purpose: they read different subsets of the same schema.

### 5. Dead code that pretended to validate

`visualner/src/main.rs` carried:

```rust
// Fail closed on unknown envelope values instead of silently ignoring them...
let _ = request.segment_id.as_deref();
let _ = request.text_hash.as_deref();
```

The comment promised a fail-closed check; the lines did nothing but silence
`dead_code`. The two fields and the two statements are gone (serde already
ignores unknown envelope keys, which is what "tolerated" meant), and the struct
comment no longer claims otherwise.

### 6. Pre-existing clippy lint unblocked

`semantic-core/src/phrase_align.rs::tokens_match` failed
`clippy::match_like_matches_macro` on the current toolchain (unmodified file, so
the lint predates this ticket). Rewritten as `matches!` with the same guard: no
behaviour change.

## Out of scope (not reachable in this checkout)

- TICKET-18 (Go lexicon/morphology demolition) and TICKET-20 (localized identity)
  require porting Go authorities across a repository boundary and their own
  parity gates. They stay OPEN.
- TICKET-08 and TICKET-10 stay BLOCKED: they need an external annotated dataset
  and SigLIP/pgvector infrastructure respectively.
- `cargo fmt --all -- --check` and the workspace-wide clippy gate cannot be
  proven green while another session's uncommitted files
  (`semantic-core/src/language_profiles.rs`, `localized_identity.rs`,
  `generated_language_profiles.rs`) are present and unformatted. Every file
  touched by this ticket is `rustfmt`-clean and clippy-clean in isolation.

## Verification summary

```
cargo test -p overlay-compiler           64 lib + 30 contract   ok
cargo test -p pipelinegen-muscles       124 lib + 4 + 4         ok
cargo test -p semantic-engine            58 lib + integration   ok
cargo clippy -p overlay-compiler -p pipelinegen-muscles -p semantic-engine -p visualner \
        --all-targets -- -D warnings                             ok
rustfmt --check <files touched by this ticket>                   clean
```
