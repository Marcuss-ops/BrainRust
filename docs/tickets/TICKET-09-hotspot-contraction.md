# TICKET 09 — targeted hotspot contraction

- Priority: P2
- Status: DONE (targeted contraction)
- Source: audit roadmap PR 9
- Files: `pipelinegen-muscles/src/render_audio_execution.rs`,
  `pipelinegen-muscles/src/render_audio_execution/sources.rs`,
  `semantic-engine/src/cache.rs`, `semantic-engine/src/lib.rs`

## Completed contractions

- Extracted audio source preparation into
  `render_audio_execution/sources.rs`: `SourceCaches` now owns bounded
  parallel probes, the authoritative source-range check and the one-per-path
  BGM loudness cache. `execute` remains the orchestrator instead of also
  holding the source-I/O state machine.
- Removed the duplicate cache fingerprint path. `ManifestCache::key` and
  `semantic_engine::fingerprint` are gone; `ManifestCache::request_key` is the
  only cache key and calls the same `request_fingerprint` recorded on the
  manifest. The old helper ignored editorial options and the safety cap, so
  this removal also closes a real collision risk.
- Kept the earlier bounded fan-out (`MAX_PARALLEL_IO`), group-kill timeout,
  poison-safe locks, and removal of `preflight`/`overlay_fingerprint`.
- Unified the audio-plan and protocol sample-rate gates on
  `audio_copy::CANONICAL_SAMPLE_RATE` instead of repeating `48000` in each
  validator.

## Deliberate non-splits

- `pipelinegen-muscles/src/protocol.rs` stays one wire module: its validation
  helpers are already grouped at the operation boundary and splitting it would
  move code without reducing coupling.
- `semantic-engine/src/consolidate.rs` already delegates to the `consolidate/`
  topic modules; `candidates.rs` remains a candidate-producer module. Further
  splits need their own corpus and benchmark, not an audit checkbox.

## Acceptance evidence

```sh
cargo test -p pipelinegen-muscles --lib   # 144/144
cargo test -p semantic-engine --lib       # 47/47
cargo test -p semantic-core --lib         # 139/139
cargo clippy -p semantic-core -p semantic-engine -p imagecatalog \
  -p pipelinegen-muscles --all-targets -- -D warnings
```

## Residual

`render_audio_execution::execute` is still the request-level orchestration,
including stem fan-out and publication. A further split should be justified by
an end-to-end audio benchmark, not by line count alone.
