# TICKET 07 — imagecatalog consumes an existing manifest

- Priority: P1
- Status: DONE (Rust wire contract)
- Source: audit roadmap PR 7
- Files: `semantic-core/src/manifest.rs`, `imagecatalog/src/types.rs`,
  `imagecatalog/src/plan.rs`, `imagecatalog/tests/manifest_reuse.rs`,
  `docs/operations/image-catalog.md`

## Problem

`plan()` called `semantic_engine::analyze_script` even when the caller already
owned the certified manifest for the same scenes, duplicating CPU work.

## Completed

- `SemanticManifest` and its nested wire types now derive `Deserialize`. The
  former `&'static str` contract fields (`schema_version`, scene `status`,
  temporal `precision`/`era`) are owned strings, so one type serves both the
  engine response and the imagecatalog request. No parallel DTO exists.
- `ImageCatalogRequest` has an optional `manifest` field. When absent,
  behavior is unchanged and the worker analyses the scenes. When present,
  `plan()` validates:
  - `schema_version == semantic_manifest.v2`;
  - language matches the request;
  - scene count and every `scene_id` match the request;
  - the manifest fingerprint equals `request_fingerprint` recomputed from the
    request and the manifest's analyzer versions.
- A valid manifest is passed directly to `plan_from_manifest`; the semantic
  engine is not called a second time.
- The wire tests prove reuse by changing a scene status after analysis: the
  fingerprint still matches (it covers inputs, not results), and the plan
  reports the supplied rejection. A re-analysis would have reported `valid`.
- Invalid schema and foreign fingerprint are rejected fail-closed.

## Acceptance evidence

```sh
cargo test -p imagecatalog --lib --tests
```

39/39 tests pass: 24 catalog/unit tests, 3 live-fixture tests, 7 manifest
reuse tests (in-process, wire reuse, bad schema, bad fingerprint, foreign
language, wrong scene count, wrong scene id) and 5 plan integration tests.

`cargo test -p semantic-core --lib` also passes 139/139 after the owned
contract-string change.

## Residual

PipelineGen must populate the optional `manifest` field to get the saving; old
scenes-only requests remain valid. The Go consumer fixture is outside this
checkout, so cross-repo adoption is not claimed here.
